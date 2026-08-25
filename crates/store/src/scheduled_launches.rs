//! Scheduled launch persistence: a run the user asked for later, recorded
//! before any run exists. The sibling of `pending_resumes` — same shape, but
//! keyed to the plan task it will launch (`plan_id`, `task_anchor`) rather than
//! to an existing run, because there is nothing to resume yet. Written when the
//! schedule is set (so a crash mid-wait doesn't lose it) and deleted when it
//! fires or is cancelled.

use rusqlite::{params, Connection};

/// A launch to schedule for a plan task.
#[derive(Debug, Clone)]
pub struct NewScheduledLaunch {
    pub plan_id: String,
    /// The task's normalized text, as in `tasks.normalized_text`.
    pub task_anchor: String,
    pub agent: String,
    /// The model to launch the agent with, if any; `None` means the agent's
    /// own default.
    pub model: Option<String>,
    /// How many passes the launched run should get.
    pub pass_count: u32,
    /// When the launch should fire, unix millis.
    pub launch_at: i64,
}

/// Schedule a launch, returning its row id (the handle used to delete it).
/// A task may have several scheduled launches; the storage layer doesn't
/// restrict that.
pub fn insert_scheduled_launch(
    conn: &Connection,
    launch: &NewScheduledLaunch,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO scheduled_launches
           (plan_id, task_anchor, agent, model, pass_count, launch_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            launch.plan_id,
            launch.task_anchor,
            launch.agent,
            launch.model,
            launch.pass_count,
            launch.launch_at,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// One scheduled launch, read back for recovery/scheduling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledLaunch {
    pub id: i64,
    pub plan_id: String,
    pub task_anchor: String,
    pub agent: String,
    pub model: Option<String>,
    pub pass_count: u32,
    pub launch_at: i64,
    /// When the schedule was recorded, unix millis.
    pub created_at: i64,
    /// How many times this launch has already been pushed back because the
    /// agent was still exhausted when it fired (0 for one that has never
    /// fired yet).
    pub reschedule_count: u32,
}

/// Every scheduled launch, ordered by when it's due to fire. Called at startup
/// to reschedule anything a crash interrupted, and by the scheduler loop.
pub fn list_scheduled_launches(conn: &Connection) -> rusqlite::Result<Vec<ScheduledLaunch>> {
    let mut stmt = conn.prepare(
        "SELECT id, plan_id, task_anchor, agent, model, pass_count, launch_at, created_at, reschedule_count
         FROM scheduled_launches ORDER BY launch_at, id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ScheduledLaunch {
                id: r.get(0)?,
                plan_id: r.get(1)?,
                task_anchor: r.get(2)?,
                agent: r.get(3)?,
                model: r.get(4)?,
                pass_count: r.get(5)?,
                launch_at: r.get(6)?,
                created_at: r.get(7)?,
                reschedule_count: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Clear one scheduled launch by its row id, whether because it fired or was
/// cancelled. A no-op if none exists.
pub fn delete_scheduled_launch(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM scheduled_launches WHERE id = ?1", params![id])?;
    Ok(())
}

/// Push a scheduled launch back to `launch_at` and bump its reschedule count,
/// because a pre-fire re-check found the agent still exhausted. A no-op if the
/// row is gone (e.g. cancelled concurrently).
pub fn reschedule_launch(
    conn: &Connection,
    id: i64,
    launch_at: i64,
    reschedule_count: u32,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE scheduled_launches SET launch_at = ?1, reschedule_count = ?2 WHERE id = ?3",
        params![launch_at, reschedule_count, id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seed a project, plan, and two tasks so a scheduled launch's FK
    /// `(plan_id, task_anchor)` resolves. Returns the plan id.
    fn seed(conn: &Connection) -> String {
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('p','/r','prd')",
            [],
        )
        .unwrap();
        let pid = crate::plan_id("p", "PRD.md");
        crate::upsert_plan(conn, &pid, "p", "PRD.md").unwrap();
        crate::upsert_task(conn, &pid, "task a", 1, "Task A", false).unwrap();
        crate::upsert_task(conn, &pid, "task b", 2, "Task B", false).unwrap();
        pid
    }

    fn new_launch(plan_id: &str, anchor: &str, launch_at: i64) -> NewScheduledLaunch {
        NewScheduledLaunch {
            plan_id: plan_id.into(),
            task_anchor: anchor.into(),
            agent: "claude".into(),
            model: None,
            pass_count: 3,
            launch_at,
        }
    }

    #[test]
    fn insert_then_list_scheduled_launches() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        let id =
            insert_scheduled_launch(&conn, &new_launch(&pid, "task a", 1_700_000_000_000)).unwrap();

        let launches = list_scheduled_launches(&conn).unwrap();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].id, id);
        assert_eq!(launches[0].plan_id, pid);
        assert_eq!(launches[0].task_anchor, "task a");
        assert_eq!(launches[0].agent, "claude");
        assert_eq!(launches[0].model, None);
        assert_eq!(launches[0].pass_count, 3);
        assert_eq!(launches[0].launch_at, 1_700_000_000_000);
        assert!(
            launches[0].created_at > 0,
            "created_at is stamped on insert"
        );
        assert_eq!(launches[0].reschedule_count, 0, "a fresh launch has never been rescheduled");
    }

    #[test]
    fn reschedule_bumps_launch_at_and_count() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        let id = insert_scheduled_launch(&conn, &new_launch(&pid, "task a", 1_000)).unwrap();

        reschedule_launch(&conn, id, 5_000, 1).unwrap();

        let launches = list_scheduled_launches(&conn).unwrap();
        assert_eq!(launches[0].launch_at, 5_000);
        assert_eq!(launches[0].reschedule_count, 1);
    }

    #[test]
    fn reschedule_is_a_noop_when_absent() {
        let conn = crate::open(":memory:").unwrap();
        seed(&conn);
        reschedule_launch(&conn, 404, 5_000, 1).unwrap();
    }

    #[test]
    fn model_override_round_trips() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_scheduled_launch(
            &conn,
            &NewScheduledLaunch {
                model: Some("opus".into()),
                ..new_launch(&pid, "task a", 1_000)
            },
        )
        .unwrap();

        let launches = list_scheduled_launches(&conn).unwrap();
        assert_eq!(launches[0].model.as_deref(), Some("opus"));
    }

    #[test]
    fn list_orders_by_launch_at() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_scheduled_launch(&conn, &new_launch(&pid, "task a", 2_000)).unwrap();
        insert_scheduled_launch(&conn, &new_launch(&pid, "task b", 1_000)).unwrap();

        let launches = list_scheduled_launches(&conn).unwrap();
        assert_eq!(
            launches.iter().map(|l| l.launch_at).collect::<Vec<_>>(),
            vec![1_000, 2_000]
        );
        assert_eq!(launches[0].task_anchor, "task b");
    }

    #[test]
    fn delete_removes_only_that_row() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        let first = insert_scheduled_launch(&conn, &new_launch(&pid, "task a", 1_000)).unwrap();
        insert_scheduled_launch(&conn, &new_launch(&pid, "task b", 2_000)).unwrap();

        delete_scheduled_launch(&conn, first).unwrap();
        let launches = list_scheduled_launches(&conn).unwrap();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].task_anchor, "task b");
    }

    #[test]
    fn delete_is_a_noop_when_absent() {
        let conn = crate::open(":memory:").unwrap();
        seed(&conn);
        delete_scheduled_launch(&conn, 404).unwrap();
    }

    #[test]
    fn scheduled_launch_requires_an_existing_task() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        let err = insert_scheduled_launch(&conn, &new_launch(&pid, "ghost task", 1_000));
        assert!(err.is_err(), "scheduled launch must bind to a real task");
    }

    #[test]
    fn cascade_deletes_with_the_plan() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_scheduled_launch(&conn, &new_launch(&pid, "task a", 1_000)).unwrap();

        conn.execute("DELETE FROM plans WHERE id = ?1", params![pid])
            .unwrap();
        assert!(list_scheduled_launches(&conn).unwrap().is_empty());
    }
}
