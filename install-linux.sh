#!/usr/bin/env bash
set -euo pipefail

# User-facing installer for common glibc Linux distributions. It installs
# runtime dependencies with the host package manager and installs MediaBridge
# LibreWolf and Moonlight entirely inside the current user's home directory.
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
DISTROBOX_RUNTIME=0
DISTROBOX_NAME="mediabridge-runtime"
DISTROBOX_MARKER="$STATE_DIR/distrobox-runtime"
NEEDS_SESSION_REFRESH=0

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
if [[ ! -r /etc/os-release ]]; then
  echo "Cannot identify this Linux distribution (missing /etc/os-release)." >&2
  exit 1
fi
# shellcheck disable=SC1091
. /etc/os-release
DISTRO_ID="${ID:-unknown}"
DISTRO_LIKE="${ID_LIKE:-}"

# Bazzite and SteamOS use image-based/read-only hosts. Keep their runtime
# packages in a dedicated user-owned Distrobox rather than changing the host
# image (or installing packages that an OS update may erase).
if [[ "$DISTRO_ID" == "bazzite" || "$DISTRO_ID" == "steamos" ]]; then
  DISTROBOX_RUNTIME=1
  if ! command -v distrobox >/dev/null 2>&1 || ! command -v podman >/dev/null 2>&1; then
    echo "$DISTRO_ID needs Distrobox and Podman for the no-host-changes install path." >&2
    echo "Install/enable those user-space tools first, then rerun this installer. It will not unlock the system image or layer packages." >&2
    exit 1
  fi
  if [[ ! -f "$DISTROBOX_MARKER" ]] && distrobox enter --name "$DISTROBOX_NAME" --headless -- true >/dev/null 2>&1; then
    echo "A Distrobox named '$DISTROBOX_NAME' already exists but is not marked as MediaBridge-managed." >&2
    echo "Rename or remove that container, then rerun this installer so it does not alter another app's environment." >&2
    exit 1
  fi
  if ! distrobox enter --name "$DISTROBOX_NAME" --headless -- true >/dev/null 2>&1; then
    echo "Creating the MediaBridge user-space Fedora runtime. The first setup downloads a Fedora container image."
    distrobox create --name "$DISTROBOX_NAME" --image docker.io/library/fedora:latest --yes
    mkdir -p "$STATE_DIR"
    touch "$DISTROBOX_MARKER"
  fi
  echo "Installing MediaBridge runtime packages inside the $DISTROBOX_NAME Distrobox."
  distrobox enter --name "$DISTROBOX_NAME" --headless -- bash -lc \
    'sudo dnf install -y ca-certificates curl mpv yt-dlp playerctl xdotool ydotool xdg-utils python3 python3-secretstorage libnotify gtk3 libappindicator-gtk3'
  PACKAGE_MANAGER="Distrobox (Fedora)"
else
  command -v sudo >/dev/null 2>&1 || { echo "sudo is required to install system dependencies." >&2; exit 1; }
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
fi
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
  if ! id -nG "$(id -un)" | tr ' ' '\n' | grep -Fxq mediabridge-input; then
    sudo usermod -aG mediabridge-input "$(id -un)"
    NEEDS_SESSION_REFRESH=1
  fi
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
    echo "Configured ydotool for Wayland. The new input group membership will apply at your next login."
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
  if curl --fail --location --silent --show-error "$URL" --output "$APP_ARCHIVE"; then
    :
  else
    CURL_STATUS=$?
    if [[ "$CURL_STATUS" -eq 7 || "$CURL_STATUS" -eq 6 || "$CURL_STATUS" -eq 28 ]]; then
      echo "Could not connect to GitHub to download MediaBridge (curl exit $CURL_STATUS). Check this machine's internet connection, DNS, firewall, or proxy, then retry." >&2
      echo "Requested URL: $URL" >&2
    elif [[ "$CURL_STATUS" -eq 22 ]]; then
      echo "GitHub responded, but the latest release asset was unavailable. Confirm the public latest release contains an asset named exactly '$APP_ASSET'." >&2
      echo "Requested URL: $URL" >&2
      echo "For a local build, run ./build-linux.sh and then ./install-linux.sh --local." >&2
    else
      echo "Could not download MediaBridge from GitHub (curl exit $CURL_STATUS). Requested URL: $URL" >&2
    fi
    exit 1
  fi
fi

