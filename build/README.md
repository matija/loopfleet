# Building loopfleet

loopfleet is a [Tauri](https://tauri.app) app: a Rust workspace (`crates/*` +
`src-tauri`) with a React/Vite frontend (`frontend/`). This directory holds the
version-bump and release scripts.

## Prerequisites

- **Rust** (stable) with the Apple Silicon target for release builds:
  `rustup target add aarch64-apple-darwin`
- **Node.js** + npm (for the Tauri CLI and the frontend)
- **Tauri system deps** — see the [Tauri prerequisites](https://tauri.app/start/prerequisites/).
  On macOS that's just the Xcode command-line tools.

Install JS dependencies once (root and frontend):

```sh
npm install
npm install --prefix frontend
```

## Develop

From the repo root:

```sh
npm run tauri dev
```

This launches the Rust app and, via `beforeDevCommand`, the Vite dev server for
the frontend. Both rebuild on change.

## Release

Release builds run on an **Apple Silicon (arm64) macOS host**, which builds
and notarizes both the native `aarch64-apple-darwin` bundle and, by
cross-compiling, the `x86_64-apple-darwin` (Intel) bundle. The scripts in
this directory are self-contained:

| Script                   | What it does                                                                                    |
| ------------------------ | ------------------------------------------------------------------------------------------------ |
| `build/bump.sh`          | Set (or print) the version across every manifest.                                                |
| `build/release.sh`       | Build, notarize, and publish the `aarch64-apple-darwin` release to GitHub.                       |
| `build/release-intel.sh` | Build and notarize the `x86_64-apple-darwin` bundle, and add it to an *existing* release.         |
| `release-common.sh`      | Shared helpers (sourced by the release scripts, not run directly).                               |

### Bump the version

The version lives in four manifests that must stay in sync — the root
`Cargo.toml` (workspace, inherited by all crates), root `package.json`,
`frontend/package.json`, and `src-tauri/tauri.conf.json`. `bump.sh` writes all
of them at once:

```sh
build/bump.sh            # print the current version
build/bump.sh 0.2.0      # set every manifest to 0.2.0
```

### Build and publish a release

```sh
build/release.sh              # build + publish the current version
build/release.sh 0.2.0        # bump to 0.2.0, then build + publish
build/release.sh --preflight  # validate env vars/tooling only, no build
```

`release.sh` requires `APPLE_SIGNING_IDENTITY`, `APPLE_API_KEY_PATH`,
`APPLE_API_KEY`/`APPLE_API_KEY_ID`, `APPLE_API_ISSUER`, a Tauri updater
signing key (`TAURI_SIGNING_PRIVATE_KEY` or `_PATH`), and an authenticated
`gh` CLI. It runs on an arm64 macOS host only, and refuses to run from a
dirty or unpushed tree.

It then:

1. Runs `tauri build --target aarch64-apple-darwin`, which also builds the
   frontend (`beforeBuildCommand`). Artifacts land under
   `target/aarch64-apple-darwin/release/bundle/` — the `.app` and updater
   tarball (`.app.tar.gz` + `.sig`) under `macos/`, the `.dmg` under `dmg/`.
2. Notarizes and staples the `.dmg`.
3. Writes `latest.json` (Tauri updater manifest) with a `darwin-aarch64`
   platform entry pointing at the updater tarball's GitHub release URL,
   signed with the tarball's `.sig`.
4. Creates the GitHub release at `RELEASE_TAG` (defaulting to the version),
   replacing it if one already exists, uploading the `.dmg`, the updater
   tarball, and `latest.json`. Release notes are a compare link against the
   previous release on GitHub.

### Add the Intel build to a release

```sh
build/release-intel.sh              # build + publish the Intel bundle
build/release-intel.sh --preflight  # validate env vars/tooling only, no build
```

`release-intel.sh` shares its environment requirements with `release.sh` and
also requires `RELEASE_TAG` (defaulting to the current version) to already
exist as a GitHub release — it does not create or replace releases. It then:

1. Runs `tauri build --target x86_64-apple-darwin` (cross-compiled from the
   arm64 host).
2. Notarizes and staples the `.dmg`.
3. Copies the updater tarball to an architecture-suffixed name (e.g.
   `loopfleet-x86_64.app.tar.gz`) so it doesn't collide with the aarch64
   tarball already on the release.
4. Downloads the release's existing `latest.json` and adds a `darwin-x86_64`
   platform entry to it, rather than replacing the file.
5. Uploads the `.dmg`, the renamed tarball, and the updated `latest.json` to
   the release with `--clobber`.
