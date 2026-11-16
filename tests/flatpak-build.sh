#!/usr/bin/env bash
set -euo pipefail
# Exercise the complete export/install flow without containers or package downloads.
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
for script in "$ROOT/build-flatpak.sh" "$ROOT"/packaging/flatpak/*.sh; do bash -n "$script"; done
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/source with spaces" "$fixture/bin"
cp "$ROOT/build-flatpak.sh" "$fixture/source with spaces/"
cp -r "$ROOT/packaging" "$fixture/source with spaces/"
cp -r "$ROOT/web" "$fixture/source with spaces/"
cp -r "$ROOT/assets" "$fixture/source with spaces/"
for file in Cargo.toml Cargo.lock build.rs dashboard.rs main.rs recommendations.rs tests.rs dial.rs youtube_receiver.rs steam.rs steam_linux.rs steam_linux.py steam_windows.rs; do
  cp "$ROOT/$file" "$fixture/source with spaces/"
done
mkdir -p "$fixture/source with spaces/dashboard-preview/svelte"
cp -r "$ROOT"/dashboard-preview/svelte/{package.json,package-lock.json,svelte.config.js,vite.config.ts,tsconfig.json,src,static,obscura} "$fixture/source with spaces/dashboard-preview/svelte/"
export FLATPAK_TEST_LOG="$fixture/calls"
export XDG_STATE_HOME="$fixture/state"
cat > "$fixture/bin/setsid" <<'EOF'
#!/usr/bin/env bash
set -eu
[[ "$1" == --fork ]]
printf 'setsid %s\n' "$*" >> "$FLATPAK_TEST_LOG"
shift
exec "$@"
EOF
cat > "$fixture/bin/podman" <<'EOF'
#!/usr/bin/env bash
set -eu
printf 'podman %s\n' "$*" >> "$FLATPAK_TEST_LOG"
if [[ "$1" == --storage-driver ]]; then
  [[ "$2" == vfs && "$3" == --root && "$5" == --runroot ]]
  shift 6
fi
case "$1" in
  exec)
    if [[ "${FLATPAK_TEST_FAIL:-0}" == 1 ]]; then exit 17; fi
    if [[ "$3" == tar ]]; then
      tree="$(mktemp -d)"
      case "$5" in
        /out/app) mkdir -p "$tree/share/applications"; echo fixture > "$tree/share/applications/io.github.murasaki_dev.MediaBridge.desktop" ;;
        /usr) mkdir -p "$tree/bin"; echo fixture > "$tree/bin/sh" ;;
        /etc) touch "$tree/machine-id" "$tree/shadow" ;;
        *) exit 2 ;;
      esac
      tar --mode=a-w -C "$tree" -cf - .
      rm -rf -- "$tree"
    fi ;;
  run) echo fixture-container ;;
  image) echo 'sha256:fixture sha256:fixture' ;;
  cp)
    case "$2" in
      *:/out/app/.) mkdir -p "$3/share/applications"; echo fixture > "$3/share/applications/io.github.murasaki_dev.MediaBridge.desktop" ;;
      *:/usr/.) mkdir -p "$3/bin"; echo fixture > "$3/bin/sh" ;;
      *:/etc/.) mkdir -p "$3"; touch "$3/machine-id" "$3/shadow" ;;
    esac ;;
esac
EOF
cat > "$fixture/bin/flatpak" <<'EOF'
#!/usr/bin/env bash
set -eu
printf 'flatpak %s\n' "$*" >> "$FLATPAK_TEST_LOG"
case "$1" in
  --gl-drivers) printf '%s\n' "${FLATPAK_TEST_GL_DRIVERS:-default}" ;;
  kill) exit 1 ;; # First install: no running instance must not prevent installation.
  ps)
    if [[ "${FLATPAK_TEST_STILL_RUNNING:-0}" == 1 ]]; then
      echo io.github.murasaki_dev.MediaBridge
    fi ;;
  build-finish)
    test -f "$2/metadata"
    test -f "$2/files/share/applications/io.github.murasaki_dev.MediaBridge.desktop"
    ! grep -q org.freedesktop.Flatpak "$2/metadata"
    grep -Fqx 'org.mpris.MediaPlayer2.firefox.*=own' "$2/metadata"
    grep -Fqx 'org.mpris.MediaPlayer2.librewolf.*=own' "$2/metadata"
    ;;
  build-export)
    if [[ "$2" == --runtime ]]; then
      test -d "$4/files"
      test -f "$4/metadata"
      grep -q '^\[Extension org.freedesktop.Platform.GL\]' "$4/metadata"
      grep -q '^version=1.4$' "$4/metadata"
      test -d "$4/usr/lib64/GL"
      test -d "$4/usr/bin"
      test -w "$4/usr/bin"
      test "$(stat -c %u "$4/usr/bin")" == "$(id -u)"
      test ! -e "$4/usr/etc/shadow"
      test ! -e "$4/usr/etc/machine-id"
    fi ;;
  build-bundle)
    shift
    if [[ "$1" == --runtime ]]; then shift; fi
    touch "$2" ;;
  install)
    grep -q '^flatpak kill io.github.murasaki_dev.MediaBridge$' "$FLATPAK_TEST_LOG"
    grep -q '^flatpak ps --columns=application$' "$FLATPAK_TEST_LOG"
    [[ " $* " == *' --noninteractive '* ]]
    [[ " $* " == *' --assumeyes '* ]]
    if [[ " $* " == *' --bundle '* ]]; then
      test -f "${@: -1}"
      [[ " $* " == *' --reinstall '* ]] || { echo "Bundle already installed" >&2; exit 1; }
    else
      [[ "${@: -1}" == org.freedesktop.Platform.GL.nvidia-*//1.4 ]]
    fi ;;
  run)
    test "$(grep -c '^flatpak install' "$FLATPAK_TEST_LOG")" -ge 2
    test ! -t 0
    ;;
