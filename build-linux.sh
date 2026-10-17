#!/usr/bin/env bash
set -euo pipefail

# Developer-only builder. The release artifact is a portable Linux tarball,
# not a package-manager-specific installer.
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

if [[ "$(uname -s)" != Linux ]]; then
  echo "Run this builder on Linux. It produces a native x86_64 Linux executable." >&2
  exit 1
fi
if [[ "$(uname -m)" != x86_64 ]]; then
  echo "This release builder currently targets x86_64 Linux (found $(uname -m))." >&2
  exit 1
fi
for tool in cargo pkg-config; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Missing '$tool'. Install Rust/Cargo and pkg-config, then rerun this script." >&2
    exit 1
  fi
done
if ! pkg-config --exists gtk+-3.0 appindicator3-0.1; then
  echo "Linux tray support needs GTK 3 and AppIndicator development libraries (gtk+-3.0 and appindicator3-0.1 pkg-config files)." >&2
  echo "Install the matching development packages for your distribution, then rerun ./build-linux.sh." >&2
  exit 1
fi

cargo build --release
mkdir -p dist
tar -C target/release -czf dist/mediabridge-linux-x86_64.tar.gz mediabridge
echo "Built $ROOT_DIR/dist/mediabridge-linux-x86_64.tar.gz"
