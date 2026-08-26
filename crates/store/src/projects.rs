//! Project persistence (PRD data model: `Project { id, repo_path, plan_convention }`).

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// A registered project: a git repo the app supervises runs against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    /// Absolute, canonicalized repo path. Unique per project.
    pub repo_path: String,
    /// Plan convention: `"prd"` (PRD.md at root) or `"folder"` (plans/ dir).
    pub plan_convention: String,
}

/// Insert a project. Errors on a duplicate `repo_path` (the UNIQUE constraint),
/// which the caller maps to an "already registered" condition.
pub fn insert_project(conn: &Connection, project: &Project) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO projects (id, repo_path, plan_convention) VALUES (?1, ?2, ?3)",
        (&project.id, &project.repo_path, &project.plan_convention),
    )?;
    Ok(())
}

/// All registered projects, ordered by repo path for a stable listing.
pub fn list_projects(conn: &Connection) -> rusqlite::Result<Vec<Project>> {
    let mut stmt = conn
        .prepare("SELECT id, repo_path, plan_convention FROM projects ORDER BY repo_path")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Project {
                id: r.get(0)?,
                repo_path: r.get(1)?,
                plan_convention: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Row counts removed by [`delete_project`], one field per table touched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteProjectSummary {
    pub events: usize,
    pub runs: usize,
    pub tasks: usize,
    pub plans: usize,
    pub sessions: usize,
    pub pending_resumes: usize,
    pub scheduled_launches: usize,
    pub projects: usize,
}

/// Delete a project and everything under it, in one transaction.
///
/// Deletion order follows the FK graph (a child before whatever it
/// references): events for the project's runs, then those runs' pending
/// resumes, then the runs themselves, then the plans' scheduled launches and
/// tasks, then the plans, the sessions, and finally the project row. `runs`
/// reference `tasks` (and `scheduled_launches` reference `tasks` too) without
/// `ON DELETE CASCADE`, so both must go before `tasks` is deleted.
pub fn delete_project(conn: &Connection, project_id: &str) -> rusqlite::Result<DeleteProjectSummary> {
    conn.execute_batch("BEGIN")?;
    let result = (|| {
        let mut summary = DeleteProjectSummary::default();

        summary.events = conn.execute(
            "DELETE FROM events WHERE run_or_session_id IN
               (SELECT r.id FROM runs r JOIN plans p ON p.id = r.plan_id WHERE p.project_id = ?1)",
            params![project_id],
        )?;

        summary.pending_resumes = conn.execute(
            "DELETE FROM pending_resumes WHERE run_id IN
               (SELECT r.id FROM runs r JOIN plans p ON p.id = r.plan_id WHERE p.project_id = ?1)",
            params![project_id],
        )?;

        summary.runs = conn.execute(
            "DELETE FROM runs WHERE plan_id IN (SELECT id FROM plans WHERE project_id = ?1)",
            params![project_id],
        )?;

        summary.scheduled_launches = conn.execute(
            "DELETE FROM scheduled_launches WHERE plan_id IN (SELECT id FROM plans WHERE project_id = ?1)",
            params![project_id],
        )?;

        summary.tasks = conn.execute(
            "DELETE FROM tasks WHERE plan_id IN (SELECT id FROM plans WHERE project_id = ?1)",
            params![project_id],
        )?;

        summary.plans = conn.execute("DELETE FROM plans WHERE project_id = ?1", params![project_id])?;

        summary.sessions =
            conn.execute("DELETE FROM sessions WHERE project_id = ?1", params![project_id])?;

        summary.projects = conn.execute("DELETE FROM projects WHERE id = ?1", params![project_id])?;

        Ok(summary)
    })();

    match result {
        Ok(summary) => {
            conn.execute_batch("COMMIT")?;
            Ok(summary)
        }
        Err(e) => {
            conn.execute_batch("ROLLBACK")?;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, path: &str) -> Project {
        Project {
            id: id.into(),
            repo_path: path.into(),
            plan_convention: "prd".into(),
        }
    }

    #[test]
    fn insert_then_list_roundtrips() {
        let conn = crate::open(":memory:").unwrap();
        insert_project(&conn, &project("p1", "/repos/b")).unwrap();
        insert_project(&conn, &project("p2", "/repos/a")).unwrap();
        let got = list_projects(&conn).unwrap();
        // Ordered by repo_path.
        assert_eq!(got, vec![project("p2", "/repos/a"), project("p1", "/repos/b")]);
    }

    #[test]
    fn duplicate_repo_path_is_rejected() {
        let conn = crate::open(":memory:").unwrap();
        insert_project(&conn, &project("p1", "/repos/a")).unwrap();
        let err = insert_project(&conn, &project("p2", "/repos/a"));
        assert!(err.is_err(), "duplicate repo_path must violate UNIQUE");
    }

    /// Populates a project with one row in every table that hangs off it
    /// (plan, task, run, event, pending resume, scheduled launch, session),
    /// so `delete_project` has something real to remove.
    fn seed_full_project(conn: &Connection, project_id: &str) {
        insert_project(conn, &project(project_id, &format!("/repos/{project_id}"))).unwrap();

        let plan_id = crate::plan_id(project_id, "PRD.md");
        crate::upsert_plan(conn, &plan_id, project_id, "PRD.md").unwrap();
        crate::upsert_task(conn, &plan_id, "do the thing", 1, "Do the thing", false).unwrap();

        let run_id = format!("{project_id}-run");
        crate::insert_run(
            conn,
            &crate::NewRun {
                id: run_id.clone(),
                plan_id: plan_id.clone(),
                task_anchor: "do the thing".into(),
                agent: "claude".into(),
                model: None,
                worktree_path: "/tmp/wt".into(),
                branch: "agent/x".into(),
                sb_profile: "default".into(),
                progress_path: "/tmp/progress.md".into(),
                max_iterations: 1,
                status: "running".into(),
            },
        )
        .unwrap();

        crate::insert_event(conn, &run_id, "{}").unwrap();

        crate::insert_pending_resume(
            conn,
            &crate::NewPendingResume {
                run_id: run_id.clone(),
                task_anchor: "do the thing".into(),
                agent: "claude".into(),
                model: None,
                pass_count: 1,
                resume_at: 0,
                attempt: 1,
            },
        )
        .unwrap();

        crate::insert_scheduled_launch(
            conn,
            &crate::NewScheduledLaunch {
                plan_id: plan_id.clone(),
                task_anchor: "do the thing".into(),
                agent: "claude".into(),
                model: None,
                pass_count: 1,
                launch_at: 0,
            },
        )
        .unwrap();

        conn.execute(
            "INSERT INTO sessions (id, project_id, agent, plan_file, status)
             VALUES (?1, ?2, 'claude', 'PRD.md', 'active')",
            params![format!("{project_id}-session"), project_id],
        )
        .unwrap();
    }

    #[test]
    fn delete_project_removes_everything_and_counts_it() {
        let conn = crate::open(":memory:").unwrap();
        seed_full_project(&conn, "p1");

        let summary = delete_project(&conn, "p1").unwrap();
        assert_eq!(
            summary,
            DeleteProjectSummary {
                events: 1,
                runs: 1,
                tasks: 1,
                plans: 1,
                sessions: 1,
                pending_resumes: 1,
                scheduled_launches: 1,
                projects: 1,
            }
        );

        for table in [
            "projects", "plans", "tasks", "runs", "events", "sessions",
            "pending_resumes", "scheduled_launches", "iterations",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "table {table} should be empty after delete_project");
        }
    }

    #[test]
    fn delete_project_leaves_other_projects_untouched() {
        let conn = crate::open(":memory:").unwrap();
        seed_full_project(&conn, "p1");
        seed_full_project(&conn, "p2");

        delete_project(&conn, "p1").unwrap();

        let remaining = list_projects(&conn).unwrap();
        assert_eq!(remaining, vec![project("p2", "/repos/p2")]);

        let run_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runs WHERE id = 'p2-run'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(run_count, 1, "other project's run must survive");
    }

    #[test]
    fn delete_project_missing_id_is_a_no_op() {
        let conn = crate::open(":memory:").unwrap();
        let summary = delete_project(&conn, "nope").unwrap();
        assert_eq!(summary, DeleteProjectSummary::default());
    }
}
