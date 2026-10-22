"""Linux tests for launcher requests and cleanup, using real GLib variants."""
import ast
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
import sys
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
for source in (ROOT / "packaging/flatpak").glob("*.py"):
    ast.parse(source.read_text())
spec = importlib.util.spec_from_file_location("launcher", ROOT / "packaging/flatpak/launcher.py")
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.config = tempfile.TemporaryDirectory()
        self.addCleanup(self.config.cleanup)
        self.environment = patch.dict(os.environ, XDG_CONFIG_HOME=self.config.name)
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def test_missing_host_locale_falls_back_for_child_processes(self):
        with patch.dict(os.environ, {"LANG": "xx_XX.UTF-8"}), \
                patch.object(launcher.locale, "setlocale", side_effect=[launcher.locale.Error(), "C.UTF-8"]) as setlocale:
            launcher.configure_locale()
            self.assertEqual(os.environ["LC_ALL"], "C.UTF-8")
            self.assertEqual(os.environ["LANG"], "C.UTF-8")
            self.assertEqual(setlocale.call_args.args, (launcher.locale.LC_ALL, "C.UTF-8"))

    def test_port_uses_private_config(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, XDG_CONFIG_HOME=directory):
            app = launcher.MediaBridge()
            self.assertEqual(app.port(), 8765)
            path = Path(directory) / "mediabridge/mediabridge.json"
            path.parent.mkdir()
            path.write_text(json.dumps({"bind": "0.0.0.0:9876"}))
            self.assertEqual(app.port(), 9876)

    def test_portal_subscribes_before_request_and_handles_denial(self):
        app = launcher.MediaBridge()
        connection = Mock()
        connection.get_unique_name.return_value = ":1.42"
        connection.signal_subscribe.return_value = 5
        app.get_dbus_connection = Mock(return_value=connection)
        app.portal_call = Mock()
        app.stop = Mock()
        app.request_background()
        arguments = connection.signal_subscribe.call_args.args
        self.assertEqual(arguments[3], "/org/freedesktop/portal/desktop/request/1_42/mediabridge_background_1")
        method, parameters = app.portal_call.call_args.args
        self.assertEqual(method, "RequestBackground")
        self.assertTrue(parameters.unpack()[1]["autostart"])
        self.assertEqual(parameters.unpack()[1]["commandline"], ["mediabridge-flatpak", "--background"])
        self.assertTrue(app.read_autostart()["pending"])
        callback = arguments[-1]
        callback(None, None, None, None, None, launcher.GLib.Variant("(ua{sv})", (0, {
            "background": launcher.GLib.Variant("b", True), "autostart": launcher.GLib.Variant("b", True)})))
        self.assertEqual(app.portal_call.call_args.args[0], "SetStatus")
        self.assertTrue(app.read_autostart()["enabled"])
        self.assertFalse(app.read_autostart()["pending"])
        app.request_background(False)
        callback = connection.signal_subscribe.call_args.args[-1]
        callback(None, None, None, None, None, launcher.GLib.Variant("(ua{sv})", (0, {
            "background": launcher.GLib.Variant("b", False)})))
        app.stop.assert_called_once()

    def test_opt_out_survives_a_new_launcher_session(self):
        app = launcher.MediaBridge()
        app.write_autostart({"enabled": False, "requested": False, "pending": False, "error": None})
        reopened = launcher.MediaBridge()
        connection = Mock()
        connection.get_unique_name.return_value = ":1.43"
        reopened.get_dbus_connection = Mock(return_value=connection)
        reopened.portal_call = Mock()
        reopened.request_background()
        self.assertFalse(reopened.portal_call.call_args.args[1].unpack()[1]["autostart"])
        self.assertFalse(reopened.read_autostart()["requested"])

    def test_portal_error_clears_pending_without_claiming_enabled(self):
        app = launcher.MediaBridge()
        connection = Mock()
        connection.get_unique_name.return_value = ":1.44"
        app.get_dbus_connection = Mock(return_value=connection)
        app.portal_call = Mock()
        app.request_background()
        app.portal_call.call_args.kwargs["on_error"]("Portal is unavailable")
        state = app.read_autostart()
        self.assertFalse(state["pending"])
        self.assertIsNone(state["enabled"])
        self.assertEqual(state["error"], "Portal is unavailable")

    def test_desktop_refusal_is_reported_as_disabled(self):
        app = launcher.MediaBridge()
        connection = Mock()
        connection.get_unique_name.return_value = ":1.45"
        app.get_dbus_connection = Mock(return_value=connection)
        app.portal_call = Mock()
        app.request_background(True)
        connection.signal_subscribe.call_args.args[-1](None, None, None, None, None,
            launcher.GLib.Variant("(ua{sv})", (0, {"background": launcher.GLib.Variant("b", True),
                                                      "autostart": launcher.GLib.Variant("b", False)})))
        state = app.read_autostart()
        self.assertFalse(state["enabled"])
        self.assertFalse(state["pending"])
        self.assertIn("did not apply", state["error"])

    def test_overlapping_requests_do_not_replace_an_open_prompt(self):
        app = launcher.MediaBridge()
        connection = Mock()
        connection.get_unique_name.return_value = ":1.46"
        app.get_dbus_connection = Mock(return_value=connection)
        app.portal_call = Mock()
        app.request_background(True)
        app.request_background(False)
        app.portal_call.assert_called_once()
        self.assertTrue(app.read_autostart()["requested"])

    def test_quit_closes_media_before_terminating_server(self):
        app = launcher.MediaBridge()
        app.port = Mock(return_value=8765)
        app.child = Mock()
        app.child.poll.return_value = None
        app.quit = Mock()
        app.dashboards = [Mock(pid=12345)]
        app.dashboards[0].poll.return_value = None
        with patch.object(launcher.urllib.request, "urlopen") as request, \
                patch.object(launcher.GLib, "timeout_add") as timer, patch.object(launcher.os, "killpg") as kill:
            app.stop()
            self.assertEqual(request.call_args.args[0].method, "POST")
            self.assertEqual(request.call_args.args[0].full_url, "http://127.0.0.1:8765/close")
            app.child.terminate.assert_not_called()
            timer.assert_called_once()
            request.return_value.__enter__.return_value.read.return_value = b'{"steam":{"phase":"idle"}}'
            app.finish_stop()
            app.child.terminate.assert_called_once()
            kill.assert_called_once_with(12345, launcher.signal.SIGTERM)
            app.quit.assert_called_once()


if __name__ == "__main__":
    unittest.main()
