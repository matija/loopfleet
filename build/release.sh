#!/usr/bin/env bash
#
# build/release.sh — build a macOS release bundle for Apple Silicon (arm64).
#
# Produces the .app and .dmg for aarch64-apple-darwin via `tauri build`,
# which also runs the frontend build (see beforeBuildCommand in
# src-tauri/tauri.conf.json).
#
# Usage:
#   build/release.sh             # build the current version
#   build/release.sh <version>   # bump every manifest to <version>, then build
#
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-common.sh"

if [ $# -gt 1 ]; then
  echo "Usage: ${0##*/} [version]" >&2
  exit 1
fi

# Optional version bump before building, so every manifest stays in sync.
if [ $# -eq 1 ]; then
  "$ROOT/build/bump.sh" "$1"
fi

require_arm64_macos

VERSION="$(release_version)"
log "Building loopfleet $VERSION for $TARGET_TRIPLE"

cd "$ROOT"
npm run tauri build -- --target "$TARGET_TRIPLE"

log "Release artifacts:"
if [ -d "$BUNDLE_DIR" ]; then
  find "$BUNDLE_DIR" -maxdepth 2 \( -name '*.app' -o -name '*.dmg' \) -print
else
  warn "no bundle directory at $BUNDLE_DIR"
fi
