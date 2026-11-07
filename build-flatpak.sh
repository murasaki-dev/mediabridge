#!/usr/bin/env bash
# Local Fedora prototype: build the app and a matching private runtime.
set -euo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
APP_ID=io.github.murasaki_dev.MediaBridge
PLATFORM_ID="$APP_ID.Platform"
BRANCH=f44
IMAGE="${MEDIABRIDGE_FLATPAK_IMAGE:-registry.fedoraproject.org/fedora:44}"
INSTALL=0
for arg in "$@"; do
  case "$arg" in
    --install) INSTALL=1 ;;
    --help|-h)
      echo 'Usage: bash build-flatpak.sh [--install]'
      echo 'Requires Linux x86_64, rootless Podman, Flatpak, tar and Python 3.'
      echo 'Builds local app/runtime bundles in dist/flatpak. No publication or host package installation.'
      exit 0 ;;
    *) echo "Unknown argument: $arg" >&2; exit 2 ;;
  esac
done
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || {
  echo 'Build on x86_64 Linux (Fedora recommended), not Windows.' >&2; exit 1;
}
[[ "$EUID" != 0 ]] || { echo 'Run as your normal desktop user, not root.' >&2; exit 1; }
for tool in podman flatpak tar python3; do
  command -v "$tool" >/dev/null || {
    echo "Missing $tool. On Fedora: sudo dnf install podman flatpak python3 tar" >&2; exit 1;
  }
