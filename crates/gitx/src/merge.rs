//! "Use this run": merge a run's final state into a target branch (PRD "Git
//! layer" / "Compare view"). A mutation, so it shells out to the `git` CLI
//! (consistent with `worktree`/`shadow`) and funnels through the serialized
//! [`crate::GitActor`].
//!
//! The default target is the repo's **currently checked-out branch** — the run's
//! work lands where the user is working, under a descriptive merge commit. The
//! caller may instead name a custom target branch.
//!
//! The run's work lives in an app-owned shadow commit (the agent never commits),
//! so `source_rev` is that final shadow ref. Three cases:
//!   * no custom target → merge into the current branch right in the main
//!     worktree (the only place the current branch can move). Guarded by a
//!     clean working tree so uncommitted work is never clobbered. A conflicting
//!     merge is aborted, leaving the branch unchanged.
//!   * custom target doesn't exist → create it pointing at the run's final
//!     commit (a pure ref creation; no working tree touched, no conflicts).
//!   * custom target exists → a real `git merge` in a THROWAWAY worktree so the
//!     user's own checkout is never disturbed. A conflicting merge is aborted
//!     and the target left unchanged (conflict assistance is post-v1).

use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The outcome of a "use this run" merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    /// The branch the run was merged into.
    pub target_branch: String,
    /// The run's final commit (the shadow commit that was merged/pointed to).
    pub merged_commit: String,
    /// The target branch was newly created at the run's final commit.
    pub created: bool,
    /// The target already contained the run's commit — the merge was a no-op.
    pub up_to_date: bool,
}

/// Why a "use this run" merge failed.
#[derive(Debug)]
pub enum MergeError {
    /// The `git` process could not be spawned or its output read.
    Io(std::io::Error),
    /// `git` ran but exited non-zero (bad target name, branch checked out
    /// elsewhere, etc.); carries the trimmed stderr.
    Git(String),
    /// The merge left conflicts; it was aborted and the target is unchanged.
    Conflict(String),
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeError::Io(e) => write!(f, "git merge: {e}"),
            MergeError::Git(msg) => write!(f, "git merge failed: {msg}"),
            MergeError::Conflict(msg) => write!(f, "merge has conflicts (aborted): {msg}"),
        }
    }
}

impl std::error::Error for MergeError {}

impl From<std::io::Error> for MergeError {
    fn from(e: std::io::Error) -> Self {
        MergeError::Io(e)
    }
}

type Result<T> = std::result::Result<T, MergeError>;

/// Merge run commit `source_rev` into a target branch in `repo`.
///
/// `target_branch = None` merges into the repo's currently checked-out branch
/// (the default "use this run" flow): the merge runs in the main worktree, so
/// the user's working tree advances to include the run. A dirty working tree is
/// refused up front so uncommitted work is never clobbered.
///
/// `Some(target)` names a custom branch: created at the run's final commit if it
/// doesn't exist, else merged in a throwaway worktree under `scratch_root` so the
/// user's own checkout is never touched. `commit_message` is the merge commit
/// message (used for the real merges; the fresh-branch case has no new commit).
pub fn merge_run(
    repo: &Path,
    source_rev: &str,
    target_branch: Option<&str>,
    commit_message: &str,
    scratch_root: &Path,
) -> Result<MergeResult> {
    // Resolve the source ref to a concrete commit sha (also validates it exists).
    let source = git(repo, &["rev-parse", "--verify", &format!("{source_rev}^{{commit}}")])?;

    match target_branch {
        None => merge_into_current(repo, &source, commit_message),
        Some(target) => merge_into_named(repo, &source, target, commit_message, scratch_root),
    }
}

