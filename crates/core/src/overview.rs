//! The plan overview (M4): render a project's plan(s) with a derived
//! `TaskStatus` overlay per task.
//!
//! This is the read side of the plan-centric UI. It discovers the project's plan
//! file(s) under its convention, parses each (deterministically, no inference),
//! syncs the plan + tasks into the store so runs can bind to a stable
//! `(plan_id, task_anchor)`, then joins the parsed tasks against their runs to
//! derive each task's live `TaskStatus`. The PRD is frozen — this never edits it;
//! `markdown` is the raw file for the UI to render as-is.

use std::path::Path;

use loopfleet_store::{Connection, Project};
use serde::Serialize;

use crate::plan::{discover_plans, parse_plan, PlanConvention};
use crate::task_status::{derive_status, TaskRun, TaskStatus};
use crate::RunState;

/// One plan rendered for the overview: its identity, raw markdown (rendered
/// as-is), and tasks with their derived status.
#[derive(Debug, Serialize)]
pub struct PlanView {
    pub plan_id: String,
    pub file_path: String,
    pub title: Option<String>,
    /// The raw plan file, for the UI to render the frozen PRD verbatim.
    pub markdown: String,
    pub tasks: Vec<TaskView>,
}

/// One task with its authored fields plus the app-derived live state.
#[derive(Debug, Serialize)]
pub struct TaskView {
    /// The stable anchor identity — what a launched run binds to.
    pub anchor: String,
    pub line_hint: u32,
    pub text: String,
    /// Authored `- [x]` state: reads as "implemented" in the derived status
    /// (the `Accepted` baseline), and is still runnable — launching is never
    /// gated by it.
    pub checked: bool,
    pub status: TaskStatus,
    /// How many runs are bound to this task (context for the compare queue).
    pub run_count: usize,
}

/// Why a plan overview could not be built.
#[derive(Debug)]
pub enum OverviewError {
    /// The project's `plan_convention` token is unrecognized.
    UnknownConvention(String),
    /// Reading a plan file (or the plans dir) failed.
    Io(std::io::Error),
    /// Persisting the synced plan/tasks or reading runs failed.
    Store(rusqlite::Error),
}

impl std::fmt::Display for OverviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OverviewError::UnknownConvention(c) => write!(f, "unknown plan convention: {c}"),
            OverviewError::Io(e) => write!(f, "reading plan: {e}"),
            OverviewError::Store(e) => write!(f, "persisting plan: {e}"),
        }
    }
}

impl std::error::Error for OverviewError {}

/// Build the overview for `project`: one [`PlanView`] per discovered plan file
/// (PRD convention → 0 or 1; folder convention → one per `.md`).
pub fn plan_overview(
    conn: &Connection,
    project: &Project,
) -> Result<Vec<PlanView>, OverviewError> {
    let convention = PlanConvention::from_token(&project.plan_convention)
        .ok_or_else(|| OverviewError::UnknownConvention(project.plan_convention.clone()))?;
    let files = discover_plans(Path::new(&project.repo_path), convention).map_err(OverviewError::Io)?;

    let mut views = Vec::with_capacity(files.len());
    for file in files {
        let file_path = file.to_string_lossy().into_owned();
        let markdown = std::fs::read_to_string(&file).map_err(OverviewError::Io)?;
        let parsed = parse_plan(&markdown);

        let pid = loopfleet_store::plan_id(&project.id, &file_path);
        loopfleet_store::upsert_plan(conn, &pid, &project.id, &file_path)
            .map_err(OverviewError::Store)?;
        for t in &parsed.tasks {
            loopfleet_store::upsert_task(
                conn,
                &pid,
                &t.anchor.normalized_text,
                t.anchor.line_hint,
                &t.text,
                t.checked,
            )
            .map_err(OverviewError::Store)?;
        }

        let runs = loopfleet_store::list_runs_for_plan(conn, &pid).map_err(OverviewError::Store)?;
        let tasks = parsed
            .tasks
            .iter()
            .map(|t| {
                // Runs bound to this exact anchor, mapped to what derivation needs.
                let task_runs: Vec<TaskRun> = runs
                    .iter()
                    .filter(|r| r.task_anchor == t.anchor.normalized_text)
                    .filter_map(|r| {
                        RunState::from_token(&r.status).map(|state| TaskRun {
                            state,
                            accepted: r.accepted,
                        })
                    })
                    .collect();
                TaskView {
                    anchor: t.anchor.normalized_text.clone(),
                    line_hint: t.anchor.line_hint,
                    text: t.text.clone(),
                    checked: t.checked,
                    status: derive_status(&task_runs, t.checked),
                    run_count: task_runs.len(),
                }
            })
            .collect();
        // Implemented tasks (Accepted — either an accepted run or an authored
        // `- [x]`) sink below not-yet-implemented ones so the work left to do
        // reads first; document order is preserved within each group.
        let mut tasks: Vec<TaskView> = tasks;
        tasks.sort_by_key(|t| t.status == TaskStatus::Accepted);

        views.push(PlanView {
            plan_id: pid,
            file_path,
            title: parsed.title,
            markdown,
            tasks,
        });
    }
    Ok(views)
}

