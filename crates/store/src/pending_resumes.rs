//! Pending resume persistence: a scheduled re-launch for a run that stopped
//! short of its task. Written when a resume is scheduled (so a crash mid-wait
//! doesn't lose the schedule) and deleted when it fires or is cancelled.

use rusqlite::{params, Connection};

/// A resume to schedule for a run.
#[derive(Debug, Clone)]
pub struct NewPendingResume {
    /// The original run's id (`runs.id`) that is being resumed.
    pub run_id: String,
    pub task_anchor: String,
    pub agent: String,
    /// The model override the original run was launched with, if any; carried
    /// forward so the resumed pass keeps it.
    pub model: Option<String>,
    pub pass_count: u32,
    /// When the resume should fire, unix millis.
    pub resume_at: i64,
    /// Which resume attempt this is (1 = first resume).
    pub attempt: u32,
}

/// Schedule a resume. A run may only have one pending resume at a time; the
/// caller deletes any existing one first (or relies on `run_id`'s FK cascade
/// clearing it if the run itself is removed).
pub fn insert_pending_resume(conn: &Connection, resume: &NewPendingResume) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO pending_resumes
           (run_id, task_anchor, agent, model, pass_count, resume_at, attempt)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            resume.run_id,
            resume.task_anchor,
            resume.agent,
            resume.model,
            resume.pass_count,
            resume.resume_at,
            resume.attempt,
        ],
    )?;
    Ok(())
}

/// One scheduled resume, read back for recovery/scheduling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingResume {
    pub run_id: String,
    pub task_anchor: String,
    pub agent: String,
    pub model: Option<String>,
    pub pass_count: u32,
    pub resume_at: i64,
    pub attempt: u32,
}

/// Every pending resume, ordered by when it's due to fire. Called at startup
/// to reschedule anything a crash interrupted, and by the scheduler loop.
pub fn list_pending_resumes(conn: &Connection) -> rusqlite::Result<Vec<PendingResume>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, task_anchor, agent, model, pass_count, resume_at, attempt
         FROM pending_resumes ORDER BY resume_at",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(PendingResume {
                run_id: r.get(0)?,
                task_anchor: r.get(1)?,
                agent: r.get(2)?,
                model: r.get(3)?,
                pass_count: r.get(4)?,
                resume_at: r.get(5)?,
                attempt: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Clear the pending resume for `run_id`, whether because it fired or was
/// cancelled. A no-op if none exists.
pub fn delete_pending_resume(conn: &Connection, run_id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM pending_resumes WHERE run_id = ?1", params![run_id])?;
    Ok(())
}

