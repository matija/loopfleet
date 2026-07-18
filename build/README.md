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

Release builds are **macOS / Apple Silicon (arm64) only** for now. The scripts
in this directory are self-contained:

| Script               | What it does                                                            |
| -------------------- | ---------------------------------------------------------------------- |
| `build/bump.sh`      | Set (or print) the version across every manifest.                      |
| `build/release.sh`   | Build the `.app` and `.dmg` for `aarch64-apple-darwin`.                |
| `release-common.sh`  | Shared helpers (sourced by `release.sh`, not run directly).            |

### Bump the version

The version lives in four manifests that must stay in sync — the root
`Cargo.toml` (workspace, inherited by all crates), root `package.json`,
`frontend/package.json`, and `src-tauri/tauri.conf.json`. `bump.sh` writes all
of them at once:

```sh
build/bump.sh            # print the current version
build/bump.sh 0.2.0      # set every manifest to 0.2.0
```

### Build the bundle

```sh
build/release.sh          # build the current version
build/release.sh 0.2.0    # bump to 0.2.0, then build
```

`release.sh` runs its own preflight (arm64 macOS host + installed Rust target),
then `tauri build`, which also builds the frontend (`beforeBuildCommand`).
Artifacts land in:

```
target/aarch64-apple-darwin/release/bundle/
```

with the `.app` under `macos/` and the `.dmg` under `dmg/`. The script prints
the exact paths when it finishes.
