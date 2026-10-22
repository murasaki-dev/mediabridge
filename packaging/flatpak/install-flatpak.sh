#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
for bundle in MediaBridge-Platform.flatpak MediaBridge.flatpak; do
  [[ -f "$HERE/$bundle" ]] || { echo "Missing $HERE/$bundle (copy both bundles)." >&2; exit 1; }
done
[[ -f "$HERE/start-flatpak.sh" ]] || { echo "Missing $HERE/start-flatpak.sh (copy the launch helper too)." >&2; exit 1; }
# Calling this helper (or building with --install) explicitly requests installation.
# Do not rely on stdin being an interactive terminal after a long container build.
flatpak install --user --noninteractive --assumeyes --or-update --bundle "$HERE/MediaBridge-Platform.flatpak"
flatpak install --user --noninteractive --assumeyes --or-update --bundle "$HERE/MediaBridge.flatpak"
bash "$HERE/start-flatpak.sh"
echo 'MediaBridge was launched independently of this terminal.'
echo 'Later launches: use GNOME or bash start-flatpak.sh; add --background for server only.'