#[cfg(test)]
mod tests {
    use super::*;
    use loopfleet_store::NewRun;

    /// A project whose repo dir holds a PRD.md, registered in the store.
    fn project_with_prd(conn: &Connection, prd: &str) -> (Project, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("PRD.md"), prd).unwrap();
        let project = Project {
            id: "proj".into(),
            repo_path: dir.path().to_string_lossy().into_owned(),
            plan_convention: "prd".into(),
        };
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES (?1, ?2, ?3)",
            rusqlite::params![project.id, project.repo_path, project.plan_convention],
        )
        .unwrap();
        (project, dir)
    }

    fn run_row(pid: &str, anchor: &str, status: &str) -> NewRun {
        NewRun {
            id: format!("run-{anchor}-{status}"),
            plan_id: pid.into(),
            task_anchor: anchor.into(),
            agent: "claude".into(),
            worktree_path: "/wt".into(),
            branch: "agent/x".into(),
            sb_profile: "/p.sb".into(),
            progress_path: "/prog.md".into(),
            max_iterations: 3,
            status: status.into(),
        }
    }

    #[test]
    fn overlays_derived_status_and_syncs_tasks() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (project, _dir) =
            project_with_prd(&conn, "# Plan\n- [ ] alpha\n- [ ] beta\n- [x] gamma\n");

        // First call syncs the plan + tasks; no runs yet. alpha/beta are
        // not-started; gamma is authored-checked → Accepted (implemented).
        let views = plan_overview(&conn, &project).unwrap();
        assert_eq!(views.len(), 1);
        let v = &views[0];
        assert_eq!(v.title.as_deref(), Some("Plan"));
        assert!(v.markdown.contains("- [ ] alpha"));
        assert_eq!(v.tasks.len(), 3);
        let gamma = v.tasks.iter().find(|t| t.anchor == "gamma").unwrap();
        assert!(gamma.checked);
        assert_eq!(gamma.status, TaskStatus::Accepted);
        // Implemented tasks sink below not-yet-implemented ones: alpha, beta,
        // then gamma — even though gamma is authored-third, it's last here.
        assert_eq!(v.tasks[0].anchor, "alpha");
        assert_eq!(v.tasks[1].anchor, "beta");
        assert_eq!(v.tasks[2].anchor, "gamma");

        // A completed run on "alpha" → completed-unaccepted overlay on re-read.
        loopfleet_store::insert_run(&conn, &run_row(&v.plan_id, "alpha", "completed")).unwrap();
        loopfleet_store::insert_run(&conn, &run_row(&v.plan_id, "beta", "running")).unwrap();

        let views = plan_overview(&conn, &project).unwrap();
        let tasks = &views[0].tasks;
        let alpha = tasks.iter().find(|t| t.anchor == "alpha").unwrap();
        let beta = tasks.iter().find(|t| t.anchor == "beta").unwrap();
        let gamma = tasks.iter().find(|t| t.anchor == "gamma").unwrap();
        assert_eq!(alpha.status, TaskStatus::CompletedUnaccepted);
        assert_eq!(alpha.run_count, 1);
        assert_eq!(beta.status, TaskStatus::InProgress);
        assert_eq!(gamma.status, TaskStatus::Accepted);
        // gamma (implemented) still sinks below alpha/beta (not implemented).
        assert_eq!(tasks.last().unwrap().anchor, "gamma");
    }

    #[test]
    fn missing_prd_yields_no_plans() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let project = Project {
            id: "proj".into(),
            repo_path: dir.path().to_string_lossy().into_owned(),
            plan_convention: "prd".into(),
        };
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES (?1, ?2, ?3)",
            rusqlite::params![project.id, project.repo_path, project.plan_convention],
        )
        .unwrap();
        assert!(plan_overview(&conn, &project).unwrap().is_empty());
    }

    #[test]
    fn rejects_unknown_convention() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let project = Project {
            id: "proj".into(),
            repo_path: "/nope".into(),
            plan_convention: "bogus".into(),
        };
        assert!(matches!(
            plan_overview(&conn, &project),
            Err(OverviewError::UnknownConvention(_))
        ));
    }
}
