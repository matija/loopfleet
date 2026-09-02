#!/usr/bin/env bash
#
# build/bump.sh — write the same version across every manifest in the repo.
#
# The version is defined in several places that must stay in sync:
#   - Cargo.toml            (workspace.package; all crates inherit it)
#   - Cargo.lock            (the `loopfleet` package entry)
#   - package.json          (root)
#   - frontend/package.json
#   - src-tauri/tauri.conf.json
#
# It also rewrites the download links in README.md (the block between the
# `download-links` markers), so the advertised .dmg URLs always point at the
# release being cut.
#
# Usage:
#   build/bump.sh <version>   # set every manifest to <version>
#   build/bump.sh             # print the current version and exit
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Source of truth: the workspace.package version in the root Cargo.toml.
current_version() {
  sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1
}

usage() {
  echo "Usage: ${0##*/} <version>   # set every manifest to <version>" >&2
  echo "       ${0##*/}             # print the current version" >&2
}

if [ $# -eq 0 ]; then
  current_version
  exit 0
fi

NEW="$1"

# Validate semver: X.Y.Z with an optional -prerelease and/or +build suffix.
if ! printf '%s' "$NEW" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)*$'; then
  echo "error: '$NEW' is not a valid semver version (expected X.Y.Z)" >&2
  usage
  exit 1
fi

# Portable in-place edit (works with both BSD and GNU sed).
sed_i() {
  sed -i.bak "$1" "$2" && rm -f "$2.bak"
}

# Cargo workspace version — the only line starting with `version = `.
# Every crate uses `version.workspace = true`, so this is enough.
sed_i "s/^version = \".*\"/version = \"$NEW\"/" "$ROOT/Cargo.toml"

# Cargo.lock — bump only the `version` line inside the `[[package]]` block
# for the `loopfleet` crate itself, leaving every other locked dependency
# version (including other `loopfleet-*` crates) untouched.
awk -v new="$NEW" '
  /^name = "loopfleet"$/ { in_pkg = 1 }
  in_pkg && /^version = / {
    print "version = \"" new "\""
    in_pkg = 0
    next
  }
  { print }
' "$ROOT/Cargo.lock" > "$ROOT/Cargo.lock.tmp" && mv "$ROOT/Cargo.lock.tmp" "$ROOT/Cargo.lock"

# JSON manifests — each has exactly one `"version": "..."` field.
for json in \
  "$ROOT/package.json" \
  "$ROOT/frontend/package.json" \
  "$ROOT/src-tauri/tauri.conf.json"; do
  sed_i "s/\"version\": \".*\"/\"version\": \"$NEW\"/" "$json"
done

# README download links — regenerate the whole block between the markers, so
# the URLs, the file names, and the version in the label all stay consistent.
# The repo is the same default release.sh uses; override with GH_REPO.
README_REPO="${GH_REPO:-matija/loopfleet}"
awk -v new="$NEW" -v repo="$README_REPO" '
  /<!-- download-links:start -->/ {
    base = "https://github.com/" repo "/releases/download/" new
    print
    print "**Download loopfleet " new "** —"
    print "[Apple Silicon](" base "/loopfleet_" new "_aarch64.dmg)"
    print "· [Intel](" base "/loopfleet_" new "_x64.dmg)"
    skip = 1
    next
  }
  /<!-- download-links:end -->/ { skip = 0 }
  !skip { print }
' "$ROOT/README.md" > "$ROOT/README.md.tmp" && mv "$ROOT/README.md.tmp" "$ROOT/README.md"

if ! grep -q '<!-- download-links:start -->' "$ROOT/README.md"; then
  echo "warning: README.md has no <!-- download-links:start --> marker; download links left untouched" >&2
fi

echo "Bumped version to $NEW"
echo
git -C "$ROOT" --no-pager diff -- Cargo.toml Cargo.lock package.json frontend/package.json src-tauri/tauri.conf.json README.md
echo
echo "Next step: review the diff above, then commit the bump (e.g. \`git add -u && git commit -m \"Bump version to $NEW\"\`)."
