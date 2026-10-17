#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
export PATH="${HOME}/.cargo/bin:${PATH}"

cargo build --release

bind="${MEDIABRIDGE_BIND:-0.0.0.0:8765}"
moonlight="${MEDIABRIDGE_MOONLIGHT:-moonlight}"
args=(--bind "$bind" --moonlight "$moonlight" --moonlight-arg stream --moonlight-arg '{host}' --moonlight-arg '{app}')
if [[ -n "${MEDIABRIDGE_COOKIES_FROM_BROWSER:-}" ]]; then
  args+=(--cookies-from-browser "$MEDIABRIDGE_COOKIES_FROM_BROWSER")
fi

args+=("$@")
exec ./target/release/mediabridge "${args[@]}"
