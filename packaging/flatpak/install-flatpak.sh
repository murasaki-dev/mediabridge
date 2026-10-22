#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
for bundle in MediaBridge-Platform.flatpak MediaBridge.flatpak; do
  [[ -f "$HERE/$bundle" ]] || { echo "Missing $HERE/$bundle (copy both bundles)." >&2; exit 1; }
done
[[ -f "$HERE/start-flatpak.sh" ]] || { echo "Missing $HERE/start-flatpak.sh (copy the launch helper too)." >&2; exit 1; }
# NVIDIA user-space libraries must match the host kernel driver. Reuse Flatpak's
# versioned extensions instead of shipping a fixed, potentially incompatible one.
nvidia_drivers=()
if graphics_drivers="$(flatpak --gl-drivers)"; then
  while IFS= read -r driver; do
    if [[ "$driver" =~ ^nvidia-[0-9]+(-[0-9]+)+$ ]]; then nvidia_drivers+=("$driver"); fi
  done < <(printf '%s\n' "$graphics_drivers" | tr '; ' '\n')
else
  echo 'Could not detect graphics drivers; NVIDIA extension setup was skipped.' >&2
fi
if (( ${#nvidia_drivers[@]} )); then
  flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
fi
# Calling this helper (or building with --install) explicitly requests installation.
# Do not rely on stdin being an interactive terminal after a long container build.
echo 'Stopping any running MediaBridge instance before updating...'
# An absent instance is normal on first install. Check afterwards so a real
# failure to stop cannot silently leave the old runtime running after an update.
flatpak kill io.github.murasaki_dev.MediaBridge 2>/dev/null || true
running_apps="$(flatpak ps --columns=application)"
if grep -Fxq io.github.murasaki_dev.MediaBridge <<< "$running_apps"; then
  echo 'MediaBridge is still running; installation stopped. Quit it and try again.' >&2
  exit 1
fi
flatpak install --user --noninteractive --assumeyes --or-update --bundle "$HERE/MediaBridge-Platform.flatpak"
flatpak install --user --noninteractive --assumeyes --or-update --bundle "$HERE/MediaBridge.flatpak"
for driver in "${nvidia_drivers[@]}"; do
  echo "Installing the Flatpak graphics extension matching $driver..."
  flatpak install --user --noninteractive --assumeyes --or-update flathub "org.freedesktop.Platform.GL.$driver//1.4"
done
bash "$HERE/start-flatpak.sh"
echo 'MediaBridge was launched independently of this terminal.'
echo 'Later launches: use GNOME or bash start-flatpak.sh; add --background for server only.'
