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
if ! grep -Fq '// MediaBridge browser chrome' "$PROFILE/user.js"; then
  cat >> "$PROFILE/user.js" <<'PREFS'

// MediaBridge browser chrome
user_pref("dom.webnotifications.enabled", false);
user_pref("permissions.default.desktop-notification", 2);
user_pref("browser.toolbars.bookmarks.visibility", "never");
user_pref("toolkit.legacyUserProfileCustomizations.stylesheets", true);
PREFS
fi
if ! grep -Fq '// MediaBridge fresh startup' "$PROFILE/user.js"; then
  cat >> "$PROFILE/user.js" <<'PREFS'

// MediaBridge fresh startup
user_pref("browser.startup.page", 1);
user_pref("browser.sessionstore.resume_from_crash", false);
PREFS
fi
mkdir -p "$PROFILE/chrome"
if ! grep -Fqs 'MediaBridge navigation bar' "$PROFILE/chrome/userChrome.css"; then
  printf '\n/* MediaBridge navigation bar */\n#main-window:not([customizing]) #nav-bar:not(:focus-within) { max-height: 0 !important; min-height: 0 !important; overflow: hidden !important; }\n' >> "$PROFILE/chrome/userChrome.css"
fi
# BiDi is required by MediaBridge's managed browser playback controls.
if [[ "${MEDIABRIDGE_LIBREWOLF_NO_BIDI:-0}" == 1 ]]; then
  exec /app/lib/librewolf/AppRun --profile "$PROFILE" "$@"
fi
# The explicit private profile isolates this browser. Allow subsequent dashboard
# and playback launches to reuse it, as the native launcher does.
exec /app/lib/librewolf/AppRun --profile "$PROFILE" --remote-debugging-port 9222 "$@"