mkdir -p "$TMP_DIR/mediabridge"
tar -xzf "$APP_ARCHIVE" -C "$TMP_DIR/mediabridge" --no-same-owner
if [[ ! -x "$TMP_DIR/mediabridge/mediabridge" ]]; then
  echo "The archive does not contain an executable named 'mediabridge'." >&2
  exit 1
fi

# Install Moonlight's official x86_64 AppImage privately with MediaBridge.
# Extract it so launching Moonlight does not depend on AppImage/FUSE support.
MOONLIGHT_RELEASE="$TMP_DIR/moonlight-release.json"
curl --fail --location --silent --show-error \
  -H 'Accept: application/vnd.github+json' -H 'User-Agent: MediaBridge-Linux-Installer' \
  https://api.github.com/repos/moonlight-stream/moonlight-qt/releases/latest \
  --output "$MOONLIGHT_RELEASE"
MOONLIGHT_METADATA="$TMP_DIR/moonlight-metadata"
python3 - "$MOONLIGHT_RELEASE" "$MOONLIGHT_METADATA" <<'PY'
import json, re, sys
release = json.load(open(sys.argv[1], encoding="utf-8"))
assets = [a for a in release.get("assets", []) if re.fullmatch(r"Moonlight-.+-x86_64\.AppImage", a.get("name", ""))]
if not assets:
    raise SystemExit("Could not find the official x86_64 Moonlight AppImage in the latest release.")
asset = assets[0]
with open(sys.argv[2], "w", encoding="utf-8") as out:
    out.write(release.get("tag_name", "current") + "\n" + asset["browser_download_url"] + "\n")
PY
MOONLIGHT_VERSION="$(sed -n '1p' "$MOONLIGHT_METADATA" | tr -cd '[:alnum:]._-')"
MOONLIGHT_URL="$(sed -n '2p' "$MOONLIGHT_METADATA")"
MOONLIGHT_FILE="$TMP_DIR/moonlight.AppImage"
curl --fail --location --silent --show-error "$MOONLIGHT_URL" --output "$MOONLIGHT_FILE"
chmod +x "$MOONLIGHT_FILE"
mkdir -p "$TMP_DIR/moonlight-extract"
if ! (cd "$TMP_DIR/moonlight-extract" && "$MOONLIGHT_FILE" --appimage-extract >/dev/null); then
  echo "Could not extract the Moonlight AppImage. Check that this distribution provides a compatible glibc environment." >&2
  exit 1
fi
if [[ ! -x "$TMP_DIR/moonlight-extract/squashfs-root/AppRun" ]]; then
  echo "Moonlight's AppImage did not contain its AppRun launcher." >&2
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
# Keep deployment-specific provider/decoder settings across reinstalls. This
# file is trusted shell configuration, never a place for SMB passwords.
if [[ ! -e "$CONFIG_DIR/runtime.env" ]]; then
  (umask 077; cat > "$CONFIG_DIR/runtime.env" <<'EOF'
# Loaded by mediabridge-run for service and desktop launches. Shell syntax.
MEDIABRIDGE_LOG_STDERR=1
MEDIABRIDGE_MPV_HWDEC=auto
# Supply your own provider executable, or configure a companion service.
# MEDIABRIDGE_SMB_CREDENTIAL_PROVIDER=/absolute/path/to/smb-provider
# MEDIABRIDGE_SMB_CONFIG_DIR=/absolute/path/to/companion-config
# MEDIABRIDGE_SMB_ROOM_PORT=8081
# Never store SMB passwords or service tokens here.
EOF
  )
fi
chmod 600 "$CONFIG_DIR/runtime.env"
cat > "$BIN_DIR/mediabridge-run" <<EOF
#!/usr/bin/env bash
set -euo pipefail
export XDG_CONFIG_HOME="$(dirname -- "$CONFIG_DIR")"
export XDG_DATA_HOME="$(dirname -- "$APP_DIR")"
export XDG_STATE_HOME="$(dirname -- "$STATE_DIR")"
export MEDIABRIDGE_LIBREWOLF_PROFILE="$APP_DIR/librewolf-profile"
export MEDIABRIDGE_LIBREWOLF_LAUNCHER="$BIN_DIR/mediabridge-librewolf"
export MEDIABRIDGE_LOG_STDERR="\${MEDIABRIDGE_LOG_STDERR:-1}"
if [[ -f "$CONFIG_DIR/runtime.env" ]]; then
  set -a
  source "$CONFIG_DIR/runtime.env"
  set +a
