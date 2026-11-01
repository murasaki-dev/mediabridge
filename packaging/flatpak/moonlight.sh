#!/usr/bin/env bash
set -euo pipefail
# The bundled upstream AppImage ships Qt's xcb plugin, not Qt Wayland/EGLFS.
# X11 must remain available alongside Wayland so this also works via XWayland.
if [[ -z "${DISPLAY:-}" ]]; then
  echo 'Moonlight needs an X11/XWayland display. Enable XWayland in the desktop and grant the MediaBridge Flatpak its X11 socket.' >&2
  exit 1
fi
export QT_QPA_PLATFORM=xcb
unset QT_PLUGIN_PATH QT_QPA_PLATFORM_PLUGIN_PATH
exec /app/lib/moonlight/AppRun "$@"
