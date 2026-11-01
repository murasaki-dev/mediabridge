"""Portal sequencing and desktop input tests; no real pointer events are sent."""
import importlib.util
from pathlib import Path
import sys
import types
import tempfile
import unittest
from unittest.mock import patch


class Variant:
    def __init__(self, signature, value):
        self.signature, self.value = signature, value

    def unpack(self):
        return self.value


glib = types.SimpleNamespace(Variant=Variant, timeout_add_seconds=lambda *args: 1, source_remove=lambda *_: None)
gio = types.SimpleNamespace(DBusCallFlags=types.SimpleNamespace(NONE=0), DBusSignalFlags=types.SimpleNamespace(NONE=0))
repository = types.ModuleType('gi.repository')
repository.Gio, repository.GLib = gio, glib
sys.modules['gi'] = types.ModuleType('gi')
sys.modules['gi.repository'] = repository
spec = importlib.util.spec_from_file_location('desktop_input', Path(__file__).resolve().parents[1] / 'packaging/flatpak/desktop_input.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Connection:
    def __init__(self):
        self.signals, self.calls = {}, []
        self.counter = 0

    def get_unique_name(self):
        return ':1.42'

    def signal_subscribe(self, bus, interface, signal, path, *_args):
        self.counter += 1
        self.signals[self.counter] = (signal, path, _args[-1])
        return self.counter

    def signal_unsubscribe(self, subscription):
        self.signals.pop(subscription, None)

    def call(self, bus, path, interface, method, parameters, *_):
        self.calls.append((method, parameters))

    def call_sync(self, *args):
        return Variant('(v)', (2,))

    def respond(self, result, code=0):
        callback = next(value[2] for value in self.signals.values() if value[0] == 'Response')
        callback(None, None, None, None, None, Variant('(ua{sv})', (code, result)))


class DesktopInputTests(unittest.TestCase):
    def bridge(self, x11=False):
        bridge = module.DesktopInput.__new__(module.DesktopInput)
        bridge.connection = Connection()
        bridge.session = None
        bridge.pending = False
        bridge.error = None
        bridge.generation = 0
        bridge.requests = {}
        bridge.closed_subscription = None
        bridge.x11 = x11
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        bridge.preference_path = Path(directory.name) / 'desktop-input.json'
        bridge.preference = {'enabled': False, 'restore_token': None}
        bridge.portal_version = 0
        return bridge

    def test_remember_restore_rotate_token_and_explicit_disconnect(self):
        bridge = self.bridge()
        bridge.enable()
        bridge.connection.respond({'session_handle': '/session/first'})
        self.assertEqual(bridge.connection.calls[-1][1].value[1]['persist_mode'].value, 2)
        bridge.connection.respond({})
        bridge.connection.respond({'devices': 2, 'restore_token': 'RESTORE_SECRET_ONE'})
        bridge.disable()  # Application shutdown preserves preference/token.
        saved = bridge.read_preference()
        self.assertTrue(saved['enabled'])
        self.assertEqual(saved['restore_token'], 'RESTORE_SECRET_ONE')
        restarted = self.bridge()
        restarted.preference_path = bridge.preference_path
        restarted.preference = restarted.read_preference()
        restarted.restore()
        self.assertTrue(restarted.pending)
        restarted.connection.respond({'session_handle': '/session/second'})
        options = restarted.connection.calls[-1][1].value[1]
        self.assertEqual(options['restore_token'].value, 'RESTORE_SECRET_ONE')
        self.assertIsNone(restarted.read_preference()['restore_token'])
        restarted.connection.respond({})
        restarted.connection.respond({'devices': 2, 'restore_token': 'RESTORE_SECRET_TWO'})
        self.assertNotIn('RESTORE_SECRET', str(restarted.status()))
        restarted.disable(forget=True)
        self.assertFalse(restarted.read_preference()['enabled'])
        self.assertIsNone(restarted.read_preference()['restore_token'])

    def test_permission_sequence_and_revocation(self):
        bridge = self.bridge()
        bridge.enable()
        self.assertTrue(bridge.status()['pending'])
        self.assertFalse(bridge.status()['enabled'])
        bridge.connection.respond({'session_handle': '/session/test'})
        self.assertEqual(bridge.connection.calls[-1][0], 'SelectDevices')
        self.assertEqual(bridge.connection.calls[-1][1].value[1]['types'].value, 2)
        bridge.connection.respond({})
        self.assertEqual(bridge.connection.calls[-1][0], 'Start')
        bridge.connection.respond({'devices': 2})
        self.assertTrue(bridge.status()['enabled'])
        calls = []
        bridge.call = lambda method, params: calls.append((method, params.value))
        bridge.mouse({'action': 'move_relative', 'dx': -12, 'dy': 7})
        bridge.mouse({'action': 'click_current', 'button': 'right'})
        self.assertEqual(calls[0], ('NotifyPointerMotion', ('/session/test', {}, -12.0, 7.0)))
        self.assertEqual([c[1][-2:] for c in calls[1:]], [(273, 1), (273, 0)])
        bridge.disable()
        self.assertFalse(bridge.status()['enabled'])
        self.assertFalse(bridge.connection.signals)
        with self.assertRaisesRegex(RuntimeError, 'Enable Desktop mouse'):
            bridge.mouse({'action': 'click_current'})

    def test_denial_and_no_pointer_grant(self):
        for code in (0, 1):
            bridge = self.bridge()
            bridge.enable()
            if not code:
                bridge.connection.respond({'session_handle': '/session/test'})
                bridge.connection.respond({})
            bridge.connection.respond({'devices': 1}, code)
            self.assertFalse(bridge.pending)
            self.assertIsNone(bridge.session)
            self.assertIsNotNone(bridge.error)

    def test_x11_normalized_coordinates_and_negative_relative(self):
        bridge = self.bridge(True)
        with patch.object(module.subprocess, 'run', return_value=types.SimpleNamespace(stdout='1920 1080')) as run:
            bridge.mouse({'action': 'click', 'x': 32767, 'y': 32767, 'button': 'middle'})
            self.assertEqual(run.call_args_list[1].args[0], ['xdotool', 'mousemove', '1919', '1079'])
            self.assertEqual(run.call_args_list[2].args[0], ['xdotool', 'click', '2'])
            bridge.mouse({'action': 'move_relative', 'dx': -10, 'dy': -2})
            self.assertEqual(run.call_args.args[0], ['xdotool', 'mousemove_relative', '--', '-10', '-2'])

    def test_invalid_requests(self):
        bridge = self.bridge()
        for request in ({'action': 'move', 'x': -1}, {'action': 'click_current', 'button': 'bad'},
                        {'action': 'move_relative', 'dx': 32768}, {'action': 'move_relative', 'dx': True}):
            with self.assertRaises(ValueError):
                bridge.mouse(request)
        bridge.session = '/session/test'
        with self.assertRaisesRegex(ValueError, 'absolute positioning'):
            bridge.mouse({'action': 'move', 'x': 12, 'y': 14})


if __name__ == '__main__':
    unittest.main()
