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
use crate::task_binding::{MatchKind, PlanBinding};
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
    /// Runs on this plan that resolve to no task at all — their history is
    /// stranded. Surfaced rather than swallowed: the tasks they belonged to
    /// otherwise read as untouched, which is indistinguishable from real work
    /// never started.
    pub unbound_runs: usize,
    /// Runs that resolved, but not by their stored anchor matching the current
    /// one — recovered via an older text form or by position. A non-zero count
    /// means this plan's anchors have drifted from what its runs were stored
    /// against.
    pub drifted_runs: usize,
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
pub fn plan_overview(conn: &Connection, project: &Project) -> Result<Vec<PlanView>, OverviewError> {
    let convention = PlanConvention::from_token(&project.plan_convention)
        .ok_or_else(|| OverviewError::UnknownConvention(project.plan_convention.clone()))?;
    let files =
        discover_plans(Path::new(&project.repo_path), convention).map_err(OverviewError::Io)?;

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
        let hints = loopfleet_store::task_line_hints(conn, &pid).map_err(OverviewError::Store)?;

        // Resolve each run to its task once. A stored anchor that no longer
        // matches the current derivation needs the whole task list (and its own
        // last known line) to place it, not a single equality test per task.
        //
        // The binder is built from the runs' own anchors, which are the only
        // record of what this file used to say — the task rows were re-synced
        // from its current contents three lines ago and would vouch for any
        // file at all. What it decides: is this still the plan these runs were
        // stored against, or has the plan at this path been replaced? Position
        // is only admissible if it is.
        let binding = PlanBinding::new(&parsed.tasks, runs.iter().map(|r| &r.task_anchor));
        let mut bound: Vec<Vec<TaskRun>> = vec![Vec::new(); parsed.tasks.len()];
        let mut unbound_runs = 0;
        let mut drifted_runs = 0;
        for r in &runs {
            let Some(state) = RunState::from_token(&r.status) else {
                continue;
            };
            match binding.resolve(&r.task_anchor, hints.get(&r.task_anchor).copied()) {
                Some(res) => {
                    if res.kind != MatchKind::Exact || res.position_disagreed {
                        drifted_runs += 1;
                    }
                    bound[res.task_index].push(TaskRun {
                        state,
                        accepted: r.accepted,
                    });
                }
                None => unbound_runs += 1,
            }
        }

        let tasks = parsed
            .tasks
            .iter()
            .zip(bound)
            .map(|(t, task_runs)| TaskView {
                anchor: t.anchor.normalized_text.clone(),
                line_hint: t.anchor.line_hint,
                text: t.text.clone(),
                checked: t.checked,
                status: derive_status(&task_runs, t.checked),
                run_count: task_runs.len(),
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
            unbound_runs,
            drifted_runs,
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
            model: None,
            worktree_path: "/wt".into(),
            branch: "agent/x".into(),
            sb_profile: "/p.sb".into(),
            progress_path: "/prog.md".into(),
            max_iterations: 3,
            status: status.into(),
        }
    }

    #[test]
    fn a_replaced_plan_does_not_inherit_the_previous_plan_s_accepted_runs() {
        // The archive-and-start-the-next-one cycle, which is how a project at
        // the `prd` convention moves from one plan to the next: PRD.md is
        // rewritten in place, so the plan id — derived from the path — carries
        // the finished plan's runs onto a plan nobody has started.
        //
        // Every ingredient for a misbinding is present: the old task rows are
        // still stored (upsert_task never deletes) with the lines they sat on,
        // those lines land inside the new file, and no new task matches any
        // stored anchor by text. Without the per-plan gate on the positional
        // signal, the new tasks read as accepted work.
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (project, dir) =
            project_with_prd(&conn, "# Old\n- [ ] ship the widget\n- [ ] ship the gadget\n");

        let views = plan_overview(&conn, &project).unwrap();
        let plan_id = views[0].plan_id.clone();
        for anchor in ["ship the widget", "ship the gadget"] {
            let run = run_row(&plan_id, anchor, "completed");
            loopfleet_store::insert_run(&conn, &run).unwrap();
            loopfleet_store::set_run_accepted(&conn, &run.id).unwrap();
        }

        // The finished plan is archived by hand and a new one written in its
        // place. Same path, same plan id, same two lines — different plan.
        std::fs::write(
            dir.path().join("PRD.md"),
            "# New\n- [ ] add a theme picker\n- [ ] remove the brand mark\n",
        )
        .unwrap();

        let views = plan_overview(&conn, &project).unwrap();
        let v = &views[0];
        assert_eq!(v.tasks.len(), 2);
        for t in &v.tasks {
            assert_eq!(
                t.status,
                TaskStatus::NotStarted,
                "{} inherited a previous plan's run",
                t.anchor
            );
            assert_eq!(t.run_count, 0);
        }
        // The old runs are not silently dropped either: they are stranded, and
        // the count says so. Drift is for anchors that moved within a plan, not
        // for runs belonging to a plan that is gone.
        assert_eq!(v.unbound_runs, 2);
        assert_eq!(v.drifted_runs, 0);
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
    fn a_run_stored_under_an_older_anchor_still_binds_to_its_task() {
        // The regression this guards: an accepted run whose stored anchor
        // predates a change in how anchors are derived. Left unresolved, its
        // task reads NotStarted — finished work looking untouched.
        let conn = loopfleet_store::open(":memory:").unwrap();
        let prd = "# Plan\n\
                   - [ ] **Add the widget to\n  the registry.**\n  Rationale here.\n";
        let (project, _dir) = project_with_prd(&conn, prd);
        let pid = loopfleet_store::plan_id(&project.id, &format!("{}/PRD.md", project.repo_path));

        // Simulate the older sync: the anchor truncated at its first physical
        // line, which is what the run's FK points at.
        let legacy = "**add the widget to";
        loopfleet_store::upsert_plan(&conn, &pid, &project.id, "unused").unwrap();
        loopfleet_store::upsert_task(&conn, &pid, legacy, 2, "**Add the widget to", false).unwrap();
        let run = run_row(&pid, legacy, "completed");
        loopfleet_store::insert_run(&conn, &run).unwrap();
        loopfleet_store::set_run_accepted(&conn, &run.id).unwrap();

        let views = plan_overview(&conn, &project).unwrap();
        let v = &views[0];
        assert_eq!(v.tasks.len(), 1);
        let task = &v.tasks[0];
        // The current anchor is the bold span; the old run binds to it anyway.
        assert_eq!(task.anchor, "add the widget to the registry.");
        assert_eq!(task.status, TaskStatus::Accepted);
        assert_eq!(task.run_count, 1);
        // Bound, but not by its stored anchor — reported, not swallowed.
        assert_eq!(v.drifted_runs, 1);
        assert_eq!(v.unbound_runs, 0);
    }

    #[test]
    fn a_run_matching_nothing_is_counted_as_unbound() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (project, _dir) = project_with_prd(&conn, "# Plan\n- [ ] alpha\n");
        let views = plan_overview(&conn, &project).unwrap();
        let pid = views[0].plan_id.clone();

        // A task that has since vanished from the file entirely.
        loopfleet_store::upsert_task(&conn, &pid, "long gone", 400, "long gone", false).unwrap();
        loopfleet_store::insert_run(&conn, &run_row(&pid, "long gone", "completed")).unwrap();

        let v = &plan_overview(&conn, &project).unwrap()[0];
        // Neither signal reaches: the text matches nothing and line 400 is far
        // outside the positional window. The run is reported as stranded rather
        // than pinned onto the unrelated task that happens to survive.
        assert_eq!(v.unbound_runs, 1);
        assert_eq!(v.drifted_runs, 0);
        assert_eq!(v.tasks[0].run_count, 0);
        assert_eq!(v.tasks[0].status, TaskStatus::NotStarted);
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