fi
EOF
if [[ "$DISTROBOX_RUNTIME" -eq 1 ]]; then
  cat >> "$BIN_DIR/mediabridge-run" <<'EOF'
# Distrobox can filter the parent environment; forward runtime settings explicitly.
runtime_env=()
for name in XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME \
  MEDIABRIDGE_LIBREWOLF_PROFILE MEDIABRIDGE_LIBREWOLF_LAUNCHER \
  MEDIABRIDGE_LIBREWOLF_NO_BIDI MEDIABRIDGE_LOG_STDERR MEDIABRIDGE_MPV_HWDEC \
  MEDIABRIDGE_SMB_CREDENTIAL_PROVIDER MEDIABRIDGE_SMB_CONFIG_DIR MEDIABRIDGE_SMB_ROOM_PORT; do
  if [[ -v "$name" ]]; then runtime_env+=("$name=${!name}"); fi
done
EOF
  cat >> "$BIN_DIR/mediabridge-run" <<EOF
exec distrobox enter --name "$DISTROBOX_NAME" --headless -- env \\
  "\${runtime_env[@]}" \\
  "PATH=$HOME/.local/bin:$HOME/.deno/bin:/usr/local/bin:/usr/bin:/bin" \\
  "\$@"
EOF
else
  cat >> "$BIN_DIR/mediabridge-run" <<'EOF'
exec "$@"
EOF
fi
chmod 755 "$BIN_DIR/mediabridge-run"
if [[ -f "$SCRIPT_DIR/uninstall-linux.sh" ]]; then
  install -m 755 "$SCRIPT_DIR/uninstall-linux.sh" "$BIN_DIR/mediabridge-uninstall"
else
  curl --fail --location --silent --show-error \
    "https://raw.githubusercontent.com/$REPOSITORY/main/uninstall-linux.sh" \
    --output "$BIN_DIR/mediabridge-uninstall"
  chmod 755 "$BIN_DIR/mediabridge-uninstall"
fi
VERSION="$(basename "$LIBREWOLF_URL" | sed -E 's/^librewolf-([0-9.]+-[0-9]+)-linux-x86_64-appimage\.AppImage$/\1/')"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?-[0-9]+$ ]]; then VERSION="unversioned"; fi
LW_STAGING="$APP_DIR/.librewolf-stage-$$"
rm -rf "$LW_STAGING"
mkdir -p "$LW_STAGING"
cp -a "$TMP_DIR/librewolf-extract/squashfs-root" "$LW_STAGING/squashfs-root"
if [[ "$VERSION" == "unversioned" || ! -x "$APP_DIR/librewolf-$VERSION/squashfs-root/AppRun" ]]; then
  rm -rf "$APP_DIR/librewolf-$VERSION"
  mv "$LW_STAGING" "$APP_DIR/librewolf-$VERSION"
else
  rm -rf "$LW_STAGING"
fi
ln -sfn "librewolf-$VERSION" "$APP_DIR/.librewolf-current-$$"
mv -Tf "$APP_DIR/.librewolf-current-$$" "$APP_DIR/librewolf-current"
if [[ ! -x "$APP_DIR/librewolf-current/squashfs-root/AppRun" ]]; then
  echo "LibreWolf installation is incomplete: its AppRun launcher is missing." >&2
  exit 1
fi

MOONLIGHT_STAGING="$APP_DIR/.moonlight-stage-$$"
rm -rf "$MOONLIGHT_STAGING"
mkdir -p "$MOONLIGHT_STAGING"
cp -a "$TMP_DIR/moonlight-extract/squashfs-root" "$MOONLIGHT_STAGING/squashfs-root"
if [[ ! -x "$APP_DIR/moonlight-$MOONLIGHT_VERSION/squashfs-root/AppRun" ]]; then
  rm -rf "$APP_DIR/moonlight-$MOONLIGHT_VERSION"
  mv "$MOONLIGHT_STAGING" "$APP_DIR/moonlight-$MOONLIGHT_VERSION"
else
  rm -rf "$MOONLIGHT_STAGING"
