//! "Use this run": merge a run's final state into a target branch (PRD "Git
//! layer" / "Compare view"). A mutation, so it shells out to the `git` CLI
//! (consistent with `worktree`/`shadow`) and funnels through the serialized
//! [`crate::GitActor`].
//!
//! The default target is the repo's **currently checked-out branch** — the run's
//! work lands where the user is working, as a single squashed commit under a
//! descriptive message. The caller may instead name a custom target branch.
//!
//! The run's work lives in an app-owned shadow commit (the agent never commits),
//! so `source_rev` is that final shadow ref. Three cases:
//!   * no custom target → squash-merge into the current branch right in the main
//!     worktree (the only place the current branch can move). Guarded by a
//!     clean working tree so uncommitted work is never clobbered. A conflicting
//!     merge is aborted, leaving the branch unchanged.
//!   * custom target doesn't exist → create it at the repo's current HEAD, then
//!     squash the run onto it in a throwaway worktree, exactly as for an
//!     existing target. A conflicting merge is aborted and the half-made branch
//!     removed, leaving the repo as it was.
//!   * custom target exists → a real squash merge in a THROWAWAY worktree so the
//!     user's own checkout is never disturbed. A conflicting merge is aborted
//!     and the target left unchanged (conflict assistance is post-v1).
//!
//! Either way the target gains exactly **one** new commit with a single parent
//! (its previous tip): the run lands squashed, not as a merge commit.
//!
//! That commit is authored by loopfleet and committed by the repo's configured
//! `user.name`/`user.email`, so history records both who produced the work and
//! who chose to land it.

use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Identity stamped as the **author** of the squashed commit: the run's work was
/// produced by loopfleet, so authorship records the tool, while the user (whoever
/// pressed "use this run") is recorded as the committer — the same author/committer
/// split git uses for applied patches. Also the committer fallback for a repo with
/// no `user.*` configured, since `git commit` needs an identity either way.
const AUTHOR_NAME: &str = "loopfleet";
const AUTHOR_EMAIL: &str = "loopfleet@localhost";

/// The outcome of a "use this run" merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    /// The branch the run was merged into.
    pub target_branch: String,
    /// The run's final commit: the shadow commit whose tree was squashed in.
    pub merged_commit: String,
    /// The target branch did not exist and was created by this merge (at the
    /// repo's HEAD, then squash-committed onto like any other target).
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
/// `Some(target)` names a custom branch: branched off the repo's HEAD first if it
/// doesn't exist, then squash-merged in a throwaway worktree under `scratch_root`
/// so the user's own checkout is never touched. `commit_message` is the squashed
/// commit's message — every path lands the run as one described commit.
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

/// Default path: squash-merge into the currently checked-out branch, in the main
/// worktree. The current branch can only move where it's checked out, so the
/// throwaway-worktree trick the named-target path uses does not apply here.
fn merge_into_current(repo: &Path, source: &str, message: &str) -> Result<MergeResult> {
    let branch = current_branch(repo)?;
    if !working_tree_clean(repo)? {
        return Err(MergeError::Git(
            "working tree is dirty; commit or stash before using this run".into(),
        ));
    }
    let up_to_date = squash_merge(repo, source, message)?;
    Ok(MergeResult {
        target_branch: branch,
        merged_commit: source.to_string(),
        created: false,
        up_to_date,
    })
}

/// Custom-target path: merge into a named branch. A branch that doesn't exist
/// yet is first created at the repo's HEAD, so it starts as an ordinary branch
/// off the user's current work; from there both cases are the same single
/// squashed commit, made in a throwaway worktree so the user's checkout is never
/// disturbed. Pointing a fresh branch straight at the shadow commit would
/// instead expose the app's synthetic run history and skip the described commit
/// every other path produces.
fn merge_into_named(
    repo: &Path,
    source: &str,
    target_branch: &str,
    message: &str,
    scratch_root: &Path,
) -> Result<MergeResult> {
    let created = !branch_exists(repo, target_branch)?;
    if created {
        // Branch off HEAD (not `source`): the run lands as a commit on top, not
        // as the branch's entire history.
        git(repo, &["branch", target_branch, "HEAD"])?;
    }

    // Target now exists either way: merge in a throwaway worktree so the user's own checkout
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

    // Squash the run onto the target inside the throwaway worktree. Whatever the
    // outcome, the worktree is torn down before returning.
    let squashed = squash_merge(&tmp, source, message);
    cleanup_worktree(repo, &tmp_str);
    let up_to_date = match squashed {
        Ok(up_to_date) => up_to_date,
        Err(e) => {
            // A branch this call invented has no reason to survive a failed
            // merge: drop it so the repo is left exactly as it was found.
            if created {
                let _ = git(repo, &["branch", "-D", target_branch]);
            }
            return Err(e);
        }
    };

    Ok(MergeResult {
        target_branch: target_branch.to_string(),
        merged_commit: source.to_string(),
        created,
        up_to_date,
    })
}

