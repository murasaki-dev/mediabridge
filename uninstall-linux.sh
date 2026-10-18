#!/usr/bin/env bash
set -euo pipefail

APP_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/mediabridge"
BIN_DIR="${HOME}/.local/bin"
CONFIG_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/mediabridge"
STATE_DIR="${XDG_STATE_HOME:-${HOME}/.local/state}/mediabridge"
DISTROBOX_MARKER="$STATE_DIR/distrobox-runtime"
DISTROBOX_NAME="mediabridge-runtime"
DESKTOP_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/applications"
SYSTEMD_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"
AUTOSTART_FILE="${XDG_CONFIG_HOME:-${HOME}/.config}/autostart/mediabridge.desktop"
PURGE_DATA=0

usage() {
  cat <<'EOF'
Usage: ./uninstall-linux.sh [--purge-data]

Removes MediaBridge, its user service, launchers, and desktop entries. By
default, settings, logs, and the persistent LibreWolf profile/cookies remain
so they survive a reinstall. --purge-data also deletes those personal files.
System dependencies installed by the package manager are left in place.
EOF
}

for arg in "$@"; do
  case "$arg" in
    --help|-h) usage; exit 0 ;;
    --purge-data) PURGE_DATA=1 ;;
    *) echo "Unknown argument: $arg" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ "${EUID}" -eq 0 ]]; then
  echo "Run the uninstaller as the desktop user that installed MediaBridge." >&2
  exit 1
fi

if command -v systemctl >/dev/null 2>&1; then
  systemctl --user disable --now mediabridge.service 2>/dev/null || true
  rm -f "$SYSTEMD_DIR/mediabridge.service"
  systemctl --user daemon-reload 2>/dev/null || true
  systemctl --user reset-failed mediabridge.service 2>/dev/null || true
fi
if command -v pkill >/dev/null 2>&1; then
  pkill --uid "$(id -u)" --exact mediabridge 2>/dev/null || true
fi

# SteamOS/Bazzite keep MediaBridge's Linux packages in a dedicated user
# Distrobox. Remove only the container created by this installer; never touch
# similarly named containers unless our ownership marker is present.
if [[ -f "$DISTROBOX_MARKER" ]]; then
  if command -v distrobox >/dev/null 2>&1; then
    distrobox rm --force "$DISTROBOX_NAME" || {
      echo "Could not remove the MediaBridge runtime container '$DISTROBOX_NAME'." >&2
      echo "Remove it later with: distrobox rm --force $DISTROBOX_NAME" >&2
    }
  else
    echo "Distrobox is unavailable; the MediaBridge runtime container could not be removed." >&2
  fi
fi

rm -f "$BIN_DIR/mediabridge-dashboard" "$BIN_DIR/mediabridge-logs" \
  "$BIN_DIR/mediabridge-autostart" "$BIN_DIR/mediabridge-librewolf" \
  "$BIN_DIR/mediabridge-run" "$BIN_DIR/mediabridge-uninstall" \
  "$DESKTOP_DIR/mediabridge.desktop" "$AUTOSTART_FILE"

if [[ "$PURGE_DATA" -eq 1 ]]; then
  rm -rf -- "$APP_DIR" "$CONFIG_DIR" "$STATE_DIR"
  echo "Removed MediaBridge and its settings, logs, browser profile, and cookies."
else
  # Keep personal data and cookies, but remove downloaded executables/browser.
  if [[ -d "$APP_DIR" ]]; then
    find "$APP_DIR" -mindepth 1 -maxdepth 1 \
      ! -name 'librewolf-profile' ! -name 'librewolf-home' -exec rm -rf -- {} +
    rmdir "$APP_DIR" 2>/dev/null || true
  fi
  echo "Removed MediaBridge. Settings, logs, and LibreWolf cookies were kept."
  echo "To remove those too, rerun with --purge-data."
fi

echo "Package-manager dependencies were left installed. Remove them manually if no other apps use them."
