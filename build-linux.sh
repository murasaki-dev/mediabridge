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
if ! command -v cargo >/dev/null 2>&1; then
  echo "Missing Rust/Cargo. Install the current Rust toolchain with rustup, then rerun this script." >&2
  exit 1
fi

# tray-icon links against GTK 3 and AppIndicator. Package names differ by
# distro; install the native development packages when pkg-config cannot find
# them. Rust/Cargo remain user-managed so the build uses the user's toolchain.
if ! command -v pkg-config >/dev/null 2>&1 || ! pkg-config --exists gtk+-3.0 appindicator3-0.1; then
  if [[ ! -r /etc/os-release ]]; then
    echo "GTK 3/AppIndicator build dependencies are missing and this distribution cannot be identified." >&2
    echo "Install pkg-config, GTK 3 development headers, and AppIndicator development headers, then rerun ./build-linux.sh." >&2
    exit 1
  fi
  # shellcheck disable=SC1091
  . /etc/os-release
  PACKAGE_MANAGER=""
  for candidate in apt-get dnf pacman zypper xbps-install; do
    if command -v "$candidate" >/dev/null 2>&1; then PACKAGE_MANAGER="$candidate"; break; fi
  done
  case "$PACKAGE_MANAGER" in
    apt-get) BUILD_PACKAGES=(pkg-config libgtk-3-dev libappindicator3-dev) ;;
    dnf) BUILD_PACKAGES=(pkgconf-pkg-config gtk3-devel libappindicator-gtk3-devel) ;;
    pacman) BUILD_PACKAGES=(pkgconf gtk3 libappindicator-gtk3) ;;
    zypper) BUILD_PACKAGES=(pkg-config gtk3-devel libappindicator3-devel) ;;
    xbps-install) BUILD_PACKAGES=(pkg-config gtk+3-devel libappindicator-devel) ;;
    *)
      echo "GTK 3/AppIndicator build dependencies are missing. No supported package manager was found for ${ID:-this distribution}." >&2
      echo "Install pkg-config, GTK 3 development headers, and AppIndicator development headers, then rerun ./build-linux.sh." >&2
      exit 1
      ;;
  esac

  echo "MediaBridge needs GTK 3 and AppIndicator development packages to build Linux tray support."
  echo "The script will install: ${BUILD_PACKAGES[*]} (using $PACKAGE_MANAGER)."
  if [[ "${MEDIABRIDGE_INSTALL_BUILD_DEPS:-}" != "1" ]]; then
    read -r -p "Continue with system package installation? [y/N] " answer
    case "$answer" in y|Y|yes|YES) ;; *) echo "Dependency installation cancelled." >&2; exit 1 ;; esac
  fi
  if [[ "$EUID" -eq 0 ]]; then
    echo "Run ./build-linux.sh as a regular user; it will use sudo for system packages." >&2
    exit 1
  fi
  command -v sudo >/dev/null 2>&1 || { echo "sudo is required to install Linux build dependencies." >&2; exit 1; }
  case "$PACKAGE_MANAGER" in
    apt-get) sudo apt-get update && sudo apt-get install -y "${BUILD_PACKAGES[@]}" ;;
    dnf) sudo dnf install -y "${BUILD_PACKAGES[@]}" ;;
    pacman) sudo pacman -S --needed --noconfirm "${BUILD_PACKAGES[@]}" ;;
    zypper) sudo zypper --non-interactive install "${BUILD_PACKAGES[@]}" ;;
    xbps-install) sudo xbps-install -Sy "${BUILD_PACKAGES[@]}" ;;
  esac
fi

if ! pkg-config --exists gtk+-3.0 appindicator3-0.1; then
  echo "The package installation completed, but pkg-config still cannot find gtk+-3.0 and appindicator3-0.1." >&2
  echo "Check that your distribution provides these development files, then rerun ./build-linux.sh." >&2
  exit 1
fi

cargo build --release --locked
mkdir -p dist
tar -C target/release -czf dist/mediabridge-linux-x86_64.tar.gz mediabridge
echo "Built $ROOT_DIR/dist/mediabridge-linux-x86_64.tar.gz"