done
podman info >/dev/null
OUT="$ROOT_DIR/dist/flatpak"
mkdir -p "$OUT"
exec > >(tee "$OUT/build.log") 2>&1
WORK="$(mktemp -d "$OUT/work.XXXXXXXX")"
CONTAINER=""
cleanup() {
  local build_status=$?
  if [[ -n "$CONTAINER" ]]; then podman rm --force "$CONTAINER" >/dev/null 2>&1 || true; fi
  # WORK is always a newly created child of our output directory.
  chmod -R u+rwX -- "$WORK" 2>/dev/null || true
  if ! rm -rf -- "$WORK"; then
    echo "Could not remove temporary build directory: $WORK" >&2
  fi
  return "$build_status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
echo 'Building inside a disposable container. First build needs internet and several GB of disk.'
network_args=()
if [[ -n "${MEDIABRIDGE_FLATPAK_NETWORK:-}" ]]; then
  network_args=(--network "$MEDIABRIDGE_FLATPAK_NETWORK")
  echo "Build container network: $MEDIABRIDGE_FLATPAK_NETWORK (installed app permissions are unchanged)"
fi
CONTAINER="$(podman run --detach --pull=missing "${network_args[@]}" "$IMAGE" sleep infinity)"
podman image inspect "$IMAGE" --format '{{.Id}} {{.Digest}}' > "$WORK/image.txt"
# Copy only source, never local settings, credentials, browser profiles or build outputs.
tar -C "$ROOT_DIR" -cf "$WORK/source.tar" \
  Cargo.toml Cargo.lock main.rs tests.rs dial.rs youtube_receiver.rs steam.rs steam_linux.rs steam_linux.py steam_windows.rs \
  web assets packaging/flatpak
podman cp "$WORK/source.tar" "$CONTAINER:/tmp/source.tar"
podman exec "$CONTAINER" bash -c 'mkdir /src; tar -xf /tmp/source.tar -C /src; rm /tmp/source.tar; bash /src/packaging/flatpak/build-runtime.sh'
# build-export checks for files/ even when --runtime exports the usr/ tree.
mkdir -p "$WORK/app/files" "$WORK/runtime/files" "$WORK/runtime/usr/etc"
copy_container_tree() {
  # Do not import container/subordinate UIDs or read-only package directory modes.
  # All extracted files belong to the desktop user, without requiring sudo.
  podman exec "$CONTAINER" tar -C "$1" -cf - . |
    tar --no-same-owner --no-same-permissions -xf - -C "$2"
  chmod -R u+rwX -- "$2"
}
copy_container_tree /out/app "$WORK/app/files"
copy_container_tree /usr "$WORK/runtime/usr"
# Flatpak constructs /etc from the runtime's /usr/etc, plus host integration files.
copy_container_tree /etc "$WORK/runtime/usr/etc"
# Never distribute the build container's identity, account database, or resolver config.
rm -f "$WORK/runtime/usr/etc/machine-id" "$WORK/runtime/usr/etc/hostname" \
  "$WORK/runtime/usr/etc/resolv.conf" "$WORK/runtime/usr/etc/shadow" \
  "$WORK/runtime/usr/etc/gshadow"
find "$WORK/runtime/usr" -type f -perm /6000 -exec chmod a-s {} +
install -Dm644 "$WORK/image.txt" "$WORK/app/files/share/mediabridge/build-image.txt"
cat > "$WORK/runtime/metadata" <<EOF
[Runtime]
name=$PLATFORM_ID
runtime=$PLATFORM_ID/x86_64/$BRANCH
sdk=$PLATFORM_ID/x86_64/$BRANCH

[Extension org.freedesktop.Platform.GL]
directory=lib64/GL
version=1.4
subdirectories=true
no-autodownload=true
autodelete=false
add-ld-path=lib
merge-dirs=glvnd/egl_vendor.d;vulkan/icd.d
download-if=active-gl-driver
enable-if=active-gl-driver
autoprune-unless=active-gl-driver
EOF
# NVIDIA extensions are mounted here by Flatpak; Mesa remains in /usr/lib64.
mkdir -p "$WORK/runtime/usr/lib64/GL"
cat > "$WORK/app/metadata" <<EOF
[Application]
name=$APP_ID
runtime=$PLATFORM_ID/x86_64/$BRANCH
command=mediabridge-flatpak

[Context]
shared=network;ipc;
sockets=wayland;x11;pulseaudio;
devices=dri;
filesystems=xdg-run/pipewire-0;

[Session Bus Policy]
org.mpris.MediaPlayer2.*=talk
org.mpris.MediaPlayer2.firefox.*=own
org.mpris.MediaPlayer2.librewolf.*=own
org.freedesktop.Notifications=talk
org.freedesktop.secrets=talk

[Environment]
PATH=/app/bin:/usr/bin
MEDIABRIDGE_LIBREWOLF_LAUNCHER=/app/bin/mediabridge-librewolf
MEDIABRIDGE_MOONLIGHT=/app/bin/mediabridge-moonlight
MEDIABRIDGE_LOG_STDERR=1
LIBVA_DRIVERS_PATH=/usr/lib64/dri-freeworld:/usr/lib64/dri-nonfree:/usr/lib64/dri
__EGL_VENDOR_LIBRARY_DIRS=/usr/lib64/GL/glvnd/egl_vendor.d:/usr/share/glvnd/egl_vendor.d
XDG_DATA_DIRS=/app/share:/usr/share:/usr/lib64/GL
EOF
flatpak build-finish "$WORK/app"
flatpak build-export --runtime "$OUT/repo" "$WORK/runtime" "$BRANCH"
flatpak build-export "$OUT/repo" "$WORK/app" "$BRANCH"
flatpak build-bundle --runtime "$OUT/repo" "$OUT/MediaBridge-Platform.flatpak" "$PLATFORM_ID" "$BRANCH"
flatpak build-bundle "$OUT/repo" "$OUT/MediaBridge.flatpak" "$APP_ID" "$BRANCH"
cp "$ROOT_DIR/packaging/flatpak/install-flatpak.sh" "$OUT/install-flatpak.sh"
cp "$ROOT_DIR/packaging/flatpak/start-flatpak.sh" "$OUT/start-flatpak.sh"
chmod +x "$OUT/install-flatpak.sh" "$OUT/start-flatpak.sh"
echo "Built $OUT/MediaBridge.flatpak and MediaBridge-Platform.flatpak"
echo 'Copy BOTH bundles, install-flatpak.sh and start-flatpak.sh to the test computer.'
echo "Build log: $OUT/build.log"
echo "Install: bash \"$OUT/install-flatpak.sh\""
if [[ "$INSTALL" == 1 ]]; then bash "$OUT/install-flatpak.sh"; fi