/// Every pending resume for a run bound to any plan in `project_id`. Used by
/// `remove_project` to find the in-memory timer handles it must abort before
/// the project's runs are reaped.
pub fn list_pending_resumes_for_project(
    conn: &Connection,
    project_id: &str,
) -> rusqlite::Result<Vec<PendingResume>> {
    let mut stmt = conn.prepare(
        "SELECT pr.run_id, pr.task_anchor, pr.agent, pr.model, pr.pass_count, pr.resume_at, pr.attempt
         FROM pending_resumes pr
         JOIN runs r ON pr.run_id = r.id
         JOIN plans pl ON r.plan_id = pl.id
         WHERE pl.project_id = ?1",
    )?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok(PendingResume {
                run_id: r.get(0)?,
                task_anchor: r.get(1)?,
                agent: r.get(2)?,
                model: r.get(3)?,
                pass_count: r.get(4)?,
                resume_at: r.get(5)?,
                attempt: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Whether `run_id` has an outstanding scheduled resume. Reaping a run's
/// worktree would pull the ground out from under that resume, so callers must
/// check this before deleting anything.
pub fn has_pending_resume(conn: &Connection, run_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pending_resumes WHERE run_id = ?1)",
        [run_id],
        |r| r.get::<_, bool>(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seed a project, plan, task, and run so a pending resume's FK `run_id`
    /// resolves.
    fn seed(conn: &Connection) {
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('p','/r','prd')",
            [],
        )
        .unwrap();
        let pid = crate::plan_id("p", "PRD.md");
        crate::upsert_plan(conn, &pid, "p", "PRD.md").unwrap();
        crate::upsert_task(conn, &pid, "task a", 1, "Task A", false).unwrap();
        crate::insert_run(
            conn,
            &crate::NewRun {
                id: "r1".into(),
                plan_id: pid,
                task_anchor: "task a".into(),
                agent: "claude".into(),
                model: None,
                worktree_path: "/wt".into(),
                branch: "agent/r1".into(),
                sb_profile: "/prof.sb".into(),
                progress_path: "/prog/progress.md".into(),
                max_iterations: 5,
                status: "running".into(),
            },
        )
        .unwrap();
    }

    fn new_resume(run_id: &str, attempt: u32) -> NewPendingResume {
        NewPendingResume {
            run_id: run_id.into(),
            task_anchor: "task a".into(),
            agent: "claude".into(),
            model: None,
            pass_count: 3,
            resume_at: 1_700_000_000_000,
            attempt,
        }
    }

    #[test]
    fn insert_then_list_pending_resumes() {
        let conn = crate::open(":memory:").unwrap();
        seed(&conn);
        insert_pending_resume(&conn, &new_resume("r1", 1)).unwrap();

        let resumes = list_pending_resumes(&conn).unwrap();
        assert_eq!(resumes.len(), 1);
        assert_eq!(resumes[0].run_id, "r1");
        assert_eq!(resumes[0].task_anchor, "task a");
        assert_eq!(resumes[0].agent, "claude");
        assert_eq!(resumes[0].pass_count, 3);
        assert_eq!(resumes[0].resume_at, 1_700_000_000_000);
        assert_eq!(resumes[0].attempt, 1);
    }

    #[test]
    fn list_orders_by_resume_at() {
        let conn = crate::open(":memory:").unwrap();
        seed(&conn);
        insert_pending_resume(
            &conn,
            &NewPendingResume {
                resume_at: 2_000,
                ..new_resume("r1", 1)
            },
        )
        .unwrap();
        // Delete first so a second resume for the same run doesn't collide;
        // a run isn't restricted to one row at the storage layer.
        delete_pending_resume(&conn, "r1").unwrap();
        insert_pending_resume(
            &conn,
            &NewPendingResume {
                resume_at: 1_000,
                ..new_resume("r1", 2)
            },
        )
        .unwrap();

        let resumes = list_pending_resumes(&conn).unwrap();
        assert_eq!(resumes.len(), 1);
        assert_eq!(resumes[0].resume_at, 1_000);
    }

    #[test]
    fn delete_removes_the_row() {
        let conn = crate::open(":memory:").unwrap();
        seed(&conn);
        insert_pending_resume(&conn, &new_resume("r1", 1)).unwrap();
        assert_eq!(list_pending_resumes(&conn).unwrap().len(), 1);

        delete_pending_resume(&conn, "r1").unwrap();
        assert!(list_pending_resumes(&conn).unwrap().is_empty());
    }

    #[test]
    fn delete_is_a_noop_when_absent() {
        let conn = crate::open(":memory:").unwrap();
        seed(&conn);
        delete_pending_resume(&conn, "nope").unwrap();
    }

    #[test]
    fn cascade_deletes_with_run() {
        let conn = crate::open(":memory:").unwrap();
        seed(&conn);
        insert_pending_resume(&conn, &new_resume("r1", 1)).unwrap();

        conn.execute("DELETE FROM runs WHERE id = 'r1'", []).unwrap();
        assert!(list_pending_resumes(&conn).unwrap().is_empty());
    }

    #[test]
    fn list_for_project_is_scoped_to_that_projects_runs() {
        let conn = crate::open(":memory:").unwrap();
        seed(&conn);
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('p2','/r2','prd')",
            [],
        )
        .unwrap();
        let pid2 = crate::plan_id("p2", "PRD.md");
        crate::upsert_plan(&conn, &pid2, "p2", "PRD.md").unwrap();
        crate::upsert_task(&conn, &pid2, "task b", 1, "Task B", false).unwrap();
        crate::insert_run(
            &conn,
            &crate::NewRun {
                id: "r2".into(),
                plan_id: pid2,
                task_anchor: "task b".into(),
                agent: "claude".into(),
                model: None,
                worktree_path: "/wt2".into(),
                branch: "agent/r2".into(),
                sb_profile: "/prof2.sb".into(),
                progress_path: "/prog2/progress.md".into(),
                max_iterations: 5,
                status: "running".into(),
            },
        )
        .unwrap();

        insert_pending_resume(&conn, &new_resume("r1", 1)).unwrap();
        insert_pending_resume(
            &conn,
            &NewPendingResume { run_id: "r2".into(), ..new_resume("r2", 1) },
        )
        .unwrap();

        let for_p1 = list_pending_resumes_for_project(&conn, "p").unwrap();
        assert_eq!(for_p1.len(), 1);
        assert_eq!(for_p1[0].run_id, "r1");

        let for_p2 = list_pending_resumes_for_project(&conn, "p2").unwrap();
        assert_eq!(for_p2.len(), 1);
        assert_eq!(for_p2[0].run_id, "r2");
    }

    #[test]
    fn pending_resume_requires_an_existing_run() {
        let conn = crate::open(":memory:").unwrap();
        seed(&conn);
        let err = insert_pending_resume(&conn, &new_resume("ghost", 1));
        assert!(err.is_err(), "pending resume must bind to a real run");
    }
}