/// Stage `source`'s tree onto the branch checked out in `dir` with
/// `git merge --squash`, then commit it under `message`: the branch gains
/// exactly one commit whose only parent is its previous tip (no merge commit,
/// and `source` is not recorded as a parent).
///
/// Returns whether the target was already up to date — nothing was staged, so
/// no commit was made. A conflicting merge is rolled back (the caller
/// guarantees `dir` was clean beforehand) and reported as
/// [`MergeError::Conflict`], leaving the branch exactly as it was.
fn squash_merge(dir: &Path, source: &str, message: &str) -> Result<bool> {
    let merge = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["merge", "--squash", source])
        .output()?;
    if !merge.status.success() {
        // `--squash` sets no MERGE_HEAD, so `merge --abort` cannot undo it: reset
        // the index and working tree back to the branch tip instead.
        let _ = Command::new("git").arg("-C").arg(dir).args(["merge", "--abort"]).output();
        let _ = Command::new("git").arg("-C").arg(dir).args(["reset", "--hard", "HEAD"]).output();
        let msg = String::from_utf8_lossy(&merge.stderr).trim().to_string();
        let msg = if msg.is_empty() {
            String::from_utf8_lossy(&merge.stdout).trim().to_string()
        } else {
            msg
        };
        return Err(MergeError::Conflict(msg));
    }

    // Nothing staged → the run's tree is already contained in the target. Commit
    // would fail on an empty change, so report it as a no-op instead.
    if index_matches_head(dir)? {
        return Ok(true);
    }

    // -m so the squashed commit carries the caller's message rather than the
    // SQUASH_MSG git assembled from the run's synthetic shadow commits. The
    // identity is passed through the environment rather than `--author` so both
    // halves (author and committer) are set explicitly, including the fallback.
    let (committer_name, committer_email) = committer_identity(dir);
    git_env(
        dir,
        &["commit", "-m", message],
        &[
            ("GIT_AUTHOR_NAME", AUTHOR_NAME),
            ("GIT_AUTHOR_EMAIL", AUTHOR_EMAIL),
            ("GIT_COMMITTER_NAME", &committer_name),
            ("GIT_COMMITTER_EMAIL", &committer_email),
        ],
    )?;
    Ok(false)
}

/// The identity to record as the squashed commit's **committer**: the repo's
/// configured `user.name`/`user.email`, so the merge is attributed to whoever
/// used the run. Falls back to [`AUTHOR_NAME`]/[`AUTHOR_EMAIL`] when git has no
/// identity configured — `git commit` needs one, and without this the merge
/// would fail outright on a fresh repo. Both fields must be set to use the user
/// identity; a half-configured repo falls back wholesale rather than mixing a
/// real name with a synthetic email.
fn committer_identity(dir: &Path) -> (String, String) {
    match (git_config(dir, "user.name"), git_config(dir, "user.email")) {
        (Some(name), Some(email)) => (name, email),
        _ => (AUTHOR_NAME.to_string(), AUTHOR_EMAIL.to_string()),
    }
}

/// Read a single git config value from `dir`, or `None` when unset or empty.
/// Respects the normal local/global/system config cascade.
fn git_config(dir: &Path, key: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["config", "--get", key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// True if `dir` has nothing staged relative to HEAD.
fn index_matches_head(dir: &Path) -> Result<bool> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["diff", "--cached", "--quiet"])
        .output()?;
    Ok(out.status.success())
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
    git_env(repo, args, &[])
}

