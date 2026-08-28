//! Read-only worktree status, for the app-sourced `FileChanged` lane.
//!
//! The live run view watches a run's worktree and emits `FileChanged { path }`
//! for files the agent touches. That signal is observed here (via `git2`, a
//! read — mutations shell out; PRD "Git layer"), never parsed from the agent
//! stream, so it is reliable across agents and catches files changed by shell
//! commands too.

use std::path::Path;

/// Paths in `worktree` that differ from `HEAD` (staged, unstaged, or untracked),
/// relative to the worktree root. Read-only; opens the worktree with `git2` and
/// never mutates. A clean worktree yields an empty list.
pub fn worktree_changes(worktree: &Path) -> Result<Vec<String>, git2::Error> {
    let repo = git2::Repository::open(worktree)?;
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = repo.statuses(Some(&mut opts))?;

    let mut paths = Vec::new();
    for entry in statuses.iter() {
        if entry.status() == git2::Status::CURRENT {
            continue;
        }
        if let Some(p) = entry.path() {
            paths.push(p.to_string());
        }
    }
    Ok(paths)
}

/// True if `repo` has a merge in progress (conflicts left for the user to
/// resolve by hand, `MERGE_HEAD` present). Auto-merge must not run against a
/// repo in this state — `git merge --squash` would land on top of an
/// unresolved merge instead of failing cleanly. Read-only; opens `repo` with
/// `git2` and never mutates.
pub fn merge_in_progress(repo: &Path) -> Result<bool, git2::Error> {
    let repo = git2::Repository::open(repo)?;
    Ok(repo.state() == git2::RepositoryState::Merge)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repo with one commit, so a linked worktree can branch from HEAD.
    fn repo_with_commit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(p)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.test"]);
        run(&["config", "user.name", "t"]);
        run(&["checkout", "-q", "-b", "main"]);
        std::fs::write(p.join("tracked.txt"), "one\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn reports_edits_and_untracked_but_not_a_clean_tree() {
        let repo = repo_with_commit();
        let wt = repo.path();

        // Clean to start.
        assert!(worktree_changes(wt).unwrap().is_empty());

        // Modify a tracked file and add an untracked one.
        std::fs::write(wt.join("tracked.txt"), "two\n").unwrap();
        std::fs::write(wt.join("fresh.txt"), "new\n").unwrap();

        let mut changed = worktree_changes(wt).unwrap();
        changed.sort();
        assert_eq!(changed, vec!["fresh.txt".to_string(), "tracked.txt".to_string()]);
    }

    #[test]
    fn clean_repo_has_no_merge_in_progress() {
        let repo = repo_with_commit();
        assert!(!merge_in_progress(repo.path()).unwrap());
    }

    /// A conflicting `git merge` leaves `MERGE_HEAD` set until the user resolves
    /// it and commits (or aborts) — that's the state auto-merge must not run in.
    #[test]
    fn conflicting_merge_is_reported_as_in_progress() {
        let repo = repo_with_commit();
        let p = repo.path();
        let run = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(p).args(args).output().unwrap()
        };
        run(&["checkout", "-qb", "side"]);
        std::fs::write(p.join("tracked.txt"), "side\n").unwrap();
        run(&["commit", "-aqm", "side change"]);
        run(&["checkout", "-q", "main"]);
        std::fs::write(p.join("tracked.txt"), "main\n").unwrap();
        run(&["commit", "-aqm", "main change"]);

        let out = run(&["merge", "side"]);
        assert!(!out.status.success(), "merge should conflict");

        assert!(merge_in_progress(p).unwrap());
    }
}
