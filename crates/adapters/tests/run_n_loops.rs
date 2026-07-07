//! End-to-end M3 capstone: run N loops on one task against a fixture repo with
//! the real Claude Code adapter, confined by a rendered Seatbelt profile.
//!
//! This is the composition every prior M3 piece was built for: the serialized
//! [`GitActor`] cuts a worktree and takes app-owned shadow snapshots, the
//! `SeatbeltSandbox` renders the per-run boundary, [`confine_prefix`] turns it
//! into the opaque wrapper prefix the [`ClaudeAdapter`] prepends to its spawn,
//! and [`run_loop`] drives the passes until the agent writes `STATUS: COMPLETE`
//! to its external progress file.
//!
//! Ignored by default and macOS-only: it spawns the real `claude` CLI (needs the
//! binary, network, and credits) under `sandbox-exec`. Run it manually, OUTSIDE
//! any nested sandbox (a nested `sandbox_apply` is denied by the OS, so the agent
//! would fail to start — the same limitation as the other live sandbox tests):
//!
//!   cargo test -p loopfleet-adapters --test run_n_loops -- --ignored
//!
//! [`GitActor`]: loopfleet_gitx::GitActor
//! [`confine_prefix`]: loopfleet_sandbox::confine_prefix

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

use loopfleet_adapters::ClaudeAdapter;
use loopfleet_core::{run_loop, LoopConfig, RunState};
use loopfleet_gitx::GitActor;
use loopfleet_sandbox::{confine_prefix, RenderParams};

/// A repo with one commit so `worktree add -b` has a HEAD to branch from.
fn repo_with_commit() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let run = |args: &[&str]| {
        let out = Command::new("git").arg("-C").arg(p).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "t@t.test"]);
    run(&["config", "user.name", "t"]);
    run(&["config", "commit.gpgsign", "false"]);
    std::fs::write(p.join("README.md"), "fixture\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "init"]);
    dir
}

/// The dirs claude writes to under `$HOME` (config, cache, session state). These
/// must be granted or the sandboxed CLI cannot start; tune per environment if the
/// agent fails to write.
fn claude_agent_dirs() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").expect("HOME is set"));
    [".claude", ".claude.json", ".config", ".cache"]
        .into_iter()
        .map(|d| home.join(d))
        .collect()
}

/// `true` if `git_ref` resolves to a commit in `repo` — i.e. the shadow snapshot
/// was taken.
fn ref_exists(repo: &Path, git_ref: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", git_ref])
        .output()
        .unwrap()
        .status
        .success()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "spawns the real claude CLI under sandbox-exec; needs network + credits, run outside a nested sandbox"]
async fn runs_n_loops_on_a_task_with_claude() {
    let run_id = "e2e-claude";
    let repo = repo_with_commit();
    let worktrees_root = tempfile::tempdir().unwrap();
    let progress_dir = tempfile::tempdir().unwrap();
    let profile_dir = tempfile::tempdir().unwrap();

    // One git actor: cut the per-run worktree (and, later, take snapshots).
    let git = GitActor::spawn();
    let worktree = git
        .worktree_add(
            repo.path().into(),
            worktrees_root.path().into(),
            run_id.into(),
        )
        .await
        .unwrap();

    // Render the per-run Seatbelt boundary and turn it into the opaque wrapper
    // prefix the adapter prepends — writes confined to the worktree + progress
    // dir + claude's config dirs + temp.
    let mut params = RenderParams::new(&worktree.path, progress_dir.path());
    params.agent_dirs = claude_agent_dirs();
    let wrapper = confine_prefix(&params, &profile_dir.path().join("run.sb")).unwrap();

    let cfg = LoopConfig {
        run_id: run_id.into(),
        repo: repo.path().into(),
        worktree: worktree.path.clone(),
        progress_path: progress_dir.path().join("progress.md"),
        task_text: "Create a file named DONE.txt in the current directory whose \
                    entire contents are the single word: done"
            .into(),
        max_iterations: 3,
        wrapper,
    };

    let (_cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
    let outcome = run_loop(&ClaudeAdapter, &git, &cfg, &mut cancel_rx, &mut |_pass, _ev| {}).await;

    // The agent wrote STATUS: COMPLETE within N passes.
    assert_eq!(
        outcome.state,
        RunState::Completed,
        "run did not complete: {outcome:?}"
    );
    assert!(!outcome.iterations.is_empty(), "no snapshots were taken");

    // Each pass produced an app-owned shadow snapshot that resolves in the repo.
    for iter in &outcome.iterations {
        assert!(
            ref_exists(repo.path(), &iter.shadow_ref),
            "shadow ref missing: {}",
            iter.shadow_ref
        );
    }

    // The task's artifact really landed in the confined worktree.
    let done = worktree.path.join("DONE.txt");
    assert!(done.exists(), "DONE.txt was not created in the worktree");
    assert_eq!(std::fs::read_to_string(&done).unwrap().trim(), "done");
}
