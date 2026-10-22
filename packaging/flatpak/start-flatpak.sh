#!/usr/bin/env bash
# Start the installed Flatpak independently of the invoking terminal.
set -euo pipefail
APP_ID=io.github.murasaki_dev.MediaBridge
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/mediabridge-flatpak"
mkdir -p "$STATE_DIR"
if command -v setsid >/dev/null 2>&1; then
  # A new session has no controlling terminal, so closing the shell cannot
  # hang up Flatpak's supervisor or its application processes.
  setsid --fork flatpak run "$APP_ID" "$@" </dev/null >>"$STATE_DIR/launcher.log" 2>&1
else
  nohup flatpak run "$APP_ID" "$@" </dev/null >>"$STATE_DIR/launcher.log" 2>&1 &
fi
echo "MediaBridge launch requested. Log: $STATE_DIR/launcher.log"