/// [`git`] with extra environment variables set on the child (used to stamp the
/// commit identity).
fn git_env(repo: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .envs(env.iter().copied())
        .output()?;
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

    /// `git -C repo <args...>`, trimmed stdout (asserts success).
    fn git_out(repo: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Number of commits reachable from `rev`.
    fn commit_count(repo: &Path, rev: &str) -> usize {
        git_out(repo, &["rev-list", "--count", rev]).parse().unwrap()
    }

    /// The parents of `rev`, as shas.
    fn parents(repo: &Path, rev: &str) -> Vec<String> {
        git_out(repo, &["rev-list", "-1", "--parents", rev])
            .split_whitespace()
            .skip(1)
            .map(str::to_string)
            .collect()
    }

    /// Assert `branch` gained exactly one commit — a squashed one, so a single
    /// parent (`prev_tip`, the branch's old tip) — carrying `subject`.
    fn assert_squashed_onto(repo: &Path, branch: &str, prev_tip: &str, prev_count: usize, subject: &str) {
        assert_eq!(commit_count(repo, branch), prev_count + 1, "{branch} should gain exactly one commit");
        assert_eq!(parents(repo, branch), vec![prev_tip.to_string()], "{branch} tip should have one parent: its old tip");
        assert_eq!(git_out(repo, &["log", "-1", "--pretty=%s", branch]), subject);
    }

    /// A custom target that doesn't exist yet is branched off HEAD and then
    /// squash-committed onto, so it ends up with the same single described
    /// commit as every other path — not pointed straight at the app's shadow
    /// commit, whose synthetic history and identities would otherwise leak onto
    /// the user's branch.
    #[test]
    fn creates_target_branch_and_squashes_run_onto_it() {
        let (repo, _root, wt) = repo_with_worktree("merge-r1");
        let head_tip = git_out(repo.path(), &["rev-parse", "HEAD"]);
        let head_count = commit_count(repo.path(), "HEAD");
        std::fs::write(wt.path.join("out.txt"), "result\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r1", 1).unwrap();

        let scratch = tempfile::tempdir().unwrap();
        let res = merge_run(
            repo.path(),
            &snap.git_ref,
            Some("review/x"),
            "Apply loopfleet run r1",
            scratch.path(),
        )
        .unwrap();

        assert!(res.created);
        assert!(!res.up_to_date);
        assert_eq!(res.target_branch, "review/x");
        assert_eq!(res.merged_commit, snap.commit);
        // The new branch carries the run's file, on top of the base content.
        assert_eq!(show(repo.path(), "review/x", "out.txt"), "result\n");
        assert_eq!(show(repo.path(), "review/x", "README.md"), "hi\n");
        // One squashed commit on top of HEAD, under the supplied message — the
        // shadow commit is not the branch tip and is not a parent.
        assert_squashed_onto(repo.path(), "review/x", &head_tip, head_count, "Apply loopfleet run r1");
        assert_ne!(git_out(repo.path(), &["rev-parse", "review/x"]), snap.commit);
        // ...stamped with the same author/committer split as the other paths.
        let (author, committer) = identity(repo.path(), "review/x");
        assert_eq!(author, "loopfleet <loopfleet@localhost>");
        assert_eq!(committer, "t <t@t.test>");
        // The throwaway worktree used for the squash is gone.
        assert!(crate::worktree::list(repo.path())
            .unwrap()
            .iter()
            .all(|w| !w.path.starts_with(scratch.path())));
    }

    /// A fresh target whose merge conflicts leaves no trace: the branch this
    /// call invented is removed rather than left behind at HEAD.
    #[test]
    fn conflicting_merge_into_fresh_target_removes_the_branch() {
        let (repo, _root, wt) = repo_with_worktree("merge-r14");
        // The run rewrites README; HEAD gains a conflicting change of its own.
        std::fs::write(wt.path.join("README.md"), "run side\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r14", 1).unwrap();
        std::fs::write(repo.path().join("README.md"), "main side\n").unwrap();
        git_out(repo.path(), &["commit", "-aqm", "main change"]);

        let scratch = tempfile::tempdir().unwrap();
        let err = merge_run(
            repo.path(),
            &snap.git_ref,
            Some("review/z"),
            "Apply loopfleet run r14",
            scratch.path(),
        )
        .unwrap_err();

        assert!(matches!(err, MergeError::Conflict(_)), "got {err:?}");
        assert!(!branch_exists(repo.path(), "review/z").unwrap(), "half-made branch should be removed");
    }

    #[test]
    fn merges_into_existing_target_branch() {
        let (repo, _root, wt) = repo_with_worktree("merge-r2");
        // Pre-create an integration branch off base (main), with its own file.
        let run = |args: &[&str]| {
            Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap()
        };
        run(&["branch", "integration", "main"]);
        let before_tip = git_out(repo.path(), &["rev-parse", "integration"]);
        let before_count = commit_count(repo.path(), "integration");

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
        assert!(!res.up_to_date);
        // The run's file landed on the existing branch; base file still present.
        assert_eq!(show(repo.path(), "integration", "feature.txt"), "feature\n");
        assert_eq!(show(repo.path(), "integration", "README.md"), "hi\n");
        // ...as a single squashed commit on top of the old tip, not a merge commit.
        assert_squashed_onto(repo.path(), "integration", &before_tip, before_count, "apply run");
        // The throwaway worktree is gone (only the main worktree remains).
        let listed = crate::worktree::list(repo.path()).unwrap();
        assert!(listed.iter().all(|w| !w.path.starts_with(scratch.path())));
    }

    /// Squashing the same run twice is a no-op the second time: the squashed
    /// commit does not record the run as a parent, so "already merged" has to be
    /// detected from an unchanged tree rather than from ancestry.
    #[test]
    fn second_squash_of_same_run_is_up_to_date() {
        let (repo, _root, wt) = repo_with_worktree("merge-r8");
        let run = |args: &[&str]| {
            Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap()
        };
        run(&["branch", "integration", "main"]);

        std::fs::write(wt.path.join("feature.txt"), "feature\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r8", 1).unwrap();

        let scratch = tempfile::tempdir().unwrap();
        let merge = || {
            merge_run(repo.path(), &snap.git_ref, Some("integration"), "apply run", scratch.path()).unwrap()
        };
        assert!(!merge().up_to_date);
        let after_first = git_out(repo.path(), &["rev-parse", "integration"]);

        let res = merge();
        assert!(res.up_to_date);
        // No second commit was made.
        assert_eq!(git_out(repo.path(), &["rev-parse", "integration"]), after_first);
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

        let int_tip = git_out(repo.path(), &["rev-parse", "integration"]);

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
        // Target unchanged: still the integration-side content, tip did not move,
        // no lingering worktree.
        assert_eq!(show(repo.path(), "integration", "README.md"), "integration side\n");
        assert_eq!(git_out(repo.path(), &["rev-parse", "integration"]), int_tip);
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
        let before_tip = git_out(repo.path(), &["rev-parse", "main"]);
        let before_count = commit_count(repo.path(), "main");
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
        // Exactly one new commit, single-parented on main's old tip, carrying the
        // supplied message — a squash, not a merge commit.
        assert_squashed_onto(repo.path(), "main", &before_tip, before_count, "Apply loopfleet run r5");
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

    /// The identity (`%an <%ae>` / `%cn <%ce>`) of `rev`'s commit.
    fn identity(repo: &Path, rev: &str) -> (String, String) {
        let author = git_out(repo, &["log", "-1", "--pretty=%an <%ae>", rev]);
        let committer = git_out(repo, &["log", "-1", "--pretty=%cn <%ce>", rev]);
        (author, committer)
    }

    /// The squashed commit is authored by loopfleet (the run produced the work)
    /// but committed by the repo's configured identity (the user chose to use
    /// it) — the author/committer split git uses for applied patches.
    #[test]
    fn squashed_commit_is_authored_by_loopfleet_and_committed_by_the_user() {
        let (repo, _root, wt) = repo_with_worktree("merge-r11");
        std::fs::write(wt.path.join("out.txt"), "result\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r11", 1).unwrap();

        let scratch = tempfile::tempdir().unwrap();
        merge_run(repo.path(), &snap.git_ref, None, "Apply loopfleet run r11", scratch.path()).unwrap();

        let (author, committer) = identity(repo.path(), "main");
        assert_eq!(author, "loopfleet <loopfleet@localhost>");
        assert_eq!(committer, "t <t@t.test>");
    }

    /// The same holds for a merge into an existing custom target, which commits
    /// in a throwaway worktree rather than the main one.
    #[test]
    fn custom_target_merge_stamps_the_same_identities() {
        let (repo, _root, wt) = repo_with_worktree("merge-r12");
        let run = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["branch", "integration", "main"]);
        std::fs::write(wt.path.join("out.txt"), "result\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "merge-r12", 1).unwrap();

        let scratch = tempfile::tempdir().unwrap();
        merge_run(repo.path(), &snap.git_ref, Some("integration"), "Apply loopfleet run r12", scratch.path()).unwrap();

        let (author, committer) = identity(repo.path(), "integration");
        assert_eq!(author, "loopfleet <loopfleet@localhost>");
        assert_eq!(committer, "t <t@t.test>");
    }

    /// With no usable `user.*` in the repo, the loopfleet identity stands in for
    /// the committer too — without it `git commit` would refuse to run at all
    /// ("Please tell me who you are"), failing the merge. Local empty values
    /// are used so the result does not depend on the machine's global git
    /// config, which the spawned `git` children would otherwise inherit.
    #[test]
    fn falls_back_to_loopfleet_committer_when_repo_has_no_identity() {
        let (repo, _root, wt) = repo_with_worktree("merge-r13");
        let run = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        std::fs::write(wt.path.join("out.txt"), "result\n").unwrap();
        // Snapshot first: the shadow commit needs an identity of its own.
        let snap = snapshot(repo.path(), &wt.path, "merge-r13", 1).unwrap();
        run(&["config", "--local", "user.name", ""]);
        run(&["config", "--local", "user.email", ""]);

        let scratch = tempfile::tempdir().unwrap();
        merge_run(repo.path(), &snap.git_ref, None, "Apply loopfleet run r13", scratch.path()).unwrap();

        let (author, committer) = identity(repo.path(), "main");
        assert_eq!(author, "loopfleet <loopfleet@localhost>");
        assert_eq!(committer, "loopfleet <loopfleet@localhost>");
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
