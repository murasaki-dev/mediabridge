#!/usr/bin/env bash
set -euo pipefail
export HOME="$XDG_DATA_HOME/librewolf-home"
PROFILE="$XDG_DATA_HOME/librewolf-profile"
mkdir -p "$HOME/.librewolf" "$PROFILE"
if [[ ! -f "$HOME/.librewolf/librewolf.overrides.cfg" ]]; then
  cat > "$HOME/.librewolf/librewolf.overrides.cfg" <<'EOF'
pref("privacy.clearOnShutdown.cookies", false);
pref("privacy.clearOnShutdown_v2.cookiesAndStorage", false);
pref("privacy.sanitize.sanitizeOnShutdown", false);
pref("media.autoplay.default", 0);
pref("media.autoplay.blocking_policy", 0);
pref("devtools.debugger.remote-enabled", true);
EOF
fi
if [[ ! -f "$PROFILE/user.js" ]]; then
  cat > "$PROFILE/user.js" <<'EOF'
user_pref("privacy.clearOnShutdown.cookies", false);
user_pref("privacy.clearOnShutdown_v2.cookiesAndStorage", false);
user_pref("privacy.sanitize.sanitizeOnShutdown", false);
user_pref("media.autoplay.default", 0);
user_pref("media.autoplay.blocking_policy", 0);
user_pref("devtools.debugger.remote-enabled", true);
user_pref("browser.shell.checkDefaultBrowser", false);
EOF
fi
# BiDi is required by MediaBridge's managed browser playback controls.
if [[ "${MEDIABRIDGE_LIBREWOLF_NO_BIDI:-0}" == 1 ]]; then
  exec /app/lib/librewolf/AppRun --no-remote --profile "$PROFILE" "$@"
fi
exec /app/lib/librewolf/AppRun --no-remote --profile "$PROFILE" --remote-debugging-port 9222 "$@"
