//! Run and iteration persistence (PRD data model: `Run`, `Iteration`).
//!
//! A run binds to one task (Model B: one run, one task) via
//! `(plan_id, task_anchor)`. The plan view reads runs back to DERIVE each task's
//! live `TaskStatus`; acceptance is a separate flag, not a status.

use rusqlite::{params, Connection};

/// A run to persist at launch. Worktree/branch/profile/progress paths are
/// app-managed (the git actor and sandbox produce them); the run starts in
/// whatever `status` the supervisor sets (`queued` or `running`).
#[derive(Debug, Clone)]
pub struct NewRun {
    pub id: String,
    pub plan_id: String,
    pub task_anchor: String,
    pub agent: String,
    pub worktree_path: String,
    pub branch: String,
    pub sb_profile: String,
    pub progress_path: String,
    pub max_iterations: u32,
    pub status: String,
}

/// Insert a launched run.
pub fn insert_run(conn: &Connection, run: &NewRun) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO runs
           (id, plan_id, task_anchor, agent, worktree_path, branch,
            sb_profile, progress_path, max_iterations, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            run.id,
            run.plan_id,
            run.task_anchor,
            run.agent,
            run.worktree_path,
            run.branch,
            run.sb_profile,
            run.progress_path,
            run.max_iterations,
            run.status,
        ],
    )?;
    Ok(())
}

/// Advance a run's persisted status (`runs.status`). The caller validates the
/// transition via `RunState` before calling.
pub fn update_run_status(conn: &Connection, run_id: &str, status: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE runs SET status = ?2 WHERE id = ?1",
        params![run_id, status],
    )?;
    Ok(())
}

/// Record one iteration's app-owned shadow-ref snapshot.
pub fn insert_iteration(
    conn: &Connection,
    run_id: &str,
    n: u32,
    shadow_ref: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO iterations (run_id, n, shadow_ref) VALUES (?1, ?2, ?3)",
        params![run_id, n, shadow_ref],
    )?;
    Ok(())
}

/// A run's bearing on its task's derived status: just its `status` token and
/// acceptance flag, keyed by the task it is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub id: String,
    pub task_anchor: String,
    pub status: String,
    pub accepted: bool,
}

/// Every run bound to any task in `plan_id`. The plan view groups these by
/// `task_anchor` and derives each task's `TaskStatus`.
pub fn list_runs_for_plan(conn: &Connection, plan_id: &str) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_anchor, status, accepted FROM runs
         WHERE plan_id = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map([plan_id], |r| {
            Ok(RunSummary {
                id: r.get(0)?,
                task_anchor: r.get(1)?,
                status: r.get(2)?,
                accepted: r.get::<_, i64>(3)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seed a project, plan, and one task so a run's FK `(plan_id, task_anchor)`
    /// resolves.
    fn seed(conn: &Connection) -> String {
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('p','/r','prd')",
            [],
        )
        .unwrap();
        let pid = crate::plan_id("p", "PRD.md");
        crate::upsert_plan(conn, &pid, "p", "PRD.md").unwrap();
        crate::upsert_task(conn, &pid, "task a", 1, "Task A", false).unwrap();
        pid
    }

    fn new_run(id: &str, pid: &str, anchor: &str, status: &str) -> NewRun {
        NewRun {
            id: id.into(),
            plan_id: pid.into(),
            task_anchor: anchor.into(),
            agent: "claude".into(),
            worktree_path: "/wt".into(),
            branch: format!("agent/{id}"),
            sb_profile: "/prof.sb".into(),
            progress_path: "/prog/progress.md".into(),
            max_iterations: 5,
            status: status.into(),
        }
    }

    #[test]
    fn insert_then_list_by_plan() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_run(&conn, &new_run("r1", &pid, "task a", "running")).unwrap();
        let runs = list_runs_for_plan(&conn, &pid).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].task_anchor, "task a");
        assert_eq!(runs[0].status, "running");
        assert!(!runs[0].accepted);
    }

    #[test]
    fn update_status_and_record_iterations() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_run(&conn, &new_run("r1", &pid, "task a", "running")).unwrap();

        insert_iteration(&conn, "r1", 1, "refs/agentapp/run-r1/iter-1").unwrap();
        insert_iteration(&conn, "r1", 2, "refs/agentapp/run-r1/iter-2").unwrap();
        update_run_status(&conn, "r1", "completed").unwrap();

        assert_eq!(list_runs_for_plan(&conn, &pid).unwrap()[0].status, "completed");
        let iters: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM iterations WHERE run_id='r1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(iters, 2);
    }

    #[test]
    fn run_requires_an_existing_task() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        // No task "ghost" exists → the FK (plan_id, task_anchor) is violated.
        let err = insert_run(&conn, &new_run("r1", &pid, "ghost", "running"));
        assert!(err.is_err(), "run must bind to a real task");
    }
}