/// Default path: merge into the currently checked-out branch, in the main
/// worktree. The current branch can only move where it's checked out, so the
/// throwaway-worktree trick the named-target path uses does not apply here.
fn merge_into_current(repo: &Path, source: &str, message: &str) -> Result<MergeResult> {
    let branch = current_branch(repo)?;
    if !working_tree_clean(repo)? {
        return Err(MergeError::Git(
            "working tree is dirty; commit or stash before using this run".into(),
        ));
    }
    // --no-ff so the run's landing is a named merge commit even when it could
    // fast-forward (the run's shadow commit would otherwise bring its own
    // synthetic message forward as HEAD).
    let merge = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge", "--no-ff", "-m", message, source])
        .output()?;
    let stdout = String::from_utf8_lossy(&merge.stdout).to_string();
    if !merge.status.success() {
        let _ = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["merge", "--abort"])
            .output();
        let msg = String::from_utf8_lossy(&merge.stderr).trim().to_string();
        let msg = if msg.is_empty() { stdout.trim().to_string() } else { msg };
        return Err(MergeError::Conflict(msg));
    }
    Ok(MergeResult {
        target_branch: branch,
        merged_commit: source.to_string(),
        created: false,
        up_to_date: stdout.contains("Already up to date"),
    })
}

/// Custom-target path: merge into a named branch. A fresh branch is created at
/// the run's commit (no merge commit); an existing branch is merged in a
/// throwaway worktree so the user's checkout is never disturbed.
fn merge_into_named(
    repo: &Path,
    source: &str,
    target_branch: &str,
    message: &str,
    scratch_root: &Path,
) -> Result<MergeResult> {
    if !branch_exists(repo, target_branch)? {
        // Fresh target: point it straight at the run's final commit. No worktree,
        // no conflicts — this is the custom-target "create" flow.
        git(repo, &["branch", target_branch, source])?;
        return Ok(MergeResult {
            target_branch: target_branch.to_string(),
            merged_commit: source.to_string(),
            created: true,
            up_to_date: false,
        });
    }

    // Existing target: merge in a throwaway worktree so the user's own checkout
    // is never disturbed. A unique path keyed by pid+nanos avoids collisions.
    std::fs::create_dir_all(scratch_root)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = scratch_root.join(format!("merge-{}-{}", std::process::id(), nanos));
    let tmp_str = tmp.to_string_lossy().into_owned();

    // Check out the target branch into the throwaway worktree. git refuses if the
    // branch is already checked out in the main worktree — a natural guard that
    // also stops a custom target from naming the current branch (use the default
    // path for that).
    git(repo, &["worktree", "add", &tmp_str, target_branch])?;

    let merge = Command::new("git")
        .arg("-C")
        .arg(&tmp)
        .args(["merge", "--no-ff", "-m", message, source])
        .output()?;
    let stdout = String::from_utf8_lossy(&merge.stdout).to_string();

    if !merge.status.success() {
        // Abort the conflicting merge and tear down the throwaway worktree so the
        // target branch is left exactly as it was.
        let _ = Command::new("git").arg("-C").arg(&tmp).args(["merge", "--abort"]).output();
        let msg = String::from_utf8_lossy(&merge.stderr).trim().to_string();
        let msg = if msg.is_empty() { stdout.trim().to_string() } else { msg };
        cleanup_worktree(repo, &tmp_str);
        return Err(MergeError::Conflict(msg));
    }

    cleanup_worktree(repo, &tmp_str);
    Ok(MergeResult {
        target_branch: target_branch.to_string(),
        merged_commit: source.to_string(),
        created: false,
        up_to_date: stdout.contains("Already up to date"),
    })
}

/// The branch currently checked out in `repo`'s main worktree, or an error if
/// HEAD is detached (no branch to merge into).
fn current_branch(repo: &Path) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(MergeError::Git(
            "HEAD is detached; check out a branch before using this run".into(),
        ))
    }
}

/// True if `repo`'s working tree has no uncommitted changes (staged or
/// unstaged). Untracked files count as dirty — a merge could clobber them.
fn working_tree_clean(repo: &Path) -> Result<bool> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["status", "--porcelain"])
        .output()?;
    Ok(out.status.success() && out.stdout.is_empty())
}

