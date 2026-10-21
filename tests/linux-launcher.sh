#!/usr/bin/env bash
set -euo pipefail
# Exercise generated launchers without installing packages or running MediaBridge.
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
launcher_fixture_home="$fixture/home with spaces"
APP_DIR="$launcher_fixture_home/custom data/mediabridge"
CONFIG_DIR="$launcher_fixture_home/custom config/mediabridge"
STATE_DIR="$launcher_fixture_home/custom state/mediabridge"
BIN_DIR="$launcher_fixture_home/bin"
DISTROBOX_NAME=mediabridge-test-runtime
mkdir -p "$APP_DIR" "$BIN_DIR" "$CONFIG_DIR" "$STATE_DIR"
export PATH="$BIN_DIR:$PATH"
# Only extract the launcher-generation block, never execute the installer.
sed -n '/^# Keep deployment-specific/,/^chmod 755.*mediabridge-run/p' \
  "$ROOT_DIR/install-linux.sh" > "$fixture/generate.sh"
cat > "$BIN_DIR/distrobox" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
while [[ "$1" != -- ]]; do shift; done
shift
exec "$@"
EOF
chmod +x "$BIN_DIR/distrobox"
for DISTROBOX_RUNTIME in 0 1; do
  source "$fixture/generate.sh"
  bash -n "$BIN_DIR/mediabridge-run"
  "$BIN_DIR/mediabridge-run" bash -c '[[ "$MEDIABRIDGE_LOG_STDERR" == 1 && "$MEDIABRIDGE_MPV_HWDEC" == auto ]]'
  cat > "$CONFIG_DIR/runtime.env" <<'EOF'
MEDIABRIDGE_LOG_STDERR=0
MEDIABRIDGE_MPV_HWDEC=no
MEDIABRIDGE_SMB_CREDENTIAL_PROVIDER='/provider path/with spaces'
MEDIABRIDGE_SMB_CONFIG_DIR='/companion path/config'
MEDIABRIDGE_SMB_ROOM_PORT=9001
EOF
  before="$(cat "$CONFIG_DIR/runtime.env")"
  source "$fixture/generate.sh"
  [[ "$(cat "$CONFIG_DIR/runtime.env")" == "$before" ]]
  "$BIN_DIR/mediabridge-run" bash -c '
    [[ "$MEDIABRIDGE_LOG_STDERR" == 0 && "$MEDIABRIDGE_MPV_HWDEC" == no ]]
    [[ "$MEDIABRIDGE_SMB_CREDENTIAL_PROVIDER" == "/provider path/with spaces" ]]
    [[ "$MEDIABRIDGE_SMB_CONFIG_DIR" == "/companion path/config" ]]
    [[ "$MEDIABRIDGE_SMB_ROOM_PORT" == 9001 ]]
    [[ "$XDG_CONFIG_HOME" == "$1/custom config" ]]
    [[ "$XDG_DATA_HOME" == "$1/custom data" ]]
    [[ "$XDG_STATE_HOME" == "$1/custom state" ]]
    [[ "$MEDIABRIDGE_LIBREWOLF_PROFILE" == "$1/custom data/mediabridge/librewolf-profile" ]]
  ' test "$launcher_fixture_home"
  rm -- "$CONFIG_DIR/runtime.env"
done
echo 'Native and Distrobox launchers: defaults, overrides, paths, and reinstall preservation passed.'
