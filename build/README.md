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
| `build/bump.sh`          | Set (or print) the version across every manifest, and refresh the README download links.         |
| `build/release.sh`       | Bump (optional), commit and push the bump, then build, notarize, and publish the `aarch64-apple-darwin` release to GitHub. |
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

It also regenerates the download links in the top-level `README.md` — the
block between the `<!-- download-links:start -->` and
`<!-- download-links:end -->` markers — so the advertised `.dmg` URLs point at
the release being cut. Keep those markers in place; without them `bump.sh`
warns and leaves the links alone. The URLs are built from `GH_REPO`
(default `matija/loopfleet`) and Tauri's bundle naming
(`loopfleet_<version>_aarch64.dmg` / `_x64.dmg`).

### Required environment variables

`release.sh` and `release-intel.sh` share the same five required variables
(validated up front by `release-common.sh`, before anything is built):

| Variable                                     | What it is                                                                 |
| --------------------------------------------- | --------------------------------------------------------------------------- |
| `APPLE_SIGNING_IDENTITY`                      | Codesigning identity name, must already be in the login keychain.          |
| `APPLE_API_KEY_PATH`                          | Path to the App Store Connect API key (`.p8`) used for notarization.       |
| `APPLE_API_KEY` (or `APPLE_API_KEY_ID`)       | The API key's ID, from App Store Connect.                                  |
| `APPLE_API_ISSUER`                            | The API key's issuer ID, from App Store Connect.                           |
| `TAURI_SIGNING_PRIVATE_KEY` (or `_PATH`)      | The Tauri updater signing key — inline (base64 or plaintext minisign secret) or a path to a key file. |

Both scripts also require an authenticated `gh` CLI (`gh auth login`), and
must run on an arm64 macOS host with the relevant Rust target installed.
`release.sh` additionally refuses to run from a dirty or unpushed tree (a
release must correspond to a real, pushed commit) — which is why, when you
pass it a version, it commits and pushes the bump for you before building.

### Generate and store the updater key

The updater key is a minisign keypair Tauri uses to sign update artifacts,
independent of Apple codesigning. Generate it once and keep the private half
somewhere durable and secret (a password manager, or a restricted-permission
file outside the repo):

```sh
npm run tauri signer generate -- --ci -w ~/.tauri/loopfleet.key
```

This writes the private key to `~/.tauri/loopfleet.key` (and prints the
public key, which belongs in `src-tauri/tauri.conf.json`'s
`plugins.updater.pubkey` — replace the `REPLACE_WITH_MINISIGN_PUBLIC_KEY`
placeholder there and commit it; it's public and safe to check in). To use
the private key in a release, either:

- point `TAURI_SIGNING_PRIVATE_KEY_PATH` at the key file, or
- put the key's contents directly in `TAURI_SIGNING_PRIVATE_KEY` (e.g. from a
  secrets manager in CI).

`release-common.sh` accepts either the plaintext minisign secret
(`untrusted comment: ...`) or its base64 encoding in either variable, and
normalizes it. The key has no password (`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
is set to `""` for you) — don't set one when generating it.

### Release sequence

A release is two commands, run in order on an arm64 macOS host:

```sh
build/release.sh 0.2.0        # bump to 0.2.0, commit + push the bump, build, notarize, publish aarch64
build/release-intel.sh        # build, notarize, add the Intel bundle to the same release
```

There is no manual commit between them: `release.sh <version>` commits the
bump as `chore(release): bump version to <version>` and pushes it, so
`release-intel.sh` finds the clean, pushed tree it requires.

If you're already on the right version, drop the version argument:
`build/release.sh` alone builds and publishes the current version. Either
script also accepts `--preflight`:

```sh
build/release.sh --preflight        # validate env vars/tooling only, no build
build/release-intel.sh --preflight  # same, for the Intel prerequisites
```

`--preflight` runs every check in `release-common.sh` — env vars, keychain
identity, updater key, `gh` auth, clean/pushed tree, macOS target — and exits
0 without building or touching GitHub. Run it before a release to catch a
missing variable or expired credential early, especially after rotating any
of the five variables above.

`build/release.sh` then:

1. If a version was given, runs `build/bump.sh <version>`, commits exactly the
   files that script rewrites (`Cargo.toml`, `Cargo.lock`, both
   `package.json`s, `src-tauri/tauri.conf.json`, `README.md`) as
   `chore(release): bump version to <version>`, and pushes it. Already at that
   version, it's a no-op. Use `build/bump.sh` directly to bump without
   committing or releasing.
2. Runs `tauri build --target aarch64-apple-darwin`, which also builds the
   frontend (`beforeBuildCommand`). Artifacts land under
   `target/aarch64-apple-darwin/release/bundle/` — the `.app` and updater
   tarball (`.app.tar.gz` + `.sig`) under `macos/`, the `.dmg` under `dmg/`.
3. Notarizes and staples the `.dmg`.
4. Writes `latest.json` (Tauri updater manifest) with a `darwin-aarch64`
   platform entry pointing at the updater tarball's GitHub release URL,
   signed with the tarball's `.sig`.
5. Creates the GitHub release at `RELEASE_TAG` (defaulting to the version),
   replacing it if one already exists, uploading the `.dmg`, the updater
   tarball, and `latest.json`. Release notes are a compare link against the
   previous release on GitHub.

`build/release-intel.sh` requires `RELEASE_TAG` (defaulting to the current
version) to already exist as a GitHub release — it does not create or
replace releases, only adds to one. It then:

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

### When a release half-fails

Both scripts are safe to just re-run — do that first.

- **`release.sh` fails partway** (build error, failed notarization, network
  blip during upload): the version-bump commit is already pushed, but nothing
  else durable has happened until the GitHub release is created near the end. Fix the underlying problem and re-run
  `build/release.sh` with the same version (the bump is then a no-op, so no
  empty commit); if a release at that tag already
  exists (e.g. it got created but a later step failed), the script deletes
  and replaces it rather than erroring.
- **`release.sh` succeeds but `release-intel.sh` fails or is never run**: the
  GitHub release exists with only the aarch64 `.dmg` and a `latest.json`
  containing just the `darwin-aarch64` platform entry. Intel users have no
  installer and no update path until you run `build/release-intel.sh`. This
  is a safe, visible half-state — arm64 users are unaffected — so there's no
  need to delete the release; just fix the problem and re-run
  `build/release-intel.sh`. It re-downloads `latest.json` fresh and uploads
  with `--clobber`, so re-running is idempotent even after a partial upload.
- **`release-intel.sh` fails after renaming the tarball but before
  uploading**: re-running rebuilds and re-notarizes, which is slower but
  always correct — there's no partial state on GitHub to clean up, since
  nothing is uploaded until the final step.
- **Wrong bits already uploaded to a public release**: delete the bad
  release (`gh release delete <tag> --cleanup-tag`) and re-run
  `build/release.sh <version>` from scratch, then `build/release-intel.sh`.
