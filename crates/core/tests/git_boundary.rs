//! Regression test for the sandbox write boundary around the parent repo's
//! `.git`.
//!
//! The PRD's Sandbox / Git-layer design makes commits app-owned: the sandboxed
//! agent never gets `.git` write. An agent that COULD write `.git` can plant a
//! `hooks/` script or set `core.hooksPath` / `core.sshCommand` in `config` that
//! later runs UNSANDBOXED with the user's privileges — a real escape. This test
//! locks the boundary in from both sides:
//!   - an agent confined by the rendered Seatbelt profile CANNOT write the
//!     parent repo's `.git` (nor anything outside its worktree), yet
//!   - the app's own out-of-sandbox git actor (the shadow snapshotter) still
//!     commits the worktree's state.
//!
//! This supersedes the original M2 bullet's ".git-grant succeeds" test, whose
//! premise (a parent-`.git` write grant) was removed once commits became
//! app-owned. See PRD "Sandbox" / "Git layer".
//!
//! Real `sandbox-exec`, macOS only, and it must run OUTSIDE a sandbox: nested
//! `sandbox_apply` is denied, so the confined command never actually runs and
//! the deny cannot be observed. `#[ignore]`d like the other sandbox-exec tests;
//! run manually:
//!   cargo test -p loopfleet-core --test git_boundary -- --ignored

#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Output};

use loopfleet_gitx::{shadow, worktree};
use loopfleet_sandbox::{RenderParams, Sandbox, SandboxCommand, SeatbeltSandbox};

/// Run `git -C <dir> <args…>`, asserting success.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repo with one commit to branch a worktree from.
fn repo_with_commit() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    let p = repo.path();
    git(p, &["init", "-q"]);
    git(p, &["config", "user.email", "t@t.test"]);
    git(p, &["config", "user.name", "t"]);
    // Don't inherit the user's global commit.gpgsign — no gpg agent in tests.
    git(p, &["config", "commit.gpgsign", "false"]);
    std::fs::write(p.join("README.md"), "hi\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "init"]);
    repo
}

/// Wrap `program args…` under a per-call profile confined to `worktree` (+ the
/// progress dir) and spawn it. temp_dirs is deliberately empty: the repo lives
/// in a temp dir (macOS `/var/folders`), so granting the standard temp set would
/// sweep the parent `.git` into the write boundary and defeat the test.
fn run_sandboxed(
    worktree: &Path,
    progress: &Path,
    profile_path: &Path,
    program: &str,
    args: &[&Path],
) -> Output {
    let mut params = RenderParams::new(worktree, progress);
    params.temp_dirs.clear();
    let cmd = SandboxCommand {
        program: OsString::from(program),
        args: args.iter().map(|p| p.as_os_str().to_os_string()).collect(),
        params,
        profile_path: profile_path.to_path_buf(),
    };
    let wrapped = SeatbeltSandbox.wrap(&cmd).unwrap();
    Command::new(wrapped.program())
        .args(wrapped.args())
        .output()
        .unwrap()
}

#[test]
#[ignore]
fn agent_cannot_write_parent_git_while_app_actor_commits() {
    let repo = repo_with_commit();
    let root = tempfile::tempdir().unwrap();
    let progress = tempfile::tempdir().unwrap();
    let run_id = "boundary-1";

    let wt = worktree::add(repo.path(), root.path(), run_id).unwrap();

    // The app's out-of-sandbox git actor snapshots the worktree to a shadow ref
    // WITHOUT the agent ever touching `.git` — this is exactly why the agent
    // needs no `.git` write. Runs unconditionally (out of sandbox), so it proves
    // the actor commits even in a nested-sandbox environment where the confined
    // checks below skip.
    std::fs::write(wt.path.join("work.txt"), "agent output\n").unwrap();
    let snap = shadow::snapshot(repo.path(), &wt.path, run_id, 1).unwrap();
    let resolved = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["rev-parse", "--verify", &snap.git_ref])
        .output()
        .unwrap();
    assert!(
        resolved.status.success(),
        "app actor's shadow ref should resolve: {}",
        String::from_utf8_lossy(&resolved.stderr)
    );

    // NEGATIVE: a confined agent tries to plant a hook in the PARENT repo's
    // `.git` — the exact escape the boundary exists to close.
    let planted = repo.path().join(".git/hooks/pwned");
    let out = run_sandboxed(
        &wt.path,
        progress.path(),
        &root.path().join("neg.sb"),
        "/usr/bin/touch",
        &[&planted],
    );

    // If THIS test itself runs inside a Seatbelt sandbox, `sandbox_apply` is
    // denied and the confined command never runs — the boundary can't be
    // observed, so skip (the app-actor half above already ran). Run outside a
    // sandbox to exercise the deny for real.
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("sandbox_apply") {
        eprintln!("skipping confined checks: nested sandbox denied sandbox_apply");
        return;
    }

    assert!(
        !out.status.success(),
        "writing the parent repo's .git must be denied, got success: {stderr}"
    );
    assert!(
        !planted.exists(),
        "the .git hook must not have been created"
    );

    // POSITIVE control: the same confined agent CAN write inside its worktree —
    // proves the profile is a real boundary, not a blanket deny.
    let inside = wt.path.join("agent_wrote_here");
    let ok = run_sandboxed(
        &wt.path,
        progress.path(),
        &root.path().join("pos.sb"),
        "/usr/bin/touch",
        &[&inside],
    );
    assert!(
        ok.status.success(),
        "writing inside the worktree should be allowed: {}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(inside.exists(), "the worktree write should have landed");
}
