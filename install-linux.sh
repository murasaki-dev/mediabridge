#!/usr/bin/env bash
set -euo pipefail

# User-facing installer for common glibc Linux distributions. It installs
# runtime dependencies with the host package manager and installs MediaBridge
# and LibreWolf entirely inside the current user's home directory.
REPOSITORY="murasaki-dev/mediabridge"
APP_ASSET="mediabridge-linux-x86_64.tar.gz"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/mediabridge"
BIN_DIR="${HOME}/.local/bin"
CONFIG_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/mediabridge"
STATE_DIR="${XDG_STATE_HOME:-${HOME}/.local/state}/mediabridge"
DESKTOP_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/applications"
SYSTEMD_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"
PACKAGE_ONLY=0
NO_AUTOSTART=0
LOCAL_ARCHIVE=""

usage() {
  cat <<'EOF'
Usage: ./install-linux.sh [--local | archive.tar.gz] [--deps-only] [--no-autostart]

With no archive argument, downloads the latest GitHub Release asset. To build
and install from a Linux checkout before the first release is published:
  chmod +x build-linux.sh install-linux.sh
  ./build-linux.sh
  ./install-linux.sh --local

--deps-only installs runtime packages without downloading MediaBridge.
--no-autostart installs the app but does not enable its login service.
EOF
}

while (($#)); do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --local) LOCAL_ARCHIVE="${SCRIPT_DIR}/dist/${APP_ASSET}" ;;
    --deps-only) PACKAGE_ONLY=1 ;;
    --no-autostart) NO_AUTOSTART=1 ;;
    *.tar.gz) LOCAL_ARCHIVE="$1" ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

if [[ "${EUID}" -eq 0 ]]; then
  echo "Run this installer as your desktop user; it uses sudo only for system dependencies." >&2
  exit 1
fi
if [[ "$(uname -m)" != x86_64 ]]; then
  echo "This installer currently supports x86_64 Linux. Build assets for other architectures are not available yet." >&2
  exit 1
fi
command -v sudo >/dev/null 2>&1 || { echo "sudo is required to install system dependencies." >&2; exit 1; }

if [[ ! -r /etc/os-release ]]; then
  echo "Cannot identify this Linux distribution (missing /etc/os-release)." >&2
  exit 1
fi
# shellcheck disable=SC1091
. /etc/os-release
DISTRO_ID="${ID:-unknown}"
DISTRO_LIKE="${ID_LIKE:-}"
PACKAGE_MANAGER=""
for candidate in apt-get dnf pacman zypper xbps-install; do
  if command -v "$candidate" >/dev/null 2>&1; then PACKAGE_MANAGER="$candidate"; break; fi
done

case "$PACKAGE_MANAGER" in
  apt-get)
    sudo apt-get update
    sudo apt-get install -y ca-certificates curl mpv yt-dlp playerctl xdotool ydotool xdg-utils python3 python3-secretstorage libnotify-bin libgtk-3-0 libayatana-appindicator3-1
    ;;
  dnf)
    sudo dnf install -y ca-certificates curl mpv yt-dlp playerctl xdotool ydotool xdg-utils python3 python3-secretstorage libnotify gtk3 libappindicator-gtk3
    ;;
  pacman)
    sudo pacman -S --needed --noconfirm ca-certificates curl mpv yt-dlp playerctl xdotool ydotool xdg-utils python python-secretstorage libnotify gtk3 libappindicator-gtk3
    ;;
  zypper)
    sudo zypper --non-interactive install ca-certificates curl mpv yt-dlp playerctl xdotool ydotool xdg-utils python3 python3-secretstorage libnotify-tools gtk3 libayatana-appindicator3-1
    ;;
  xbps-install)
    sudo xbps-install -Sy ca-certificates curl mpv yt-dlp playerctl xdotool ydotool xdg-utils python3 python3-secretstorage libnotify gtk+3 libappindicator
    ;;
  *)
    echo "Unsupported package manager for '$DISTRO_ID' (ID_LIKE: ${DISTRO_LIKE:-none}). Supported managers: apt, dnf, pacman, zypper, and xbps." >&2
    echo "Install curl, mpv, yt-dlp, playerctl, xdotool, ydotool, xdg-utils, Python 3 with SecretStorage, and libnotify manually, then adapt this installer." >&2
    exit 1
    ;;