/// True if `refs/heads/<branch>` exists in `repo`.
fn branch_exists(repo: &Path, branch: &str) -> Result<bool> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .output()?;
    Ok(out.status.success())
}

/// Best-effort removal of the throwaway merge worktree.
fn cleanup_worktree(repo: &Path, path: &str) {
    let _ = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "remove", "--force", path])
        .output();
}

/// Run `git -C <repo> <args...>`, returning trimmed stdout or the stderr on a
/// non-zero exit.
fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        Err(MergeError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow::snapshot;

    /// A repo with one commit plus a run worktree cut from it, so a run's final
    /// shadow ref can be produced.
    fn repo_with_worktree(run_id: &str) -> (tempfile::TempDir, tempfile::TempDir, crate::worktree::Worktree) {
        let repo = tempfile::tempdir().unwrap();
        let p = repo.path();
        let run = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(p).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.test"]);
        run(&["config", "user.name", "t"]);
        run(&["config", "commit.gpgsign", "false"]);
        // A default branch that is NOT what tests target, so the target branch is
        // never the checked-out one (git forbids checking that out twice).
        run(&["checkout", "-q", "-b", "main"]);
        std::fs::write(p.join("README.md"), "hi\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);

        let root = tempfile::tempdir().unwrap();
        let wt = crate::worktree::add(p, root.path(), run_id).unwrap();
        (repo, root, wt)
    }

    fn show(repo: &Path, rev: &str, path: &str) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["show", &format!("{rev}:{path}")])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn creates_target_branch_at_run_final_commit() {
        let (repo, _root, wt) = repo_with_worktree("merge-r1");
        std::fs::write(wt.path.join("out.txt"), "result\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r1", 1).unwrap();

        let scratch = tempfile::tempdir().unwrap();
        let res = merge_run(
            repo.path(),
            &snap.git_ref,
            Some("review/x"),
            "apply run",
            scratch.path(),
        )
        .unwrap();

        assert!(res.created);
        assert_eq!(res.target_branch, "review/x");
        assert_eq!(res.merged_commit, snap.commit);
        // The new branch carries the run's file.
        assert_eq!(show(repo.path(), "review/x", "out.txt"), "result\n");
    }

    #[test]
    fn merges_into_existing_target_branch() {
        let (repo, _root, wt) = repo_with_worktree("merge-r2");
        // Pre-create an integration branch off base (main), with its own file.
        let run = |args: &[&str]| {
            Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap()
        };
        run(&["branch", "integration", "main"]);

        std::fs::write(wt.path.join("feature.txt"), "feature\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r2", 1).unwrap();

        let scratch = tempfile::tempdir().unwrap();
        let res = merge_run(
            repo.path(),
            &snap.git_ref,
            Some("integration"),
            "apply run",
            scratch.path(),
        )
        .unwrap();

        assert!(!res.created);
        // The run's file landed on the existing branch; base file still present.
        assert_eq!(show(repo.path(), "integration", "feature.txt"), "feature\n");
        assert_eq!(show(repo.path(), "integration", "README.md"), "hi\n");
        // The throwaway worktree is gone (only the main worktree remains).
        let listed = crate::worktree::list(repo.path()).unwrap();
        assert!(listed.iter().all(|w| !w.path.starts_with(scratch.path())));
    }

    #[test]
    fn conflicting_merge_is_aborted_and_target_unchanged() {
        let (repo, _root, wt) = repo_with_worktree("merge-r3");
        let run = |args: &[&str]| {
            Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap()
        };
        // Integration branch changes README to a conflicting value and commits.
        run(&["branch", "integration", "main"]);
        let iwt = tempfile::tempdir().unwrap();
        run(&["worktree", "add", &iwt.path().to_string_lossy(), "integration"]);
        std::fs::write(iwt.path().join("README.md"), "integration side\n").unwrap();
        Command::new("git").arg("-C").arg(iwt.path()).args(["commit", "-aqm", "int"]).output().unwrap();
        run(&["worktree", "remove", "--force", &iwt.path().to_string_lossy()]);

        // The run changes the same file differently.
        std::fs::write(wt.path.join("README.md"), "run side\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r3", 1).unwrap();

        let scratch = tempfile::tempdir().unwrap();
        let err = merge_run(
            repo.path(),
            &snap.git_ref,
            Some("integration"),
            "apply run",
            scratch.path(),
        )
        .unwrap_err();
        assert!(matches!(err, MergeError::Conflict(_)), "got {err:?}");
        // Target unchanged: still the integration-side content, no lingering worktree.
        assert_eq!(show(repo.path(), "integration", "README.md"), "integration side\n");
        assert!(crate::worktree::list(repo.path())
            .unwrap()
            .iter()
            .all(|w| !w.path.starts_with(scratch.path())));
    }

    #[test]
    fn missing_source_ref_errors() {
        let (repo, _root, _wt) = repo_with_worktree("merge-r4");
        let scratch = tempfile::tempdir().unwrap();
        let err = merge_run(
            repo.path(),
            "refs/agentapp/run-nope/iter-9",
            Some("review/y"),
            "apply run",
            scratch.path(),
        )
        .unwrap_err();
        assert!(matches!(err, MergeError::Git(_)), "got {err:?}");
    }

    /// Default flow: no custom target merges into the currently checked-out
    /// branch (`main`) in the main worktree, under the supplied commit message.
    /// The run's file lands on `main` and HEAD's subject is the merge message.
    #[test]
    fn merges_into_current_branch_by_default() {
        let (repo, _root, wt) = repo_with_worktree("merge-r5");
        std::fs::write(wt.path.join("out.txt"), "result\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r5", 1).unwrap();

        let scratch = tempfile::tempdir().unwrap();
        let res = merge_run(repo.path(), &snap.git_ref, None, "Apply loopfleet run r5", scratch.path()).unwrap();

        assert_eq!(res.target_branch, "main");
        assert!(!res.created);
        assert!(!res.up_to_date);
        // The run's file is now on the checked-out branch, in the working tree.
        assert_eq!(show(repo.path(), "main", "out.txt"), "result\n");
        assert!(repo.path().join("out.txt").exists());
        // The merge commit carries the supplied message.
        let subject = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["log", "-1", "--pretty=%s"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&subject.stdout).trim(), "Apply loopfleet run r5");
    }

    /// A dirty working tree is refused — the merge must not clobber uncommitted
    /// work. `main` stays at its original tip.
    #[test]
    fn dirty_working_tree_refuses_default_merge() {
        let (repo, _root, wt) = repo_with_worktree("merge-r6");
        std::fs::write(wt.path.join("out.txt"), "result\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r6", 1).unwrap();

        // An untracked file in the main worktree makes it dirty.
        std::fs::write(repo.path().join("uncommitted.txt"), "local\n").unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let err = merge_run(repo.path(), &snap.git_ref, None, "Apply loopfleet run r6", scratch.path()).unwrap_err();
        assert!(matches!(err, MergeError::Git(ref msg) if msg.contains("dirty")), "got {err:?}");
        // main never moved: out.txt is not on it.
        let show_out = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["show", "main:out.txt"])
            .output()
            .unwrap();
        assert!(!show_out.status.success(), "main should not carry the run's file");
    }

    /// A detached HEAD has no current branch to merge into, so the default flow
    /// errors rather than guessing.
    #[test]
    fn detached_head_refuses_default_merge() {
        let (repo, _root, wt) = repo_with_worktree("merge-r7");
        std::fs::write(wt.path.join("out.txt"), "result\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r7", 1).unwrap();

        let run = |args: &[&str]| {
            Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap()
        };
        run(&["checkout", "-q", "--detach", "main"]);

        let scratch = tempfile::tempdir().unwrap();
        let err = merge_run(repo.path(), &snap.git_ref, None, "Apply loopfleet run r7", scratch.path()).unwrap_err();
        assert!(matches!(err, MergeError::Git(ref msg) if msg.contains("detached")), "got {err:?}");
    }
}
