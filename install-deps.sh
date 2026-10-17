#!/usr/bin/env bash
set -euo pipefail

# Installs build tools, Chromium, mpv/yt-dlp, and MPRIS tooling on common Linux distributions.
# Moonlight is optional and is installed separately because its package source varies.
if [[ $EUID -eq 0 ]]; then SUDO=; else SUDO=sudo; fi

if command -v apt-get >/dev/null 2>&1; then
  $SUDO apt-get update
  browser_package=chromium
  if ! apt-cache show "$browser_package" >/dev/null 2>&1; then browser_package=chromium-browser; fi
  $SUDO apt-get install -y build-essential pkg-config curl ca-certificates unzip xdg-utils playerctl xdotool ydotool mpv yt-dlp python3-secretstorage libnotify-bin "$browser_package"
elif command -v dnf >/dev/null 2>&1; then
  $SUDO dnf install -y gcc make pkgconf-pkg-config curl ca-certificates unzip xdg-utils playerctl xdotool ydotool chromium mpv yt-dlp python3-secretstorage libnotify
elif command -v pacman >/dev/null 2>&1; then
  # Pacman's signature checks require a correct clock and an initialized,
  # current Arch keyring. Old or rarely updated installations commonly need
  # this bootstrap before package signatures can be verified again.
  if command -v timedatectl >/dev/null 2>&1; then
    if ! $SUDO timedatectl set-ntp true; then
      echo "Could not enable network time synchronization; package signature verification may fail if the system clock is wrong." >&2
    fi
    ntp_synced=no
    for _ in {1..30}; do
      if [[ "$(timedatectl show -p NTPSynchronized --value 2>/dev/null || true)" == "yes" ]]; then
        ntp_synced=yes
        break
      fi
      sleep 1
    done
    if [[ "$ntp_synced" != yes ]]; then
      echo "System time is not yet synchronized. Continuing, but pacman may reject package signatures if the clock is inaccurate." >&2
    fi
  fi
  $SUDO pacman-key --init
  $SUDO pacman-key --populate archlinux

  # Arch systems must be upgraded as a whole. Refresh the signing keyring,
  # then perform a full upgrade together with the required packages to avoid
  # leaving the machine in a partial-upgrade state.
  $SUDO pacman -Sy --needed --noconfirm archlinux-keyring
  $SUDO pacman -Su --needed --noconfirm base-devel pkgconf curl ca-certificates unzip xdg-utils playerctl xdotool ydotool chromium mpv yt-dlp libnotify
elif command -v zypper >/dev/null 2>&1; then
  $SUDO zypper --non-interactive install -t pattern devel_basis
  $SUDO zypper --non-interactive install pkg-config curl ca-certificates unzip xdg-utils playerctl xdotool ydotool chromium mpv yt-dlp libnotify-tools
else
  echo "Unsupported package manager. Install a C build toolchain, pkg-config, curl, ca-certificates, unzip, xdg-utils, playerctl, xdotool, ydotool, Chromium, mpv, yt-dlp, and notify-send (libnotify) manually." >&2
  exit 1
fi

# Verify the mouse tools used by /mouse are available after package installation.
missing_mouse_tools=()
for tool in xdotool ydotool ydotoold; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    missing_mouse_tools+=("$tool")
  fi