esac
command -v curl >/dev/null 2>&1 || { echo "Could not install curl through $PACKAGE_MANAGER." >&2; exit 1; }

# Deno supplies yt-dlp's JavaScript challenge runtime.
if ! command -v deno >/dev/null 2>&1 && [[ ! -x "$HOME/.deno/bin/deno" ]]; then
  export DENO_INSTALL="$HOME/.deno"
  curl --proto '=https' --tlsv1.2 -sSf https://deno.land/install.sh | sh
fi
if [[ ! -x "$HOME/.deno/bin/deno" ]] && ! command -v deno >/dev/null 2>&1; then
  echo "Deno was not installed successfully; yt-dlp's YouTube JavaScript support will be unavailable." >&2
  exit 1
fi

# Configure Wayland pointer injection when this distribution ships the
# ydotool system service. Mouse support is optional on hosts without it.
if command -v systemctl >/dev/null 2>&1 && sudo systemctl cat ydotool.service >/dev/null 2>&1; then
  if ! getent group mediabridge-input >/dev/null 2>&1; then sudo groupadd --system mediabridge-input; fi
  sudo usermod -aG mediabridge-input "$(id -un)"
  YDOTOOLD_PATH="$(command -v ydotoold || true)"
  if [[ -n "$YDOTOOLD_PATH" ]]; then
    sudo install -d /etc/systemd/system/ydotool.service.d
    sudo tee /etc/systemd/system/ydotool.service.d/mediabridge.conf >/dev/null <<EOF
[Service]
Group=mediabridge-input
RuntimeDirectory=ydotool
RuntimeDirectoryMode=0770
ExecStart=
ExecStart=$YDOTOOLD_PATH --socket-path=/run/ydotool/socket --socket-perm=0660
EOF
    sudo systemctl daemon-reload
    sudo systemctl enable --now ydotool.service
    sudo systemctl restart ydotool.service
    echo "Configured ydotool for Wayland. Log out and back in to apply the new input group membership."
  fi
fi

# Allow LAN access through recognized firewalls without enabling an inactive one.
if command -v ufw >/dev/null 2>&1 && sudo ufw status 2>/dev/null | grep -q "Status: active"; then
  sudo ufw allow 8765/tcp
elif command -v firewall-cmd >/dev/null 2>&1 && sudo firewall-cmd --state >/dev/null 2>&1; then
  sudo firewall-cmd --query-port=8765/tcp >/dev/null 2>&1 || sudo firewall-cmd --add-port=8765/tcp
  sudo firewall-cmd --permanent --query-port=8765/tcp >/dev/null 2>&1 || sudo firewall-cmd --permanent --add-port=8765/tcp
fi

if [[ "$PACKAGE_ONLY" -eq 1 ]]; then
  echo "MediaBridge runtime dependencies are installed for $DISTRO_ID."
  exit 0
fi

TMP_DIR="$(mktemp -d)"
cleanup() { rm -rf "$TMP_DIR"; }
trap cleanup EXIT
if [[ -n "$LOCAL_ARCHIVE" ]]; then
  if [[ ! -f "$LOCAL_ARCHIVE" ]]; then
    echo "Local build archive not found: $LOCAL_ARCHIVE. Run ./build-linux.sh first." >&2
    exit 1
  fi
  APP_ARCHIVE="$LOCAL_ARCHIVE"
else
  APP_ARCHIVE="$TMP_DIR/$APP_ASSET"
  URL="https://github.com/$REPOSITORY/releases/latest/download/$APP_ASSET"
  if ! curl --fail --location --silent --show-error "$URL" --output "$APP_ARCHIVE"; then
    echo "No downloadable MediaBridge release was found. Build on Linux with ./build-linux.sh, or publish $APP_ASSET to a GitHub Release." >&2
    exit 1
  fi
fi

mkdir -p "$TMP_DIR/mediabridge"
tar -xzf "$APP_ARCHIVE" -C "$TMP_DIR/mediabridge" --no-same-owner
if [[ ! -x "$TMP_DIR/mediabridge/mediabridge" ]]; then
  echo "The archive does not contain an executable named 'mediabridge'." >&2
  exit 1
fi

