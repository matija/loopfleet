#!/usr/bin/env bash
#
# build/bump.sh — write the same version across every manifest in the repo.
#
# The version is defined in several places that must stay in sync:
#   - Cargo.toml            (workspace.package; all crates inherit it)
#   - Cargo.lock            (the `loopfleet` package entry)
#   - package.json          (root)
#   - package-lock.json     (root; the lockfile's own package entry)
#   - frontend/package.json
#   - frontend/package-lock.json
#   - node_modules/.package-lock.json (the root node_modules is checked in)
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

# Cargo.lock — bump the `version` line of every workspace member, not just
# `loopfleet`: the sibling crates all use `version.workspace = true`, so cargo
# rewrites their lock entries on the next build anyway. Bumping them here keeps
# the tree clean after the bump commit, which build/release.sh depends on.
#
# A workspace member is a `[[package]]` block with no `source = ` line (that
# field only appears on crates fetched from a registry or git), so a published
# third-party crate that happens to be named `loopfleet-*` is left alone. Each
# block is buffered until its end, since `source` can follow `version`.
awk -v new="$NEW" '
  function flush_block() {
    if (n == 0) return
    for (i = 1; i <= n; i++) {
      if (is_member && buf[i] ~ /^version = /) print "version = \"" new "\""
      else print buf[i]
    }
    n = 0; is_member = 0
  }
  /^\[\[package\]\]$/ { flush_block(); in_block = 1 }
  !in_block { print; next }
  {
    buf[++n] = $0
    if ($0 ~ /^name = "loopfleet(-[a-z-]+)?"$/) is_member = 1
    if ($0 ~ /^source = /) is_member = 0
  }
  END { flush_block() }
' "$ROOT/Cargo.lock" > "$ROOT/Cargo.lock.tmp" && mv "$ROOT/Cargo.lock.tmp" "$ROOT/Cargo.lock"

# JSON manifests — each has exactly one `"version": "..."` field.
for json in \
  "$ROOT/package.json" \
  "$ROOT/frontend/package.json" \
  "$ROOT/src-tauri/tauri.conf.json"; do
  sed_i "s/\"version\": \".*\"/\"version\": \"$NEW\"/" "$json"
done

# npm lockfiles — these carry the package's own version in two places, and npm
# rewrites them on the next `npm install`. Leaving them stale means the tree
# goes dirty after the release commit. Edited through node rather than sed
# because a lockfile has a `"version"` field for every dependency too; node
# reproduces npm's formatting byte for byte.
# node_modules/.package-lock.json is in the list because the root node_modules
# (the vendored Tauri CLI) is checked in, so npm dirties that file too.
for lock in \
  "$ROOT/package-lock.json" \
  "$ROOT/frontend/package-lock.json" \
  "$ROOT/node_modules/.package-lock.json"; do
  [ -f "$lock" ] || continue
  node -e '
    const fs = require("fs");
    const [path, version] = process.argv.slice(1);
    const lock = JSON.parse(fs.readFileSync(path, "utf8"));
    lock.version = version;
    if (lock.packages && lock.packages[""]) lock.packages[""].version = version;
    fs.writeFileSync(path, JSON.stringify(lock, null, 2) + "\n");
  ' "$lock" "$NEW"
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
git -C "$ROOT" --no-pager diff --stat -- Cargo.toml Cargo.lock package.json package-lock.json \
  frontend/package.json frontend/package-lock.json node_modules/.package-lock.json \
  src-tauri/tauri.conf.json README.md
echo
# build/release.sh commits and pushes the bump itself, so the manual hint would
# only be misleading there.
if [ -z "${BUMP_FROM_RELEASE:-}" ]; then
  echo "Next step: review the diff above, then commit the bump (e.g. \`git add -u && git commit -m \"chore(release): bump version to $NEW\"\`)."
fi
