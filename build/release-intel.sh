#!/usr/bin/env bash
#
# build/release-intel.sh — build, notarize, and publish the Intel
# (x86_64-apple-darwin) bundle for an *existing* GitHub release.
#
# Unlike release.sh, this script does not create or replace the release: it
# requires RELEASE_TAG (or the current version) to already exist as a GitHub
# release, published by build/release.sh for aarch64-apple-darwin. It builds
# the Intel .app/.dmg/updater-tarball via `tauri build`, notarizes and staples
# the .dmg, copies the updater tarball to an architecture-suffixed name (so it
# doesn't collide with the aarch64 tarball already on the release), downloads
# the release's existing latest.json and adds a darwin-x86_64 entry to it, and
# uploads the Intel assets to the release with --clobber.
#
# Usage:
#   build/release-intel.sh               # build the current version
#   build/release-intel.sh --preflight   # validate the environment and exit, without building
#
set -euo pipefail

args=()
for arg in "$@"; do
  if [ "$arg" = "--preflight" ]; then
    export RELEASE_PREFLIGHT_ONLY=1
  else
    args+=("$arg")
  fi
done
if [ "${#args[@]}" -gt 0 ]; then
  set -- "${args[@]}"
else
  set --
fi

if [ $# -gt 0 ]; then
  echo "Usage: ${0##*/} [--preflight]" >&2
  exit 1
fi

export TARGET_TRIPLE="x86_64-apple-darwin"
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-common.sh"

# Cross-compiling to x86_64 still requires an Apple Silicon host: that's the
# only host that can codesign/notarize with the loopfleet signing identity.
require_arm64_macos

VERSION="$RELEASE_VERSION"
banner "Release loopfleet $VERSION ($RELEASE_TAG) for $TARGET_TRIPLE"

step "Checking that release $RELEASE_TAG already exists..."
gh release view "$RELEASE_TAG" --repo "$GH_REPO" >/dev/null 2>&1 || {
  err "release $RELEASE_TAG does not exist. Run build/release.sh first to publish the aarch64 release."
  exit 1
}
ok "Release $RELEASE_TAG exists"

cd "$ROOT"
npm run tauri build -- --target "$TARGET_TRIPLE"

DMG="$(find "$BUNDLE_DIR/dmg" -maxdepth 1 -name '*.dmg' -print -quit)"
[[ -n "$DMG" ]] || { err "no .dmg found under $BUNDLE_DIR/dmg"; exit 1; }

TARBALL="$(find "$BUNDLE_DIR/macos" -maxdepth 1 -name '*.app.tar.gz' -print -quit)"
[[ -n "$TARBALL" ]] || { err "no updater tarball found under $BUNDLE_DIR/macos"; exit 1; }

SIGNATURE_FILE="$TARBALL.sig"
[[ -f "$SIGNATURE_FILE" ]] || { err "no updater signature found at $SIGNATURE_FILE"; exit 1; }

notarize_dmg "$DMG"

step "Renaming updater tarball with architecture suffix..."
ARCH_TARBALL="$(dirname "$TARBALL")/$(basename "$TARBALL" .app.tar.gz)-x86_64.app.tar.gz"
cp "$TARBALL" "$ARCH_TARBALL"
ok "Copied to $ARCH_TARBALL"

step "Downloading existing latest.json from $RELEASE_TAG..."
LATEST_JSON="$BUNDLE_DIR/latest.json"
gh release download "$RELEASE_TAG" --repo "$GH_REPO" \
  --pattern 'latest.json' --output "$LATEST_JSON" --clobber
ok "Downloaded $LATEST_JSON"

step "Adding darwin-x86_64 entry to latest.json..."
SIGNATURE="$(<"$SIGNATURE_FILE")"
TARBALL_URL="https://github.com/$GH_REPO/releases/download/$RELEASE_TAG/$(basename "$ARCH_TARBALL")"

jq \
  --arg signature "$SIGNATURE" \
  --arg url "$TARBALL_URL" \
  '.platforms["darwin-x86_64"] = { signature: $signature, url: $url }' \
  "$LATEST_JSON" > "$LATEST_JSON.tmp"
mv "$LATEST_JSON.tmp" "$LATEST_JSON"
ok "Updated $LATEST_JSON"

step "Uploading Intel assets to $RELEASE_TAG..."
gh release upload "$RELEASE_TAG" \
  --repo "$GH_REPO" \
  --clobber \
  "$DMG" "$ARCH_TARBALL" "$LATEST_JSON"
ok "Published https://github.com/$GH_REPO/releases/tag/$RELEASE_TAG"

banner "Intel release complete"
info "Artifacts:"
info "  $DMG"
info "  $ARCH_TARBALL"
info "  $LATEST_JSON"