# LibreWolf's official cross-distro AppImage includes uBlock Origin. Download
# the current stable link from LibreWolf's official installation page.
LIBREWOLF_PAGE="$TMP_DIR/librewolf-installation.html"
curl --fail --location --silent --show-error https://librewolf.net/installation/linux/ --output "$LIBREWOLF_PAGE"
LIBREWOLF_URL="$(grep -oE 'https://dl\.librewolf\.net/librewolf/[^" ]+-linux-x86_64-appimage\.AppImage' "$LIBREWOLF_PAGE" | head -n 1 || true)"
if [[ -z "$LIBREWOLF_URL" ]]; then
  echo "Could not find the official x86_64 LibreWolf AppImage link on the installation page." >&2
  exit 1
fi
LIBREWOLF_FILE="$TMP_DIR/librewolf.AppImage"
curl --fail --location --silent --show-error "$LIBREWOLF_URL" --output "$LIBREWOLF_FILE"
chmod +x "$LIBREWOLF_FILE"
mkdir -p "$TMP_DIR/librewolf-extract"
if ! (cd "$TMP_DIR/librewolf-extract" && "$LIBREWOLF_FILE" --appimage-extract >/dev/null); then
  echo "Could not extract the LibreWolf AppImage. Check that this distribution provides a compatible glibc environment." >&2
  exit 1
fi
if [[ ! -x "$TMP_DIR/librewolf-extract/squashfs-root/AppRun" ]]; then
  echo "LibreWolf's AppImage did not contain its AppRun launcher." >&2
  exit 1
fi

mkdir -p "$APP_DIR" "$BIN_DIR" "$CONFIG_DIR" "$STATE_DIR" "$DESKTOP_DIR" "$SYSTEMD_DIR"
VERSION="$(basename "$LIBREWOLF_URL" | sed -E 's/^librewolf-([0-9.]+-[0-9]+)-linux-x86_64-appimage\.AppImage$/\1/')"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?-[0-9]+$ ]]; then VERSION="current"; fi
LW_STAGING="$APP_DIR/.librewolf-stage-$$"
rm -rf "$LW_STAGING"
mkdir -p "$LW_STAGING"
cp -a "$TMP_DIR/librewolf-extract/squashfs-root" "$LW_STAGING/squashfs-root"
if [[ ! -d "$APP_DIR/librewolf-$VERSION" ]]; then
  mv "$LW_STAGING/squashfs-root" "$APP_DIR/librewolf-$VERSION"
fi
rm -rf "$LW_STAGING"
ln -sfn "librewolf-$VERSION" "$APP_DIR/.librewolf-current-$$"
mv -Tf "$APP_DIR/.librewolf-current-$$" "$APP_DIR/librewolf-current"

install -m 755 "$TMP_DIR/mediabridge/mediabridge" "$APP_DIR/mediabridge"
cat > "$BIN_DIR/mediabridge-librewolf" <<EOF
#!/usr/bin/env bash
set -euo pipefail
APP_DIR="$APP_DIR"
PROFILE="$APP_DIR/librewolf-profile"
REAL_HOME="$HOME"
BROWSER_HOME="$APP_DIR/librewolf-home"
mkdir -p "\$BROWSER_HOME/.librewolf" "\$PROFILE"
if [[ -d "\$REAL_HOME/Downloads" && ! -L "\$BROWSER_HOME/Downloads" && ! -e "\$BROWSER_HOME/Downloads" ]]; then
  ln -s "\$REAL_HOME/Downloads" "\$BROWSER_HOME/Downloads"
fi
if [[ ! -f "\$BROWSER_HOME/.librewolf/librewolf.overrides.cfg" ]]; then
  cat > "\$BROWSER_HOME/.librewolf/librewolf.overrides.cfg" <<'PREFS'
// Preserve auth cookies in MediaBridge's dedicated LibreWolf profile.
pref("privacy.clearOnShutdown.cookies", false);
pref("privacy.clearOnShutdown_v2.cookiesAndStorage", false);
pref("privacy.sanitize.sanitizeOnShutdown", false);
PREFS
fi
mkdir -p "\$PROFILE"
if [[ ! -f "\$PROFILE/user.js" ]]; then
  cat > "\$PROFILE/user.js" <<'PREFS'