done
if ((${#missing_mouse_tools[@]})); then
  echo "Missing mouse input tool(s): ${missing_mouse_tools[*]}. Install them with your distribution's package manager before using /mouse." >&2
  exit 1
fi
echo "Mouse tools found: xdotool, ydotool, and ydotoold."

# Wayland uses ydotoold, which must hold the virtual input device open.
# Give the desktop user access to a dedicated socket group, then point both
# the daemon and MediaBridge at the same socket instead of the package defaults.
if command -v systemctl >/dev/null 2>&1 && $SUDO systemctl cat ydotool.service >/dev/null 2>&1; then
  if ! getent group mediabridge-input >/dev/null 2>&1; then
    $SUDO groupadd --system mediabridge-input
  fi
  desktop_user="${SUDO_USER:-${USER:-}}"
  if [[ -z "$desktop_user" || "$desktop_user" == "root" ]]; then
    echo "Could not identify the desktop user for ydotool socket access; skipping service configuration." >&2
  else
    $SUDO usermod -aG mediabridge-input "$desktop_user"
    ydotoold_path="$(command -v ydotoold)"
    $SUDO install -d /etc/systemd/system/ydotool.service.d
    $SUDO tee /etc/systemd/system/ydotool.service.d/mediabridge.conf >/dev/null <<EOF
[Service]
Group=mediabridge-input
RuntimeDirectory=ydotool
RuntimeDirectoryMode=0770
ExecStart=
ExecStart=${ydotoold_path} --socket-path=/run/ydotool/socket --socket-perm=0660
EOF
    if $SUDO systemctl daemon-reload && $SUDO systemctl enable --now ydotool.service && $SUDO systemctl restart ydotool.service; then
      echo "Configured ydotool.service with a shared /run/ydotool/socket for group mediabridge-input."
      echo "Added $desktop_user to mediabridge-input. Log out and back in before running MediaBridge so the new group takes effect."
    else
      echo "Could not configure ydotool.service. On Wayland, configure ydotoold with a socket accessible to the MediaBridge desktop user." >&2
    fi
  fi
else
  echo "No ydotool.service unit found. On Wayland, start ydotoold with a socket accessible to the MediaBridge desktop user and permission to access /dev/uinput."
fi

# Open the MediaBridge HTTP port in a recognized host firewall. Do not enable
# an inactive firewall; that could change unrelated network access.
if command -v ufw >/dev/null 2>&1 && $SUDO ufw status 2>/dev/null | grep -q "Status: active"; then
  $SUDO ufw allow 8765/tcp
  echo "Allowed TCP port 8765 through UFW."
elif command -v firewall-cmd >/dev/null 2>&1 && $SUDO firewall-cmd --state >/dev/null 2>&1; then
  if ! $SUDO firewall-cmd --query-port=8765/tcp >/dev/null 2>&1; then
    $SUDO firewall-cmd --add-port=8765/tcp
  fi
  if ! $SUDO firewall-cmd --permanent --query-port=8765/tcp >/dev/null 2>&1; then
    $SUDO firewall-cmd --permanent --add-port=8765/tcp
  fi
  echo "Allowed TCP port 8765 through firewalld (current and permanent rules)."
elif command -v ufw >/dev/null 2>&1; then
  $SUDO ufw allow 8765/tcp
  echo "Added a UFW rule for TCP 8765. UFW is currently inactive, so the rule will apply if it is enabled later."
else
  echo "No active UFW or firewalld manager detected. If another firewall is enabled, allow inbound TCP port 8765 there."
fi

# Install Deno for yt-dlp's YouTube JavaScript challenge solver (EJS).
# Respect DENO_INSTALL so installation, PATH setup, and verification agree.
deno_install_dir="${DENO_INSTALL:-${HOME}/.deno}"
export DENO_INSTALL="$deno_install_dir"
if ! command -v deno >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://deno.land/install.sh | sh
fi
export DENO_INSTALL="$deno_install_dir"
export PATH="${DENO_INSTALL}/bin:${PATH}"
if ! command -v deno >/dev/null 2>&1; then
  echo "Deno was not found on PATH or at ${DENO_INSTALL}/bin/deno after installation. The Deno installer needs unzip or 7z to extract its download; install unzip and rerun this script." >&2
  echo "Check the installer output above, then run: ${DENO_INSTALL}/bin/deno --version" >&2
  exit 1
fi

# Rust's official rustup installer keeps the compiler current across distros.
if ! command -v rustup >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
# Make rustup available in this script's process, including after first install.
export PATH="${HOME}/.cargo/bin:${PATH}"
rustup toolchain install stable --profile minimal
rustup default stable

echo
echo "Installed build tools, Chromium, mpv, yt-dlp, Deno, optional Chromium cookie support, and playerctl. Deno: $(deno --version | head -n 1); Rust: $(rustc --version); Cargo: $(cargo --version)"
echo "Moonlight is optional for building. Install Moonlight Qt separately to use /sunshine."
