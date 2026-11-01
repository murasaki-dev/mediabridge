"""Desktop pointer bridge. Runs on the launcher's GLib main loop."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import uuid

from gi.repository import Gio, GLib

BUS = 'org.freedesktop.portal.Desktop'
ROOT = '/org/freedesktop/portal/desktop'
REMOTE = 'org.freedesktop.portal.RemoteDesktop'


class DesktopInput:
    def __init__(self, connection):
        self.connection = connection
        self.session = None
        self.pending = False
        self.error = None
        self.generation = 0
        self.requests = {}
        self.closed_subscription = None
        self.preference_path = Path(os.environ['XDG_CONFIG_HOME']) / 'mediabridge/desktop-input.json'
        self.preference = self.read_preference()
        self.portal_version = 0
        # DISPLAY also exists under XWayland; never mistake it for a host X11 session.
        session_type = os.environ.get('XDG_SESSION_TYPE', '')
        self.x11 = session_type == 'x11' or (not session_type and not os.environ.get('WAYLAND_DISPLAY') and bool(os.environ.get('DISPLAY')))
        self.directory = Path(__import__('tempfile').mkdtemp(prefix='mediabridge-input-', dir=os.environ['XDG_RUNTIME_DIR']))
        self.path = self.directory / 'control.sock'
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.bind(str(self.path))
        os.chmod(self.path, 0o600)
        self.socket.listen(8)
        self.socket.setblocking(False)
        self.watch = GLib.io_add_watch(self.socket.fileno(), GLib.IO_IN, self.receive)
        GLib.idle_add(self.restore)

    def read_preference(self):
        try:
            saved = json.loads(self.preference_path.read_text())
            token = saved.get('restore_token')
            return {'enabled': saved.get('enabled') is True,
                    'restore_token': token if isinstance(token, str) and len(token) <= 8192 else None}
        except (OSError, ValueError, AttributeError):
            return {'enabled': False, 'restore_token': None}

    def save_preference(self):
        self.preference_path.parent.mkdir(parents=True, exist_ok=True)
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(mode='w', dir=self.preference_path.parent, delete=False) as output:
                temporary = output.name
                os.chmod(temporary, 0o600)
                json.dump(self.preference, output)
            os.replace(temporary, self.preference_path)
        finally:
            if temporary and os.path.exists(temporary):
                os.unlink(temporary)

    def restore(self):
        if self.preference['enabled'] and not self.x11:
            try:
                self.enable()
            except Exception as error:
                self.error = f'Could not restore desktop mouse: {error}'
        return False

    def status(self):
        return {'supported': True, 'backend': 'x11' if self.x11 else 'portal',
                'enabled': self.x11 or self.session is not None and not self.pending,
                'pending': self.pending, 'error': self.error,
                'absolute_supported': self.x11, 'scope': 'desktop',
                'remembered': self.preference['enabled'], 'persistent_permission': bool(self.preference.get('restore_token'))}

    def receive(self, *_):
        client, _ = self.socket.accept()
        with client:
            client.settimeout(1)
            try:
                data = b''
                while b'\n' not in data and len(data) <= 4096:
                    chunk = client.recv(4097 - len(data))
                    if not chunk:
                        break
                    data += chunk
                if len(data) > 4096 or b'\n' not in data:
                    raise ValueError('Invalid desktop input request')
                request = json.loads(data.split(b'\n', 1)[0])
                operation = request.get('operation')
                if operation == 'enable':
                    self.enable()
                elif operation == 'disable':
                    self.disable(forget=True)
                elif operation == 'mouse':
                    self.mouse(request['request'])
                elif operation != 'status':
                    raise ValueError('Unknown desktop input operation')
                result = self.status()
            except Exception as error:
                result = {'error': str(error)}
            try:
                client.sendall(json.dumps(result).encode() + b'\n')
            except OSError:
                pass
        return True

    def call(self, method, parameters):
        return self.connection.call_sync(BUS, ROOT, REMOTE, method, parameters, None,
                                         Gio.DBusCallFlags.NONE, 3000, None)

    def request(self, method, signature, arguments, options, callback):
        generation = self.generation
        token = 'mediabridge_' + uuid.uuid4().hex
        sender = self.connection.get_unique_name().lstrip(':').replace('.', '_')
        path = f'{ROOT}/request/{sender}/{token}'
        options = dict(options, handle_token=GLib.Variant('s', token))

        def cleanup():
            saved = self.requests.pop(path, None)
            if saved:
                self.connection.signal_unsubscribe(saved[0])
                GLib.source_remove(saved[1])

        def failed(error):
            cleanup()
            if generation == self.generation:
                self.disable()
                self.error = str(error)

        def response(_connection, _sender, _path, _interface, _signal, parameters, *_):
            cleanup()
            if generation != self.generation:
                return
            code, result = parameters.unpack()
            if code:
                self.preference.update(enabled=False, restore_token=None)
                self.save_preference()
                failed('Desktop mouse permission was cancelled or denied')
                return
            try:
                callback(result)
            except Exception as error:
                failed(error)

        def timeout():
            failed('Desktop mouse permission timed out; try again')
            return False

        subscription = self.connection.signal_subscribe(BUS, 'org.freedesktop.portal.Request', 'Response',
            path, None, Gio.DBusSignalFlags.NONE, response)
        self.requests[path] = (subscription, GLib.timeout_add_seconds(120, timeout))

        def completed(connection, result, *_):
            try:
                connection.call_finish(result)
            except Exception as error:
                failed(error)
        self.connection.call(BUS, ROOT, REMOTE, method,
            GLib.Variant(signature, (*arguments, options)), None, Gio.DBusCallFlags.NONE, 10000, None, completed)

    def enable(self):
        if self.x11 or self.pending or self.session:
            return
        self.preference['enabled'] = True
        self.save_preference()
        self.error = None
        self.pending = True
        self.generation += 1

        def started(result):
            if not result.get('devices', 0) & 2:
                raise RuntimeError('The desktop did not grant pointer control')
            self.pending = False
            token = result.get('restore_token')
            self.preference['restore_token'] = token if isinstance(token, str) else None
            self.save_preference()

        def selected(_):
            self.request('Start', '(osa{sv})', (self.session, ''), {}, started)

        def created(result):
            self.session = result['session_handle']
            def closed(*_):
                self.disable()
                self.error = 'Desktop mouse session ended; enable it again'
            self.closed_subscription = self.connection.signal_subscribe(BUS, 'org.freedesktop.portal.Session',
                'Closed', self.session, None, Gio.DBusSignalFlags.NONE, closed)
            options = {'types': GLib.Variant('u', 2)}
            try:
                result = self.connection.call_sync(BUS, ROOT, 'org.freedesktop.DBus.Properties', 'Get',
                    GLib.Variant('(ss)', (REMOTE, 'version')), None, Gio.DBusCallFlags.NONE, 3000, None)
                self.portal_version = int(result.unpack()[0])
            except Exception:
                self.portal_version = 1
            if self.portal_version >= 2:
                options['persist_mode'] = GLib.Variant('u', 2)
                if self.preference.get('restore_token'):
                    options['restore_token'] = GLib.Variant('s', self.preference['restore_token'])
                    # Restore tokens are single-use; Start returns their replacement.
                    self.preference['restore_token'] = None
                    self.save_preference()
            self.request('SelectDevices', '(oa{sv})', (self.session,), options, selected)

        self.request('CreateSession', '(a{sv})', (),
            {'session_handle_token': GLib.Variant('s', 'mediabridge_' + uuid.uuid4().hex)}, created)

    def disable(self, forget=False):
        if forget:
            self.preference.update(enabled=False, restore_token=None)
            self.save_preference()
        self.generation += 1
        self.pending = False
        for path, (subscription, timeout) in list(self.requests.items()):
            self.connection.signal_unsubscribe(subscription)
            GLib.source_remove(timeout)
            self.connection.call(BUS, path, 'org.freedesktop.portal.Request', 'Close', None, None,
                                 Gio.DBusCallFlags.NONE, 3000, None, None)
        self.requests.clear()
        if self.closed_subscription:
            self.connection.signal_unsubscribe(self.closed_subscription)
            self.closed_subscription = None
        session, self.session = self.session, None
        if session:
            self.connection.call(BUS, session, 'org.freedesktop.portal.Session', 'Close', None, None,
                                 Gio.DBusCallFlags.NONE, 3000, None, None)

    @staticmethod
    def validate(request):
        action = request.get('action')
        if action not in ('move', 'move_relative', 'click', 'click_current'):
            raise ValueError('Invalid mouse action')
        if request.get('button', 'left') not in ('left', 'right', 'middle'):
            raise ValueError('Invalid mouse button')
        for key in ('x', 'y') if action in ('move', 'click') else ('dx', 'dy') if action == 'move_relative' else ():
            value = request.get(key, 0)
            if type(value) is not int or not (0 if key in ('x', 'y') else -32767) <= value <= 32767:
                raise ValueError('Mouse coordinates are out of range')

    def mouse(self, request):
        self.validate(request)
        action = request['action']
        if self.x11:
            def run(*args):
                return subprocess.run(['xdotool', *map(str, args)], check=True, capture_output=True, text=True, timeout=3).stdout
            if action in ('move', 'click'):
                width, height = map(int, run('getdisplaygeometry').split())
                run('mousemove', round(request.get('x', 0) * (width - 1) / 32767),
                    round(request.get('y', 0) * (height - 1) / 32767))
            elif action == 'move_relative':
                run('mousemove_relative', '--', request.get('dx', 0), request.get('dy', 0))
            if action in ('click', 'click_current'):
                run('click', {'left': 1, 'middle': 2, 'right': 3}[request.get('button', 'left')])
            return
        if self.pending or not self.session:
            raise RuntimeError('Enable Desktop mouse in dashboard settings and approve it on the server computer')
        if action in ('move', 'click'):
            raise ValueError('This Wayland backend supports relative movement and click_current; absolute positioning requires screen sharing')
        try:
            if action == 'move_relative':
                self.call('NotifyPointerMotion', GLib.Variant('(oa{sv}dd)',
                    (self.session, {}, float(request.get('dx', 0)), float(request.get('dy', 0)))))
            else:
                button = {'left': 272, 'right': 273, 'middle': 274}[request.get('button', 'left')]
                for pressed in (1, 0):
                    self.call('NotifyPointerButton', GLib.Variant('(oa{sv}iu)', (self.session, {}, button, pressed)))
        except Exception:
            self.disable()
            self.error = 'Desktop input failed; enable desktop mouse again'
            raise

    def close(self):
        self.disable()
        GLib.source_remove(self.watch)
        self.socket.close()
        self.path.unlink(missing_ok=True)
        self.directory.rmdir()
