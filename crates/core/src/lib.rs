//! loopfleet core: supervisor, run loop, and the normalized event enum.
//!
//! This crate owns the domain logic shared across the app. M0 contributes
//! project registration (tying git-repo validation in `gitx` to persistence in
//! `store`); M1 the normalized event enum and the [`AgentAdapter`] trait; M3 the
//! supervisor foundations (run lifecycle state machine + process-group spawning)
//! and the driving [`run_loop`], which composes the adapter trait, the state
//! machine, and the serialized git actor into the ralph-style iteration loop.

use std::fmt;
use std::path::Path;

use loopfleet_store::Project;
use rusqlite::Connection;

pub mod adapter;
pub mod autopilot;
pub mod commit_message;
pub mod compare;
pub mod event;
pub mod overview;
pub mod plan;
pub mod progress;
pub mod report;
pub mod run_loop;
pub mod supervisor;
pub mod task_binding;
pub mod task_status;
pub mod timeline;
pub mod usage;
pub use adapter::{AdapterError, AgentAdapter, RunHandle, RunSpec, SessionHandle, SessionSeed};
pub use autopilot::{should_auto_merge, AutoMergeBlockedReason, AutoMergeDecision};
pub use commit_message::compose_commit_message;
pub use compare::{compare_view, CompareError, CompareView, RunCompare};
pub use event::{Lane, NormalizedEvent, Usage};
pub use overview::{plan_overview, OverviewError, PlanView, TaskView};
pub use plan::{
    anchor_for, discover_plans, legacy_anchor_for, parse_plan, parse_plan_file, ParsedPlan,
    ParsedTask, PlanConvention, TaskAnchor,
};
pub use progress::{
    contents_mark_complete, file_marks_complete, watch_for_completion, COMPLETION_MARKER,
};
pub use report::{
    plan_report, render_plan_report, render_task_report, task_report, PlanReport, ReportError,
    ReportEvent, RunReport, TaskReport,
};
pub use run_loop::{run_loop, IterationRecord, LoopConfig, LoopOutcome};
pub use supervisor::{InvalidTransition, RunProcess, RunState};
pub use task_binding::{
    is_legacy_form_of, MatchKind, PlanBinding, Resolution,
};
pub use task_status::{derive_status, TaskRun, TaskStatus};
pub use timeline::{
    run_timeline, DiffView, FileChangeView, IterationView, RunTimeline, TimelineError,
    TimelineEvent,
};
pub use usage::{
    fold_rate_limit, is_stale, launch_decision, resolve_display, resolve_display_opt,
    usage_for_run, LaunchDecision, RateLimitNotice, UsageDisplay, UsageSnapshot, UsageSource,
    UsageThresholds,
};

/// Why a project could not be registered.
#[derive(Debug)]
pub enum RegisterError {
    /// The path is not a git repository (or does not exist / is unreadable).
    NotAGitRepo,
    /// A project with this repo path is already registered.
    AlreadyRegistered,
    /// Underlying persistence failure.
    Store(rusqlite::Error),
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegisterError::NotAGitRepo => write!(f, "the selected folder is not a git repository"),
            RegisterError::AlreadyRegistered => write!(f, "this repository is already registered"),
            RegisterError::Store(e) => write!(f, "failed to persist project: {e}"),
        }
    }
}

impl std::error::Error for RegisterError {}

/// Register a folder as a project: canonicalize it, validate it is a git repo,
/// and persist it. Returns the stored `Project`. Plan convention defaults to
/// `"prd"` (PRD.md at repo root); folder-convention detection lands with M3.
pub fn register_project(conn: &Connection, folder: &Path) -> Result<Project, RegisterError> {
    // Canonicalize so the same repo can't be registered twice via different
    // relative paths. A path that can't be canonicalized can't be a repo.
    let repo_path = folder
        .canonicalize()
        .map_err(|_| RegisterError::NotAGitRepo)?;

    if !loopfleet_gitx::is_git_repo(&repo_path) {
        return Err(RegisterError::NotAGitRepo);
    }

    let project = Project {
        id: uuid::Uuid::new_v4().to_string(),
        repo_path: repo_path.to_string_lossy().into_owned(),
        plan_convention: "prd".into(),
    };

    match loopfleet_store::insert_project(conn, &project) {
        Ok(()) => Ok(project),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Err(RegisterError::AlreadyRegistered)
        }
        Err(e) => Err(RegisterError::Store(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_a_git_repo() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        let project = register_project(&conn, dir.path()).unwrap();
        assert_eq!(project.plan_convention, "prd");
        assert_eq!(loopfleet_store::list_projects(&conn).unwrap().len(), 1);
    }

    #[test]
    fn rejects_non_repo() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            register_project(&conn, dir.path()),
            Err(RegisterError::NotAGitRepo)
        ));
    }

    #[test]
    fn rejects_duplicate_registration() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        register_project(&conn, dir.path()).unwrap();
        assert!(matches!(
            register_project(&conn, dir.path()),
            Err(RegisterError::AlreadyRegistered)
        ));
    }
}
