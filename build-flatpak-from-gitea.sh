#!/usr/bin/env bash
set -euo pipefail

repo="${MEDIABRIDGE_GITEA_REPO:-http://192.168.0.199:3000/homelab/mediabridge.git}"
checkout="${MEDIABRIDGE_FLATPAK_SOURCE:-$PWD/mediabridge-flatpak-source}"
export MEDIABRIDGE_FLATPAK_OUTPUT="${MEDIABRIDGE_FLATPAK_OUTPUT:-$PWD/dist/flatpak}"

if [[ "${1:-}" == --help || "${1:-}" == -h ]]; then
  echo 'Usage: bash build-flatpak-from-gitea.sh [--install]'
  echo "Clones or updates Gitea main in $checkout."
  echo "Writes both bundles and installer scripts to $MEDIABRIDGE_FLATPAK_OUTPUT."
  echo 'Arch prerequisites: sudo pacman -Syu --needed git podman fuse-overlayfs flatpak python tar'
  exit 0
fi
if (( $# > 1 )) || { (( $# == 1 )) && [[ "$1" != --install ]]; }; then
  echo 'Usage: bash build-flatpak-from-gitea.sh [--install]' >&2
  exit 2
fi
if (( EUID == 0 )); then
  echo 'Run the build as your normal desktop user.' >&2
  exit 1
fi
command -v git >/dev/null || { echo 'Install Git before running this script.' >&2; exit 1; }

if [[ ! -e "$checkout" ]]; then
  git -c core.autocrlf=false clone --single-branch --branch main -- "$repo" "$checkout"
else
  if [[ ! -d "$checkout/.git" ]] ||
      [[ "$(git -C "$checkout" remote get-url origin)" != "$repo" ]] ||
      [[ "$(git -C "$checkout" branch --show-current)" != main ]]; then
    echo "Expected a MediaBridge checkout of Gitea main at $checkout." >&2
    exit 1
  fi
  if [[ -n "$(git -C "$checkout" status --porcelain)" ]]; then
    echo "Local edits in $checkout. Set MEDIABRIDGE_FLATPAK_SOURCE to a new directory to build from Gitea." >&2
    exit 1
  fi
  git -C "$checkout" fetch --no-tags origin main
  if ! git -C "$checkout" merge-base --is-ancestor HEAD FETCH_HEAD; then
    echo "Local main has commits outside Gitea main. Choose a fresh checkout directory." >&2
    exit 1
  fi
  git -C "$checkout" merge --ff-only FETCH_HEAD
fi

printf 'Building Gitea commit %s\n' "$(git -C "$checkout" rev-parse HEAD)"
exec bash "$checkout/build-flatpak.sh" "$@"
