"""Embedded host helper: only allowlisted Steam/process/window information leaves it."""
import ctypes as C
import ctypes.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

BOOT = None
TICKS = None


def process(pid):
    global BOOT, TICKS
    try:
        base = Path('/proc') / str(pid)
        if base.stat().st_uid != os.getuid():
            return None
        stat = (base / 'stat').read_text().rsplit(')', 1)[1].split()
        if stat[0] == 'Z':
            return None
        exe = os.readlink(base / 'exe')
        app_id = None
        try:
            fields = (base / 'environ').read_bytes().split(b'\0')
            env = dict(f.split(b'=', 1) for f in fields if f.startswith((b'SteamAppId=', b'SteamGameId=')) and b'=' in f)
            raw = env.get(b'SteamGameId') or env.get(b'SteamAppId')
            if raw and raw.isdigit():
                app_id = raw.decode('ascii')
        except OSError:
            pass
        started = int(stat[19])
        if BOOT is None:
            BOOT = next(int(line.split()[1]) for line in Path('/proc/stat').read_text().splitlines() if line.startswith('btime '))
            TICKS = os.sysconf('SC_CLK_TCK')
        started_local = time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(BOOT + started / TICKS))
        return dict(pid=int(pid), started=started, started_local=started_local, exe=exe, app_id=app_id)
    except (OSError, ValueError, IndexError):
        return None


class Desktop:
    def __init__(self):
        self.x = C.CDLL(ctypes.util.find_library('X11') or 'libX11.so.6')
        self.x.XOpenDisplay.restype = C.c_void_p
        self.x.XOpenDisplay.argtypes = [C.c_char_p]
        self.d = self.x.XOpenDisplay(None)
        if not self.d:
            raise RuntimeError('X11/XWayland window access unavailable')
        # Windows can disappear between enumeration and property reads.
        self.error_callback = C.CFUNCTYPE(C.c_int, C.c_void_p, C.c_void_p)(lambda display, event: 0)
        self.x.XSetErrorHandler.argtypes = [C.c_void_p]
        self.x.XSetErrorHandler(self.error_callback)
        self.x.XDefaultRootWindow.argtypes = [C.c_void_p]
        self.x.XDefaultRootWindow.restype = C.c_ulong
        self.root = self.x.XDefaultRootWindow(self.d)
        self.x.XInternAtom.argtypes = [C.c_void_p, C.c_char_p, C.c_int]
        self.x.XInternAtom.restype = C.c_ulong
        self.x.XGetWindowProperty.argtypes = [C.c_void_p, C.c_ulong, C.c_ulong, C.c_long, C.c_long, C.c_int, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_int), C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.c_void_p)]
        self.x.XFree.argtypes = [C.c_void_p]
        self.x.XIconifyWindow.argtypes = [C.c_void_p, C.c_ulong, C.c_int]
        self.x.XFlush.argtypes = [C.c_void_p]
        self.x.XQueryTree.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.POINTER(C.c_ulong)), C.POINTER(C.c_uint)]

    def prop(self, window, name):
        atom = self.x.XInternAtom(self.d, name.encode(), 1)
        kind, fmt, count, remaining, data = C.c_ulong(), C.c_int(), C.c_ulong(), C.c_ulong(), C.c_void_p()
        if not atom or self.x.XGetWindowProperty(self.d, window, atom, 0, 65536, 0, 0, C.byref(kind), C.byref(fmt), C.byref(count), C.byref(remaining), C.byref(data)) != 0 or not data.value:
            return None
        try:
            if fmt.value == 32:
                return list(C.cast(data, C.POINTER(C.c_ulong))[:count.value])
            if fmt.value == 8:
                return C.string_at(data, count.value).decode('utf-8', 'replace')
        finally:
            self.x.XFree(data)

    def windows(self, pids):
        clients = self.prop(self.root, '_NET_CLIENT_LIST')
        if clients is None:
            # Gamescope and minimal window managers need a direct tree walk.
            clients, pending = [], [(self.root, 0)]
            while pending and len(clients) < 4096:
                node, depth = pending.pop()
                root, parent, children, count = C.c_ulong(), C.c_ulong(), C.POINTER(C.c_ulong)(), C.c_uint()
                if self.x.XQueryTree(self.d, node, C.byref(root), C.byref(parent), C.byref(children), C.byref(count)):
                    try:
                        found = list(children[:count.value]) if children else []
                        clients.extend(found)
                        if depth < 2:
                            pending.extend((child, depth + 1) for child in found)
                    finally:
                        if children:
                            self.x.XFree(children)
        result = []
        for window in clients:
            pid = self.prop(window, '_NET_WM_PID')
            if pid and pid[0] in pids:
                title = self.prop(window, '_NET_WM_NAME') or self.prop(window, 'WM_NAME') or ''
                result.append((window, title.lower()))
        return result


def all_processes():
    return [p for entry in Path('/proc').iterdir() if entry.name.isdigit() for p in [process(entry.name)] if p]


def steam_processes(processes):
    return [p for p in processes if Path(p['exe']).name in ('steam', 'steamwebhelper')]


def probe():
    processes = all_processes()
    steam = steam_processes(processes)
    running = any(Path(p['exe']).name == 'steam' for p in steam)
    root = None
    for p in steam:
        if Path(p['exe']).name == 'steam':
            # The Linux client binary normally lives in <Steam>/ubuntu12_32/.
            root = next((str(parent) for parent in Path(p['exe']).parents if (parent / 'steamapps').is_dir() and (parent / 'logs').is_dir()), None)
            if root:
                break
    picture = False if not running else None
    if running:
        try:
            windows = Desktop().windows({p['pid'] for p in steam})
            picture = any('big picture' in title or title == 'steam gamepad ui' for _, title in windows)
        except (OSError, RuntimeError):
            pass
    return dict(root=root, running=running, big_picture_open=picture, processes=processes)


def main():
    action = sys.argv[1]
    if action == 'probe':
        print(json.dumps(probe()))
    elif action in ('open', 'close'):
        # Host xdg-open routes native and Flatpak Steam installations appropriately.
        subprocess.run(['xdg-open', 'steam://' + ('open' if action == 'open' else 'close') + '/bigpicture'], check=True, timeout=5, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    elif action == 'hide':
        desktop = Desktop()
        for window, title in desktop.windows({p['pid'] for p in steam_processes(all_processes())}):
            if 'big picture' not in title:
                desktop.x.XIconifyWindow(desktop.d, window, 0)
        desktop.x.XFlush(desktop.d)
    elif action in ('stop', 'kill'):
        pid, started, exe = int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
        p = process(pid)
        if p and p['started'] == started and p['exe'] == exe:
            if Path(exe).name in ('steam', 'steamwebhelper'):
                raise RuntimeError('refusing to terminate Steam')
            # pidfd prevents PID reuse between validation and sending the signal.
            fd = os.pidfd_open(pid)
            try:
                p = process(pid)
                if p and p['started'] == started and p['exe'] == exe:
                    signal.pidfd_send_signal(fd, signal.SIGKILL if action == 'kill' else signal.SIGTERM)
            finally:
                os.close(fd)
    else:
        raise RuntimeError('unsupported Steam helper action')


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(str(error))
        sys.exit(1)