fi
ln -sfn "moonlight-$MOONLIGHT_VERSION" "$APP_DIR/.moonlight-current-$$"
mv -Tf "$APP_DIR/.moonlight-current-$$" "$APP_DIR/moonlight-current"
if [[ ! -x "$APP_DIR/moonlight-current/squashfs-root/AppRun" ]]; then
  echo "Moonlight installation is incomplete: its AppRun launcher is missing." >&2
  exit 1
fi

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
pref("media.autoplay.default", 0);
pref("media.autoplay.blocking_policy", 0);
pref("devtools.debugger.remote-enabled", true);
PREFS
fi
if ! grep -q 'media.autoplay.default' "\$BROWSER_HOME/.librewolf/librewolf.overrides.cfg"; then
  printf '\npref("media.autoplay.default", 0);\npref("media.autoplay.blocking_policy", 0);\n' >> "\$BROWSER_HOME/.librewolf/librewolf.overrides.cfg"
fi
if ! grep -q 'devtools.debugger.remote-enabled' "\$BROWSER_HOME/.librewolf/librewolf.overrides.cfg"; then
  printf '\npref("devtools.debugger.remote-enabled", true);\n' >> "\$BROWSER_HOME/.librewolf/librewolf.overrides.cfg"
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
user_pref("media.autoplay.default", 0);
user_pref("media.autoplay.blocking_policy", 0);
user_pref("browser.aboutwelcome.enabled", false);
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.startup.firstrunSkipsHomepage", true);
user_pref("security.insecure_connection_text.enabled", false);
user_pref("privacy.resistFingerprinting", true);
user_pref("devtools.debugger.remote-enabled", true);
PREFS
fi
for pref in \
  'browser.aboutwelcome.enabled", false' \
  'browser.shell.checkDefaultBrowser", false' \
  'browser.startup.firstrunSkipsHomepage", true' \
  'security.insecure_connection_text.enabled", false' \
  'privacy.resistFingerprinting", true' \
  'devtools.debugger.remote-enabled", true'; do
  if ! grep -q "\$pref" "\$PROFILE/user.js"; then
    printf 'user_pref("%s);\n' "\$pref" >> "\$PROFILE/user.js"
  fi
done
if ! grep -q 'media.autoplay.default' "\$PROFILE/user.js"; then
  printf '\nuser_pref("media.autoplay.default", 0);\nuser_pref("media.autoplay.blocking_policy", 0);\n' >> "\$PROFILE/user.js"
fi
export HOME="\$BROWSER_HOME"
unset XDG_CONFIG_HOME XDG_CACHE_HOME XDG_DATA_HOME
if [[ "\${MEDIABRIDGE_LIBREWOLF_NO_BIDI:-0}" == 1 ]]; then
  exec "\$APP_DIR/librewolf-current/squashfs-root/AppRun" --profile "\$PROFILE" "\$@"
fi
exec "\$APP_DIR/librewolf-current/squashfs-root/AppRun" --profile "\$PROFILE" --remote-debugging-port 9222 "\$@"
EOF
chmod 755 "$BIN_DIR/mediabridge-librewolf"

cat > "$BIN_DIR/mediabridge-dashboard" <<EOF
#!/usr/bin/env bash
set -euo pipefail
PORT="\$("$BIN_DIR/mediabridge-run" python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("bind", "0.0.0.0:8765").rsplit(":",1)[-1])' "$CONFIG_DIR/mediabridge.json" 2>/dev/null || echo 8765)"
if ! systemctl --user start mediabridge.service 2>/dev/null; then
  if ! curl --fail --silent --max-time 1 "http://127.0.0.1:\$PORT/status" >/dev/null; then
    "$BIN_DIR/mediabridge-run" "$APP_DIR/mediabridge" >/dev/null 2>&1 &
  fi
fi
exec "$BIN_DIR/mediabridge-run" "$BIN_DIR/mediabridge-librewolf" "http://127.0.0.1:\$PORT/"
EOF
chmod 755 "$BIN_DIR/mediabridge-dashboard"

cat > "$BIN_DIR/mediabridge-logs" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$BIN_DIR/mediabridge-run" xdg-open "$STATE_DIR/mediabridge.log"
EOF
chmod 755 "$BIN_DIR/mediabridge-logs"

