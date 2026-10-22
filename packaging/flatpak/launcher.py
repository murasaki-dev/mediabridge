#!/usr/bin/python3
"""Single-instance desktop/background entry point for the experimental Flatpak."""
import json
import locale
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import urllib.request

import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib
try:
    from gi.repository import GLibUnix
except ImportError:
    GLibUnix = None

APP_ID = "io.github.murasaki_dev.MediaBridge"
PORTAL = "org.freedesktop.portal.Desktop"
PORTAL_PATH = "/org/freedesktop/portal/desktop"


def configure_locale():
    # The user's host locale may not be included in the private runtime.
    try:
        locale.setlocale(locale.LC_ALL, "")
    except locale.Error:
        os.environ["LANG"] = "C.UTF-8"
        os.environ["LC_ALL"] = "C.UTF-8"
        locale.setlocale(locale.LC_ALL, "C.UTF-8")


class MediaBridge(Gio.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)
        self.child = None
        self.stopping = False
        self.failed = False
        self.dashboards = []
        self.background_pending = False
        self.background_started = False
        self.background_generation = 0

    def port(self):
        path = Path(os.environ["XDG_CONFIG_HOME"]) / "mediabridge/mediabridge.json"
        if path.exists():
            return int(json.loads(path.read_text()).get("bind", "0.0.0.0:8765").rsplit(":", 1)[1])
        return 8765

    def do_startup(self):
        Gio.Application.do_startup(self)
        action = Gio.SimpleAction.new("quit", None)
        action.connect("activate", lambda *_: self.stop())
        self.add_action(action)
        self.hold()
        try:
            # Refuse to share a port with a native installation or another server.
            with socket.socket() as probe:
                probe.settimeout(0.3)
                if probe.connect_ex(("127.0.0.1", self.port())) == 0:
                    raise RuntimeError("MediaBridge's port is already in use. Stop the native service before starting the Flatpak.")
            env = os.environ.copy()
            env["MEDIABRIDGE_LIBREWOLF_PROFILE"] = str(Path(env["XDG_DATA_HOME"]) / "librewolf-profile")
            env["MEDIABRIDGE_LOG_STDERR"] = "1"
            self.child = subprocess.Popen(["/app/bin/mediabridge-server"], env=env)
            GLib.timeout_add(500, self.check_child)
            GLib.idle_add(self.ensure_background)
        except Exception as error:
            print(f"MediaBridge could not start: {error}", file=sys.stderr, flush=True)
            self.failed = True
            GLib.idle_add(self.quit)

    def do_command_line(self, command):
        args = command.get_arguments()[1:]
        if any(arg not in ("--background", "--quit", "--autostart=on", "--autostart=off") for arg in args):
            command.printerr("Supported arguments: --background, --quit, --autostart=on, --autostart=off\n")
            return 2
        if "--autostart=on" in args or "--autostart=off" in args:
            if self.background_pending:
                command.printerr("A desktop permission request is already pending.\n")
                return 1
            self.request_background("--autostart=on" in args)
        elif "--quit" in args:
            self.stop()
        elif "--background" not in args and not self.failed:
            self.activate()
        return 1 if self.failed else 0

    def do_activate(self):
        if self.failed or self.stopping:
            return
        self.open_attempts = 0
        GLib.timeout_add(250, self.open_dashboard)

    def open_dashboard(self):
        if self.stopping or self.child is None or self.child.poll() is not None:
            return False
        self.open_attempts += 1
        with socket.socket() as probe:
            probe.settimeout(0.2)
            ready = probe.connect_ex(("127.0.0.1", self.port())) == 0
        if not ready:
            return self.open_attempts < 80
        self.dashboards.append(subprocess.Popen(
            ["/app/bin/mediabridge-librewolf", f"http://127.0.0.1:{self.port()}/"], start_new_session=True))
        return False

    def check_child(self):
        if self.child is None or self.child.poll() is None:
            return not self.stopping
        print(f"MediaBridge server exited: {self.child.returncode}", file=sys.stderr, flush=True)
        self.failed = self.child.returncode != 0
        self.quit()
        return False

    def autostart_path(self):
        return Path(os.environ["XDG_CONFIG_HOME"]) / "mediabridge/autostart.json"

    def read_autostart(self):
        try:
            return json.loads(self.autostart_path().read_text())
        except FileNotFoundError:
            return {"enabled": None, "requested": True, "pending": False, "error": None}
        except (OSError, ValueError):
            # Do not accidentally re-enable an opt-out if its state cannot be read.
            return {"enabled": None, "requested": False, "pending": False,
                    "error": "Could not read the saved sign-in startup choice"}

    def write_autostart(self, state):
        path = self.autostart_path()
        path.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as output:
            json.dump(state, output)
            temporary = output.name
        try:
            os.replace(temporary, path)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)

    def ensure_background(self):
        if not self.background_started:
            self.request_background()
        return False

    def request_background(self, autostart=None):
        if self.background_pending:
            return
        state = self.read_autostart()
        desired = state.get("requested", True) if autostart is None else autostart
        state.update(requested=desired, pending=True, error=None)
        self.write_autostart(state)
        self.background_pending = True
        self.background_started = True
        self.background_generation += 1
        generation = self.background_generation
        connection = self.get_dbus_connection()
        token = f"mediabridge_background_{self.background_generation}"
        sender = connection.get_unique_name().lstrip(":").replace(".", "_")
        path = f"{PORTAL_PATH}/request/{sender}/{token}"

        def finish(error=None, enabled=None):
            if not self.background_pending or self.background_generation != generation:
                return
            self.background_pending = False
            connection.signal_unsubscribe(subscription)
            state.update(pending=False, error=error)
            if enabled is not None:
                state["enabled"] = enabled
            self.write_autostart(state)

        def response(_connection, _sender, _path, _interface, _signal, parameters, *_data):
            if not self.background_pending or self.background_generation != generation:
                return
            code, results = parameters.unpack()
            if code != 0:
                finish("Desktop startup request was cancelled or denied")
                return
            enabled = bool(results.get("autostart", False))
            error = None if enabled == desired else "The desktop did not apply the requested sign-in startup setting"
            finish(error, enabled)
            if results.get("background", False):
                self.portal_call("SetStatus", GLib.Variant("(a{sv})", ({
                    "message": GLib.Variant("s", "Ready for LAN playback")},)))
            else:
                print("Background permission denied; stopping MediaBridge.", file=sys.stderr)
                self.stop()

        subscription = connection.signal_subscribe(PORTAL, "org.freedesktop.portal.Request", "Response",
                                                    path, None, Gio.DBusSignalFlags.NONE, response)
        self.portal_call("RequestBackground", GLib.Variant("(sa{sv})", ("", {
            "handle_token": GLib.Variant("s", token),
            "reason": GLib.Variant("s", "Receive LAN playback commands and optionally start at sign-in"),
            "autostart": GLib.Variant("b", desired),
            "commandline": GLib.Variant("as", ["mediabridge-flatpak", "--background"]),
        })), on_error=lambda error: finish(error))

        def timeout():
            if self.background_pending and self.background_generation == int(token.rsplit("_", 1)[1]):
                finish("Desktop startup approval timed out; try again")
                connection.call(PORTAL, path, "org.freedesktop.portal.Request", "Close", None, None,
                                Gio.DBusCallFlags.NONE, 5000, None, None)
            return False
        GLib.timeout_add_seconds(120, timeout)

    def portal_call(self, method, parameters, on_error=None):
        def completed(connection, result, *_data):
            try:
                connection.call_finish(result)
            except GLib.Error as error:
                print(f"Background portal {method} unavailable: {error.message}", file=sys.stderr, flush=True)
                if on_error is not None:
                    on_error(f"Desktop portal unavailable: {error.message}")
        self.get_dbus_connection().call(PORTAL, PORTAL_PATH, "org.freedesktop.portal.Background",
                                       method, parameters, None, Gio.DBusCallFlags.NONE, 10000, None, completed)

    def stop(self):
        if self.stopping:
            return
        self.stopping = True
        # Ask the server to clean up its players before terminating it. Steam cleanup
        # can be asynchronous, so allow a bounded grace period below.
        if self.child is not None and self.child.poll() is None:
            try:
                req = urllib.request.Request(f"http://127.0.0.1:{self.port()}/close", method="POST")
                with urllib.request.urlopen(req, timeout=3):
                    pass
            except Exception as error:
                print(f"Playback cleanup failed: {error}", file=sys.stderr, flush=True)
            self.close_attempts = 0
            GLib.timeout_add(250, self.finish_stop)
        else:
            self.quit()

    def finish_stop(self):
        self.close_attempts += 1
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{self.port()}/status", timeout=1) as response:
                steam = json.load(response).get("steam", {})
            if steam.get("phase") == "closing" and self.close_attempts < 32:
                return True
        except Exception:
            pass
        if self.child is not None and self.child.poll() is None:
            self.child.terminate()
        for browser in self.dashboards:
            if browser.poll() is None:
                try:
                    os.killpg(browser.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        self.quit()
        return False


if __name__ == "__main__":
    configure_locale()
    application = MediaBridge()
    for signum in (signal.SIGTERM, signal.SIGINT):
        signal_add = GLibUnix.signal_add if GLibUnix is not None else GLib.unix_signal_add
        signal_add(GLib.PRIORITY_DEFAULT, signum, lambda: (application.stop(), False)[1])
    status = application.run(sys.argv)
    sys.exit(1 if application.failed else status)
