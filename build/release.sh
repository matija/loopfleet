#!/usr/bin/env bash
#
# build/release.sh — build, notarize, and publish a macOS release for
# Apple Silicon (arm64).
#
# Builds the .app/.dmg/updater-tarball for aarch64-apple-darwin via
# `tauri build` (which also runs the frontend build, see
# beforeBuildCommand in src-tauri/tauri.conf.json), notarizes and staples
# the .dmg, writes latest.json for the Tauri updater, and creates (or
# replaces) the GitHub release at RELEASE_TAG with all three artifacts.
#
# When given a version, it bumps every manifest, commits that bump, and pushes
# it before building, so the published release points at a commit that exists
# on the remote and no manual `git commit` sits between this script and
# build/release-intel.sh.
#
# Usage:
#   build/release.sh               # build the current version
#   build/release.sh <version>     # bump every manifest to <version>, commit, push, then build
#   (to bump without releasing, run build/bump.sh <version> on its own)
#   build/release.sh --preflight   # validate the environment and exit, without building
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

# Capture any explicit override before release-common.sh defaults it to the
# (pre-bump) current version.
_release_tag_override="${RELEASE_TAG:-}"

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-common.sh"

if [ $# -gt 1 ]; then
  echo "Usage: ${0##*/} [version] [--preflight]" >&2
  exit 1
fi

# Optional version bump before building, so every manifest stays in sync. The
# bump is committed and pushed straight away: the tree was clean when
# release-common.sh checked it, so the commit contains the bump and nothing
# else, and the release below can target the commit it was built from.
if [ $# -eq 1 ]; then
  BUMP_FROM_RELEASE=1 "$ROOT/build/bump.sh" "$1"
  RELEASE_VERSION="$(release_version)"
  RELEASE_TAG="${_release_tag_override:-$RELEASE_VERSION}"
  commit_version_bump "$RELEASE_VERSION"
fi

require_macos_target "$TARGET_TRIPLE"

VERSION="$RELEASE_VERSION"
banner "Release loopfleet $VERSION ($RELEASE_TAG) for $TARGET_TRIPLE"

cd "$ROOT"
npm run tauri build -- --target "$TARGET_TRIPLE"

DMG="$(find "$BUNDLE_DIR/dmg" -maxdepth 1 -name '*.dmg' -print -quit)"
[[ -n "$DMG" ]] || { err "no .dmg found under $BUNDLE_DIR/dmg"; exit 1; }

TARBALL="$(find "$BUNDLE_DIR/macos" -maxdepth 1 -name '*.app.tar.gz' -print -quit)"
[[ -n "$TARBALL" ]] || { err "no updater tarball found under $BUNDLE_DIR/macos"; exit 1; }

SIGNATURE_FILE="$TARBALL.sig"
[[ -f "$SIGNATURE_FILE" ]] || { err "no updater signature found at $SIGNATURE_FILE"; exit 1; }

notarize_dmg "$DMG"

step "Resolving previous release for changelog..."
PREV_TAG="$(gh release list --repo "$GH_REPO" --limit 50 --json tagName \
  --jq "[.[] | select(.tagName != \"$RELEASE_TAG\")][0].tagName" 2>/dev/null || true)"

if [[ -n "$PREV_TAG" ]]; then
  NOTES="**Full Changelog**: https://github.com/$GH_REPO/compare/$PREV_TAG...$RELEASE_TAG"
  info "Previous release: $PREV_TAG"
else
  NOTES="**Full Changelog**: https://github.com/$GH_REPO/commits/$RELEASE_TAG"
  info "No previous release found; this is the first one."
fi

step "Writing latest.json..."
LATEST_JSON="$BUNDLE_DIR/latest.json"
PUB_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
SIGNATURE="$(<"$SIGNATURE_FILE")"
TARBALL_URL="https://github.com/$GH_REPO/releases/download/$RELEASE_TAG/$(basename "$TARBALL")"

jq -n \
  --arg version "$VERSION" \
  --arg notes "$NOTES" \
  --arg pub_date "$PUB_DATE" \
  --arg signature "$SIGNATURE" \
  --arg url "$TARBALL_URL" \
  '{
    version: $version,
    notes: $notes,
    pub_date: $pub_date,
    platforms: {
      "darwin-aarch64": { signature: $signature, url: $url }
    }
  }' > "$LATEST_JSON"
ok "Wrote $LATEST_JSON"

if gh release view "$RELEASE_TAG" --repo "$GH_REPO" >/dev/null 2>&1; then
  step "Release $RELEASE_TAG already exists; replacing it..."
  gh release delete "$RELEASE_TAG" --repo "$GH_REPO" --yes --cleanup-tag
fi

step "Creating GitHub release $RELEASE_TAG..."
gh release create "$RELEASE_TAG" \
  --repo "$GH_REPO" \
  --title "$RELEASE_TAG" \
  --notes "$NOTES" \
  --target "$(git rev-parse HEAD)" \
  "$DMG" "$TARBALL" "$LATEST_JSON"
ok "Published https://github.com/$GH_REPO/releases/tag/$RELEASE_TAG"

banner "Release complete"
info "Artifacts:"
info "  $DMG"
info "  $TARBALL"
info "  $LATEST_JSON"