cat > "$BIN_DIR/mediabridge" <<EOF
#!/usr/bin/env bash
set -euo pipefail
case "\${1:-}" in
  login)
    shift
    exec "$BIN_DIR/mediabridge-run" env MEDIABRIDGE_LIBREWOLF_NO_BIDI=1 "$BIN_DIR/mediabridge-librewolf" "\${@:-https://accounts.google.com/}"
    ;;
  browser)
    shift
    if [[ \$# -eq 0 ]]; then
      exec "$BIN_DIR/mediabridge-dashboard"
    fi
    exec "$BIN_DIR/mediabridge-run" "$BIN_DIR/mediabridge-librewolf" "\$@"
    ;;
  dashboard)
    exec "$BIN_DIR/mediabridge-dashboard"
    ;;
  logs)
    exec "$BIN_DIR/mediabridge-logs"
    ;;
  uninstall)
    exec "$BIN_DIR/mediabridge-uninstall" "\${@:2}"
    ;;
  *)
    echo "Usage: mediabridge {login|browser|dashboard|logs|uninstall}"
    echo "  login     Open Google sign-in in managed LibreWolf without BiDi; close other LibreWolf windows first"
    echo "  browser   Open the dashboard in the managed LibreWolf profile (or a supplied URL)"
    echo "  dashboard Open the MediaBridge dashboard in managed LibreWolf"
    echo "  logs      Open the MediaBridge log file"
    echo "  uninstall Run the MediaBridge uninstaller"
    exit 2
    ;;
esac
EOF
chmod 755 "$BIN_DIR/mediabridge"

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
ExecStart="$BIN_DIR/mediabridge-run" "$APP_DIR/mediabridge"
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
Exec="$BIN_DIR/mediabridge-run" "$APP_DIR/mediabridge"
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
fi

if [[ "$HAS_USER_SYSTEMD" -eq 1 ]]; then
  # The installer may start the service before desktop autostart runs. Import
  # this live GNOME/KDE/Wayland session first so mpv and browsers can create
  # visible windows immediately, rather than waiting for the next login.
  systemctl --user import-environment DISPLAY WAYLAND_DISPLAY XAUTHORITY DBUS_SESSION_BUS_ADDRESS XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_RUNTIME_DIR 2>/dev/null || true
  systemctl --user daemon-reload || true
  if [[ "$NO_AUTOSTART" -eq 0 ]] && systemctl --user enable --now mediabridge.service; then
    echo "MediaBridge is installed and enabled for this user. Its tray icon opens the dashboard in bundled LibreWolf."
  else
    echo "MediaBridge is installed. Start it with: systemctl --user start mediabridge.service"
  fi
else
  echo "MediaBridge is installed. This system has no systemd user manager; use the desktop menu entry to open the dashboard."
fi
if [[ "$HAS_USER_SYSTEMD" -eq 1 ]] && systemctl --user is-active --quiet mediabridge.service; then
  echo "MediaBridge server status: running. Check it with: systemctl --user status mediabridge.service"
else
  echo "Check whether the server is reachable locally with: curl -i http://127.0.0.1:8765/status"
  if [[ "$HAS_USER_SYSTEMD" -eq 1 ]]; then
    echo "If it is not running, inspect its log with: journalctl --user -u mediabridge.service -n 80 --no-pager"
  fi
fi
echo "Default dashboard address: http://127.0.0.1:8765/ (the configured port may differ)."
echo "LibreWolf profile and YouTube cookies persist in: $APP_DIR/librewolf-profile"
echo "MediaBridge logs: $STATE_DIR/mediabridge.log"
echo "Runtime settings (preserved on reinstall): $CONFIG_DIR/runtime.env"
echo "SMB: guest playback needs SMB-enabled mpv; authenticated/relay playback needs a separately configured provider."
if [[ "$NEEDS_SESSION_REFRESH" -eq 1 ]]; then
  echo "No restart is needed to run MediaBridge. Log out and back in later (or restart) only to activate the ydotool input group; you do not need to rerun this installer."
fi
if [[ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]]; then
  # Open the onboarding dashboard in the managed browser. The dashboard lets
  # the user open each supported sign-in page and verify cookie access.
  nohup "$BIN_DIR/mediabridge-dashboard" >>"$STATE_DIR/dashboard-launch.log" 2>&1 </dev/null &
  echo "Opening the MediaBridge dashboard in bundled LibreWolf. If it does not appear, inspect: $STATE_DIR/dashboard-launch.log"
else
  echo "No graphical session was detected, so the dashboard was not opened automatically. Open it later with: $BIN_DIR/mediabridge-dashboard"
fi