esac
EOF
chmod +x "$fixture/bin/"*
PATH="$fixture/bin:$PATH" bash "$fixture/source with spaces/build-flatpak.sh" --install
test -f "$fixture/source with spaces/dist/flatpak/MediaBridge.flatpak"
test -f "$fixture/source with spaces/dist/flatpak/MediaBridge-Platform.flatpak"
test -f "$fixture/source with spaces/dist/flatpak/start-flatpak.sh"
test -f "$fixture/source with spaces/dist/flatpak/install-flatpak.sh"
test "$(grep -c '^flatpak install' "$FLATPAK_TEST_LOG")" == 2
test "$(grep -c '^setsid --fork flatpak run io.github.murasaki_dev.MediaBridge' "$FLATPAK_TEST_LOG")" == 1
test "$(grep -c '^flatpak run io.github.murasaki_dev.MediaBridge' "$FLATPAK_TEST_LOG")" == 1
! grep -q '^flatpak remote-add' "$FLATPAK_TEST_LOG"
test "$(grep -c '^podman rm --force fixture-container' "$FLATPAK_TEST_LOG")" == 1
export FLATPAK_TEST_FAIL=1
if PATH="$fixture/bin:$PATH" bash "$fixture/source with spaces/build-flatpak.sh"; then
  echo 'Build failure was unexpectedly ignored' >&2; exit 1
else
  test "$?" == 17
fi
test "$(grep -c '^podman rm --force fixture-container' "$FLATPAK_TEST_LOG")" == 2
test "$(grep -c '^flatpak build-export' "$FLATPAK_TEST_LOG")" == 2
test -z "$(find "$fixture/source with spaces/dist/flatpak" -maxdepth 1 -name 'work.*' -print -quit)"
export FLATPAK_TEST_GL_DRIVERS='default;nvidia-580-126-18'
PATH="$fixture/bin:$PATH" bash "$fixture/source with spaces/dist/flatpak/install-flatpak.sh"
grep -q '^flatpak remote-add --user --if-not-exists flathub ' "$FLATPAK_TEST_LOG"
grep -q 'org.freedesktop.Platform.GL.nvidia-580-126-18//1.4' "$FLATPAK_TEST_LOG"
installs_before="$(grep -c '^flatpak install' "$FLATPAK_TEST_LOG")"
if FLATPAK_TEST_STILL_RUNNING=1 PATH="$fixture/bin:$PATH" bash "$fixture/source with spaces/dist/flatpak/install-flatpak.sh"; then
  echo 'Installer continued with an old instance still running' >&2; exit 1
fi
test "$(grep -c '^flatpak install' "$FLATPAK_TEST_LOG")" == "$installs_before"
FLATPAK_TEST_FAIL=0 MEDIABRIDGE_FLATPAK_STORAGE=vfs MEDIABRIDGE_FLATPAK_OUTPUT="$fixture/build output" \
  PATH="$fixture/bin:$PATH" bash "$fixture/source with spaces/build-flatpak.sh"
grep -q '^podman --storage-driver vfs .* rm --force fixture-container$' "$FLATPAK_TEST_LOG"
test -f "$fixture/build output/MediaBridge.flatpak"
test -f "$fixture/build output/MediaBridge-Platform.flatpak"
test -f "$fixture/build output/install-flatpak.sh"
test -f "$fixture/build output/start-flatpak.sh"
upstream="$fixture/upstream"
git init -q -b main "$upstream"
git -C "$upstream" config user.email fixture@example.invalid
git -C "$upstream" config user.name Fixture
printf 'echo first\n' > "$upstream/build-flatpak.sh"
git -C "$upstream" add build-flatpak.sh
git -C "$upstream" commit -qm first
export MEDIABRIDGE_GITEA_REPO="$upstream"
export MEDIABRIDGE_FLATPAK_SOURCE="$fixture/git checkout"
launcher="$ROOT/build-flatpak-from-gitea.sh"
bash "$launcher" | grep -Fxq first
printf 'echo "second $*"; test "$MEDIABRIDGE_FLATPAK_OUTPUT" = "$PWD/dist/flatpak"\n' > "$upstream/build-flatpak.sh"
git -C "$upstream" commit -qam second
bash "$launcher" --install | grep -Fxq 'second --install'
printf '# local edit\n' >> "$MEDIABRIDGE_FLATPAK_SOURCE/build-flatpak.sh"
if bash "$launcher"; then
  echo 'Gitea launcher accepted a dirty checkout' >&2; exit 1
fi
echo 'Flatpak build orchestration: bundles, runtime-first install, spaces, default permissions and cleanup passed.'