// Keep the dedicated MediaBridge browser profile signed in across restarts.
user_pref("privacy.clearOnShutdown.cookies", false);
user_pref("privacy.clearOnShutdown.history", false);
user_pref("privacy.clearOnShutdown.sessions", false);
user_pref("privacy.clearOnShutdown_v2.cookiesAndStorage", false);
user_pref("privacy.clearOnShutdown_v2.historyFormDataAndDownloads", false);
user_pref("privacy.clearOnShutdown_v2.siteSettings", false);
user_pref("privacy.sanitize.sanitizeOnShutdown", false);
PREFS
fi
export HOME="\$BROWSER_HOME"
unset XDG_CONFIG_HOME XDG_CACHE_HOME XDG_DATA_HOME
exec "\$APP_DIR/librewolf-current/squashfs-root/AppRun" --profile "\$PROFILE" "\$@"
EOF
chmod 755 "$BIN_DIR/mediabridge-librewolf"

cat > "$BIN_DIR/mediabridge-dashboard" <<EOF
#!/usr/bin/env bash
set -euo pipefail
PORT="\$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("bind", "0.0.0.0:8765").rsplit(":",1)[-1])' "$CONFIG_DIR/mediabridge.json" 2>/dev/null || echo 8765)"
if ! systemctl --user start mediabridge.service 2>/dev/null; then
  if ! curl --fail --silent --max-time 1 "http://127.0.0.1:\$PORT/status" >/dev/null; then
    "$APP_DIR/mediabridge" >/dev/null 2>&1 &
  fi
fi
exec "$BIN_DIR/mediabridge-librewolf" "http://127.0.0.1:\$PORT/"
EOF
chmod 755 "$BIN_DIR/mediabridge-dashboard"

cat > "$BIN_DIR/mediabridge-logs" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec xdg-open "$STATE_DIR/mediabridge.log"
EOF
chmod 755 "$BIN_DIR/mediabridge-logs"

cat > "$BIN_DIR/mediabridge-autostart" <<EOF
#!/usr/bin/env bash
set -euo pipefail
systemctl --user import-environment DISPLAY WAYLAND_DISPLAY XAUTHORITY DBUS_SESSION_BUS_ADDRESS XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_RUNTIME_DIR 2>/dev/null || true
exec systemctl --user restart mediabridge.service
EOF
chmod 755 "$BIN_DIR/mediabridge-autostart"
HAS_USER_SYSTEMD=0
if command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then HAS_USER_SYSTEMD=1; fi

cat > "$SYSTEMD_DIR/mediabridge.service" <<EOF
[Unit]
Description=MediaBridge LAN media playback service and desktop tray icon
After=graphical-session.target network.target

[Service]
Type=simple
Environment="PATH=%h/.local/bin:%h/.deno/bin:/usr/local/bin:/usr/bin:/bin"
Environment="MEDIABRIDGE_LIBREWOLF_PROFILE=$APP_DIR/librewolf-profile"
Environment="MEDIABRIDGE_LIBREWOLF_LAUNCHER=$BIN_DIR/mediabridge-librewolf"
ExecStart="$APP_DIR/mediabridge"
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
EOF

cat > "$DESKTOP_DIR/mediabridge.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=MediaBridge Dashboard
Comment=Open the MediaBridge dashboard in its bundled LibreWolf browser
Exec="$BIN_DIR/mediabridge-dashboard"
Icon=multimedia-video-player
Terminal=false
Categories=AudioVideo;Network;
EOF
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
if [[ "$HAS_USER_SYSTEMD" -eq 1 ]]; then
  cat > "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/mediabridge.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=MediaBridge service and tray
Exec="$BIN_DIR/mediabridge-autostart"
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
else
  cat > "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/mediabridge.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=MediaBridge service and tray
Exec="$APP_DIR/mediabridge"
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
fi

if [[ "$HAS_USER_SYSTEMD" -eq 1 ]]; then
  systemctl --user daemon-reload || true
  if [[ "$NO_AUTOSTART" -eq 0 ]] && systemctl --user enable --now mediabridge.service; then
    echo "MediaBridge is installed and enabled for this user. Its tray icon opens the dashboard in bundled LibreWolf."
  else
    echo "MediaBridge is installed. Start it with: systemctl --user start mediabridge.service"
  fi
else
  echo "MediaBridge is installed. This system has no systemd user manager; use the desktop menu entry to open the dashboard."
fi
echo "LibreWolf profile and YouTube cookies persist in: $APP_DIR/librewolf-profile"
echo "MediaBridge logs: $STATE_DIR/mediabridge.log"
