#!/usr/bin/env bash
#
# build/release-common.sh — shared helpers for the release scripts.
#
# This file is meant to be sourced (by build/release.sh), not run directly.
# It defines the target triple, common paths, logging helpers, and the
# host/toolchain preflight checks used before a release build.

# Apple Silicon is the only supported target for now.
TARGET_TRIPLE="aarch64-apple-darwin"

# Repo root, resolved relative to this file so the scripts work from anywhere.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Cargo places workspace build output at <root>/target (see .gitignore).
BUNDLE_DIR="$ROOT/target/$TARGET_TRIPLE/release/bundle"

log()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# Source of truth: the workspace.package version in the root Cargo.toml.
release_version() {
  sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1
}

# Fail early unless we're on an arm64 macOS host with the Rust target ready.
require_arm64_macos() {
  [ "$(uname -s)" = "Darwin" ] || die "release builds are macOS-only (host is $(uname -s))"
  [ "$(uname -m)" = "arm64" ]  || die "expected an arm64 (Apple Silicon) host, got $(uname -m)"
  if ! rustup target list --installed 2>/dev/null | grep -qx "$TARGET_TRIPLE"; then
    die "Rust target $TARGET_TRIPLE is not installed (run: rustup target add $TARGET_TRIPLE)"
  fi
}
