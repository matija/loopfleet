//! The compare view (M4): the runs bound to one task, side by side, each with
//! the diff of what it produced (its final-ref cumulative diff).
//!
//! This is the read side of the review/compare queue. Two runs both "complete"
//! on the same task is resolved by showing both diffs — the app never scores or
//! judges (PRD open question 3). Each run's diff is `base HEAD .. final shadow
//! ref`, so the columns are directly comparable. The mutation half — "use this
//! run" (merge the chosen run's branch into a user-named target) — lives in the
//! git actor and store, orchestrated by the app command; this module is
//! read-only over the app-owned shadow refs.

use loopfleet_gitx::run_cumulative_diff_at;
use loopfleet_store::Connection;
use serde::Serialize;

use crate::timeline::{to_diff_view, DiffView};

/// The runs competing on one task, each with its produced diff.
#[derive(Debug, Serialize)]
pub struct CompareView {
    pub task_anchor: String,
    pub runs: Vec<RunCompare>,
}

/// One run in the compare view: its identity, acceptance, and cumulative diff.
#[derive(Debug, Serialize)]
pub struct RunCompare {
    pub run_id: String,
    pub agent: String,
    pub status: String,
    pub accepted: bool,
    /// The run's final iteration shadow ref (`None` if it produced no snapshot).
    pub final_ref: Option<String>,
    /// What the run produced against its base (`None` if there is no snapshot or
    /// the shadow refs are unreadable).
    pub diff: Option<DiffView>,
}

/// Why a compare view could not be built.
#[derive(Debug)]
pub enum CompareError {
    /// Reading runs or iterations failed.
    Store(rusqlite::Error),
}

impl std::fmt::Display for CompareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompareError::Store(e) => write!(f, "reading compare view: {e}"),
        }
    }
}

impl std::error::Error for CompareError {}

impl From<rusqlite::Error> for CompareError {
    fn from(e: rusqlite::Error) -> Self {
        CompareError::Store(e)
    }
}

/// Build the compare view for the task anchored at `task_anchor` in `plan_id`:
/// every run bound to it, in insertion order, with its final-ref cumulative diff.
pub fn compare_view(
    conn: &Connection,
    plan_id: &str,
    task_anchor: &str,
) -> Result<CompareView, CompareError> {
    let summaries = loopfleet_store::list_runs_for_plan(conn, plan_id)?;
    let mut runs = Vec::new();
    for s in summaries.into_iter().filter(|s| s.task_anchor == task_anchor) {
        // Recover the parent repo (where the shadow refs live) and the final
        // iteration to diff against its base.
        let detail = loopfleet_store::load_run(conn, &s.id)?;
        let iters = loopfleet_store::load_iterations(conn, &s.id)?;
        let final_iter = iters.last();

        let diff = match (&detail, final_iter) {
            (Some(d), Some(it)) => {
                let repo = std::path::Path::new(&d.repo_path);
                run_cumulative_diff_at(repo, &s.id, it.n).ok().map(to_diff_view)
            }
            _ => None,
        };

        runs.push(RunCompare {
            run_id: s.id,
            agent: detail.map(|d| d.agent).unwrap_or_default(),
            status: s.status,
            accepted: s.accepted,
            final_ref: final_iter.and_then(|it| it.shadow_ref.clone()),
            diff,
        });
    }

    Ok(CompareView {
        task_anchor: task_anchor.to_string(),
        runs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use loopfleet_store::NewRun;

    fn seed(conn: &Connection, repo_path: &str) -> String {
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('p', ?1, 'prd')",
            [repo_path],
        )
        .unwrap();
        let pid = loopfleet_store::plan_id("p", "PRD.md");
        loopfleet_store::upsert_plan(conn, &pid, "p", "PRD.md").unwrap();
        loopfleet_store::upsert_task(conn, &pid, "task a", 1, "Task A", false).unwrap();
        loopfleet_store::upsert_task(conn, &pid, "task b", 2, "Task B", false).unwrap();
        pid
    }

    fn run(id: &str, pid: &str, anchor: &str, status: &str) -> NewRun {
        NewRun {
            id: id.into(),
            plan_id: pid.into(),
            task_anchor: anchor.into(),
            agent: "claude".into(),
            worktree_path: "/wt".into(),
            branch: format!("agent/{id}"),
            sb_profile: "/p.sb".into(),
            progress_path: "/prog.md".into(),
            max_iterations: 3,
            status: status.into(),
        }
    }

    #[test]
    fn lists_only_the_tasks_runs_with_final_refs() {
        // repo_path is a non-git temp dir → diffs resolve to None, exercising the
        // assembly without a live git fixture (diffs are covered by gitx tests).
        let dir = tempfile::tempdir().unwrap();
        let conn = loopfleet_store::open(":memory:").unwrap();
        let pid = seed(&conn, &dir.path().to_string_lossy());

        loopfleet_store::insert_run(&conn, &run("r1", &pid, "task a", "completed")).unwrap();
        loopfleet_store::insert_run(&conn, &run("r2", &pid, "task a", "completed")).unwrap();
        // A run on a different task must not appear.
        loopfleet_store::insert_run(&conn, &run("r3", &pid, "task b", "completed")).unwrap();

        loopfleet_store::insert_iteration(&conn, "r1", 1, "refs/agentapp/run-r1/iter-1", None).unwrap();
        loopfleet_store::insert_iteration(&conn, "r1", 2, "refs/agentapp/run-r1/iter-2", None).unwrap();

        let view = compare_view(&conn, &pid, "task a").unwrap();
        assert_eq!(view.task_anchor, "task a");
        assert_eq!(view.runs.len(), 2);

        let r1 = view.runs.iter().find(|r| r.run_id == "r1").unwrap();
        assert_eq!(r1.agent, "claude");
        // The final ref is the last iteration's shadow ref.
        assert_eq!(r1.final_ref.as_deref(), Some("refs/agentapp/run-r1/iter-2"));
        assert!(r1.diff.is_none()); // repo_path is not a git repo

        // A run with no iterations has no final ref.
        let r2 = view.runs.iter().find(|r| r.run_id == "r2").unwrap();
        assert!(r2.final_ref.is_none());
    }

    #[test]
    fn empty_when_no_runs_on_the_task() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let pid = seed(&conn, "/nope");
        assert!(compare_view(&conn, &pid, "task a").unwrap().runs.is_empty());
    }
}
