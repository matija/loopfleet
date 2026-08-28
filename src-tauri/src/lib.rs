use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use loopfleet_adapters::{ClaudeAdapter, CursorAdapter, PiAdapter};
use loopfleet_core::{
    fold_rate_limit, launch_decision, resolve_display, run_loop, should_auto_merge, AgentAdapter,
    AutoMergeBlockedReason, AutoMergeDecision, CompareView, LaunchDecision, LoopConfig,
    NormalizedEvent, PlanView, RateLimitNotice, RunSpec, RunState, RunTimeline, UsageDisplay,
    UsageSnapshot, UsageSource, UsageThresholds,
};
use loopfleet_gitx::GitActor;
use loopfleet_sandbox::{confine_prefix, RenderParams};
use loopfleet_store::{Connection, NewRun, Project, RunSummary};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::{NotificationExt, PermissionState};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::sync::watch;

mod path_env;

/// The future returned by [`spawn_run`]. Boxed and type-erased so a rate-limited
/// run can schedule another `spawn_run` from inside its own completion handler
/// without the recursion making the future infinitely sized.
type RunFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>;

/// App-owned state shared across commands. The connection is behind
/// `Arc<Mutex<…>>` so a background launch task can persist run progress on the
/// same single writer the commands use (SQLite is single-writer by design). The
/// git actor serializes all mutating git ops; `data_dir` roots the app-managed
/// worktrees, progress files, and sandbox profiles. `stops` holds a cancel
/// sender per active run so the live-run Stop button can signal it. `edits`
/// holds AI plan edits proposed but not yet accepted/discarded, keyed by
/// `edit_id`, so `plan_edit_apply`/`plan_edit_discard` can find the scratch
/// worktree to write from or clean up.
struct AppState {
    db: Arc<Mutex<Connection>>,
    git: GitActor,
    data_dir: PathBuf,
    stops: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    edits: Arc<Mutex<HashMap<String, PendingEdit>>>,
    /// Runs that reached a terminal state while the main window was unfocused
    /// and haven't been seen since — mirrored onto the dock badge, cleared when
    /// the window regains focus.
    unacknowledged_runs: Arc<AtomicI64>,
    /// Handle to a pending rate-limit re-run's sleep-then-relaunch task, keyed
    /// by the original run id, so `cancel_scheduled_resume` can abort it before
    /// it fires.
    scheduled_resumes: Arc<Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>>,
    /// Handle to a user-scheduled launch's sleep-then-launch task, keyed by the
    /// `scheduled_launches` row id, so `cancel_scheduled_launch` can abort it
    /// before it fires.
    scheduled_launches: Arc<Mutex<HashMap<i64, tauri::async_runtime::JoinHandle<()>>>>,
    /// Handle to a run's armed auto-merge countdown, keyed by run id, so a
    /// future cancel path can abort it before it fires. Armed when a run
    /// reaches a terminal state and [`should_auto_merge`] says to.
    scheduled_auto_merges: Arc<Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>>,
    /// The last usage snapshot published to the UI for each agent, keyed by
    /// agent key. Purely a de-duplication ledger for the `agent_usage` event:
    /// a snapshot is emitted only when it says something different from what
    /// the UI was last told, so a surface can listen instead of polling
    /// without being woken by every re-probe of unchanged headroom.
    published_usage: Arc<Mutex<HashMap<String, UsageSnapshot>>>,
}

/// A live run event pushed to the UI as it happens: the run it belongs to, its
/// `seq` in the run's event log, and the normalized event payload (the same
/// `{"kind":…}` shape the timeline renders).
#[derive(Clone, serde::Serialize)]
struct RunEventPayload {
    run_id: String,
    seq: i64,
    event: serde_json::Value,
}

/// A run reaching a terminal state, pushed to the UI so the live view can update
/// its status and disable the Stop button.
#[derive(Clone, serde::Serialize)]
struct RunStatusPayload {
    run_id: String,
    status: String,
}

/// A rate-limited run's re-run has been scheduled, pushed to the UI so it can
/// show when the original run will resume (and offer to cancel it).
#[derive(Clone, serde::Serialize)]
struct ScheduledResumePayload {
    run_id: String,
    resume_at: String,
}

/// A previously scheduled re-run was cancelled before it fired.
#[derive(Clone, serde::Serialize)]
struct ScheduledResumeCancelledPayload {
    run_id: String,
}

/// A launch has been scheduled for later, pushed to the UI so it can show when
/// the task will launch (and offer to cancel it). `launch_at` is RFC 3339.
#[derive(Clone, serde::Serialize)]
struct ScheduledLaunchPayload {
    id: i64,
    plan_id: String,
    task_anchor: String,
    launch_at: String,
    /// What scheduled this launch: `"manual"` or `"auto_advance"`.
    origin: String,
}

/// A scheduled launch fired, pushed with the run id it produced.
#[derive(Clone, serde::Serialize)]
struct ScheduledLaunchFiredPayload {
    id: i64,
    run_id: String,
}

/// A previously scheduled launch was cancelled before it fired.
#[derive(Clone, serde::Serialize)]
struct ScheduledLaunchCancelledPayload {
    id: i64,
}

/// A run's auto-merge countdown has armed, pushed to the UI so it can show
/// when the merge will fire (and offer to cancel it). `target_branch` is
/// empty for the repo's currently checked-out branch (see [`use_run`]).
/// `merge_at` is RFC 3339.
#[derive(Clone, serde::Serialize)]
struct AutoMergeArmedPayload {
    run_id: String,
    task_anchor: String,
    target_branch: String,
    merge_at: String,
}

/// A previously armed auto-merge countdown was cancelled before it fired. The
/// run stays in its terminal state, unaccepted, until the user acts on it.
#[derive(Clone, serde::Serialize)]
struct AutoMergeCancelledPayload {
    run_id: String,
}

/// A fired auto-merge attempt failed — a dirty-tree refusal, a conflict, or
/// any other error from [`merge_and_accept_run`]. The run stays in its
/// terminal state, unaccepted; nothing else in the chain (e.g. a resume) is
/// scheduled off the back of it, so it's left for the user to resolve.
#[derive(Clone, serde::Serialize)]
struct AutoMergeFailedPayload {
    run_id: String,
    reason: String,
}

/// Persist one event to the run's log and push it to the live UI. Returns the
/// event's `seq` (its `rowid`), captured under the same lock as the insert so it
/// is that event's even though other writers share the connection.
fn record_event(
    db: &Mutex<Connection>,
    app: &AppHandle,
    run_id: &str,
    ev: &NormalizedEvent,
) -> Option<i64> {
    let json = serde_json::to_string(ev).ok()?;
    let seq = {
        let conn = db.lock().ok()?;
        loopfleet_store::insert_event(&conn, run_id, &json).ok()?;
        conn.last_insert_rowid()
    };
    let event = serde_json::from_str(&json).unwrap_or(serde_json::Value::Null);
    let _ = app.emit(
        "run_event",
        RunEventPayload {
            run_id: run_id.to_string(),
            seq,
            event,
        },
    );
    Some(seq)
}

/// Persist the latest rate-limit observation for `agent`, overwriting whatever
/// was recorded before: the snapshot is the agent's current headroom, so only
/// the most recent one is meaningful. Best-effort — a poisoned lock or a write
/// error must not take down the run that saw the limit.
///
/// The observation is also folded into the agent's normalized snapshot and
/// published (see [`publish_usage`]), so a limit seen mid-run reaches every open
/// surface immediately rather than on their next `agent_usage` call.
fn record_agent_limit(
    db: &Mutex<Connection>,
    app: &AppHandle,
    agent: &str,
    reset_at: Option<&str>,
    message: Option<&str>,
) {
    let observed_at = now_ms();
    if let Ok(conn) = db.lock() {
        let _ = loopfleet_store::record_agent_usage(&conn, agent, reset_at, message, observed_at);
    }

    // Folded over what the UI was last told, so a terse notice ("limited", no
    // reset time) keeps the labels a richer earlier one established.
    let prior = published_snapshot(app, agent);
    let notice = rate_limit_notice(agent, reset_at, observed_at);
    publish_usage(app, fold_rate_limit(prior.as_ref(), &notice));
}

/// Now, in epoch millis — the instant vocabulary `core::usage` and the store's
/// `observed_at` both speak.
fn now_ms() -> i64 {
    (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// Parse an agent-supplied RFC 3339 reset time into epoch millis. Agents write
/// this string themselves, so an unparseable one is expected rather than
/// exceptional: it degrades to "limited, reset time unknown".
fn reset_at_ms(reset_at: Option<&str>) -> Option<i64> {
    let parsed = OffsetDateTime::parse(reset_at?, &Rfc3339).ok()?;
    Some((parsed.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// The `RateLimitNotice` a stored/observed rate limit amounts to. Our agents
/// report the fact of a limit, never a fraction, so `used_fraction` stays `None`
/// and `fold_rate_limit` reads it as inferred-exhausted.
fn rate_limit_notice(agent: &str, reset_at: Option<&str>, observed_at_ms: i64) -> RateLimitNotice {
    let notice = RateLimitNotice::new(agent, observed_at_ms);
    match reset_at_ms(reset_at) {
        Some(ms) => notice.with_reset_at(ms),
        None => notice,
    }
}

/// The snapshot the UI was last told about `agent`, if any.
fn published_snapshot(app: &AppHandle, agent: &str) -> Option<UsageSnapshot> {
    let state = app.state::<AppState>();
    let published = state.published_usage.lock().ok()?;
    published.get(agent).cloned()
}

/// Whether two snapshots for the same agent say anything different.
///
/// `observed_at_ms` is deliberately excluded: every probe restamps it, and
/// "we asked again and got the same answer" is not news the UI needs waking
/// for. Everything a surface actually renders is compared.
fn usage_changed(before: &UsageSnapshot, after: &UsageSnapshot) -> bool {
    before.agent_key != after.agent_key
        || before.model != after.model
        || before.limit_window != after.limit_window
        || before.used_fraction != after.used_fraction
        || before.reset_at_ms != after.reset_at_ms
        || before.source != after.source
}

/// Record `snapshot` as the agent's current published state and, when it
/// differs from what the UI was last told, emit it on the `agent_usage` event.
/// Returns the snapshot so callers can hand it straight back to a command.
///
/// Best-effort on the lock: a poisoned ledger costs de-duplication, never the
/// answer itself.
fn publish_usage(app: &AppHandle, snapshot: UsageSnapshot) -> UsageSnapshot {
    let state = app.state::<AppState>();
    let changed = match state.published_usage.lock() {
        Ok(mut published) => {
            let changed = published
                .get(&snapshot.agent_key)
                .map(|prior| usage_changed(prior, &snapshot))
                .unwrap_or(true);
            published.insert(snapshot.agent_key.clone(), snapshot.clone());
            changed
        }
        Err(_) => true,
    };
    if changed {
        let _ = app.emit("agent_usage", snapshot.clone());
    }
    snapshot
}

/// Validate `path` is a git repo and persist it as a project.
#[tauri::command]
fn register_project(path: String, state: State<'_, AppState>) -> Result<Project, String> {
    let conn = state.db.lock().unwrap();
    loopfleet_core::register_project(&conn, std::path::Path::new(&path)).map_err(|e| e.to_string())
}

/// All registered projects.
#[tauri::command]
fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    let conn = state.db.lock().unwrap();
    loopfleet_store::list_projects(&conn).map_err(|e| e.to_string())
}

/// A project was removed entirely, pushed so any open surface (plan view, run
/// timeline, …) showing it drops it rather than pointing at deleted rows.
#[derive(Clone, serde::Serialize)]
struct ProjectRemovedPayload {
    project_id: String,
}

/// Remove a project and everything under it.
///
/// Refuses (without touching anything) while any of the project's runs are
/// still `queued`/`running` — an active run's background task, cancel
/// channel, and worktree would be orphaned out from under it.
///
/// Otherwise: aborts and deletes every pending resume and scheduled launch
/// bound to the project — both their in-memory timer handles (so a stale
/// sleep-then-relaunch/sleep-then-launch task can't fire into a project that
/// no longer exists) and their persisted rows — then reaps each of the
/// project's runs' worktrees (same path as `reap_run`: sandbox profile and
/// progress dir included, and it defers rather than deletes out from under a
/// worktree some live process still has open) and deletes each run's
/// `agent/<run-id>` branch. Worktree/branch cleanup is best-effort per run —
/// logged, not fatal — so one stuck run's checkout can't block the rest of
/// the removal. Finally deletes the project's rows via the store's
/// `delete_project` and emits `project_removed`.
#[tauri::command]
async fn remove_project(project_id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let repo_path = {
        let conn = state.db.lock().unwrap();
        let project = get_project(&conn, &project_id)?;
        if loopfleet_store::has_active_runs_for_project(&conn, &project_id).map_err(|e| e.to_string())? {
            return Err(format!(
                "cannot remove project {project_id}: it has runs still queued or running"
            ));
        }
        project.repo_path
    };
    let repo_path = PathBuf::from(repo_path);

    let (resumes, launches, runs) = {
        let conn = state.db.lock().unwrap();
        (
            loopfleet_store::list_pending_resumes_for_project(&conn, &project_id)
                .map_err(|e| e.to_string())?,
            loopfleet_store::list_scheduled_launches_for_project(&conn, &project_id)
                .map_err(|e| e.to_string())?,
            loopfleet_store::list_runs_for_project(&conn, &project_id).map_err(|e| e.to_string())?,
        )
    };

    for resume in &resumes {
        if let Some(handle) = state.scheduled_resumes.lock().unwrap().remove(&resume.run_id) {
            handle.abort();
        }
        let conn = state.db.lock().unwrap();
        let _ = loopfleet_store::delete_pending_resume(&conn, &resume.run_id);
    }

    for launch in &launches {
        if let Some(handle) = state.scheduled_launches.lock().unwrap().remove(&launch.id) {
            handle.abort();
        }
        let conn = state.db.lock().unwrap();
        let _ = loopfleet_store::delete_scheduled_launch(&conn, launch.id);
    }

    for run in &runs {
        if let Err(e) = reap_run(&state.db, &state.git, &state.data_dir, &run.id).await {
            eprintln!("remove_project: failed to reap run {}: {e}", run.id);
        }
        let branch = loopfleet_gitx::worktree::branch_for(&run.id);
        if let Err(e) = state.git.delete_branch(repo_path.clone(), branch).await {
            eprintln!("remove_project: failed to delete branch for run {}: {e}", run.id);
        }
    }

    {
        let conn = state.db.lock().unwrap();
        loopfleet_store::delete_project(&conn, &project_id).map_err(|e| e.to_string())?;
    }

    let _ = app.emit("project_removed", ProjectRemovedPayload { project_id });

    Ok(())
}

/// Counts a removal confirmation dialog shows for a project — plans, runs,
/// active runs, and worktrees still on disk — gathered without deleting
/// anything.
#[tauri::command]
fn project_removal_preview(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<loopfleet_store::ProjectRemovalPreview, String> {
    let conn = state.db.lock().unwrap();
    loopfleet_store::project_removal_preview(&conn, &project_id).map_err(|e| e.to_string())
}

/// The global app settings (default agent, default iteration count, concurrency
/// cap, worktree retention). Unset fields fall back to code defaults.
#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<loopfleet_store::Settings, String> {
    let conn = state.db.lock().unwrap();
    loopfleet_store::load_settings(&conn).map_err(|e| e.to_string())
}

/// Persist the global app settings.
#[tauri::command]
fn save_settings(
    settings: loopfleet_store::Settings,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    loopfleet_store::save_settings(&conn, &settings).map_err(|e| e.to_string())
}

/// A project's sandbox write overrides (extra absolute paths granted per run).
#[tauri::command]
fn project_sandbox_writes(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let conn = state.db.lock().unwrap();
    loopfleet_store::project_sandbox_writes(&conn, &project_id).map_err(|e| e.to_string())
}

/// Replace a project's sandbox write overrides. Each path must be absolute (the
/// Seatbelt boundary needs absolute subpaths); relative entries are rejected so
/// a bad override never silently widens or breaks the boundary.
#[tauri::command]
fn set_project_sandbox_writes(
    project_id: String,
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    for p in &paths {
        let p = p.trim();
        if !p.is_empty() && !std::path::Path::new(p).is_absolute() {
            return Err(format!("sandbox write path must be absolute: {p}"));
        }
    }
    let conn = state.db.lock().unwrap();
    loopfleet_store::set_project_sandbox_writes(&conn, &project_id, &paths)
        .map_err(|e| e.to_string())
}

/// The plan overview for a project: its plan(s) with a derived `TaskStatus`
/// overlay per task. Syncs plan + tasks into the store as a side effect (so runs
/// can bind to them); never edits the frozen plan file.
#[tauri::command]
fn plan_overview(project_id: String, state: State<'_, AppState>) -> Result<Vec<PlanView>, String> {
    let conn = state.db.lock().unwrap();
    let project = get_project(&conn, &project_id)?;
    loopfleet_core::plan_overview(&conn, &project).map_err(|e| e.to_string())
}

/// The raw markdown of a single plan document, resolved by `plan_id`. Read-only:
/// unlike `plan_overview` it neither parses tasks nor syncs anything into the
/// store — it just reads the frozen plan file recorded for the plan and returns
/// it verbatim, for the UI to render the full PRD on demand.
#[tauri::command]
fn plan_document(plan_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let file_path = {
        let conn = state.db.lock().unwrap();
        loopfleet_store::plan_file_path(&conn, &plan_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("unknown plan: {plan_id}"))?
    };
    std::fs::read_to_string(&file_path).map_err(|e| format!("reading plan {file_path}: {e}"))
}

/// A proposed AI edit to a plan document, returned by `plan_edit`. The default
/// agent ran a single pass in an isolated worktree against the PRD; the UI
/// renders `original` vs `proposed` as a reviewable diff and lands or drops it
/// through `plan_edit_apply` / `plan_edit_discard`. `edit_id` keys the pending
/// scratch worktree so those follow-ups can find it.
#[derive(serde::Serialize)]
struct PlanEditProposal {
    edit_id: String,
    agent: String,
    path: String,
    original: String,
    proposed: String,
}

/// An AI plan edit proposed but not yet accepted/discarded: the scratch worktree
/// to clean up, and what to write where on accept. `original` is the real file's
/// content at proposal time, so accept can refuse to clobber a since-changed
/// source.
struct PendingEdit {
    repo_path: PathBuf,
    worktree_path: PathBuf,
    file_path: PathBuf,
    original: String,
    proposed: String,
}

/// Run one AI pass over a plan document and return the proposed edit for review.
/// Given `plan_id` and a free-text `instruction`, this resolves the plan file,
/// its owning repo, and the project's default agent; cuts a fresh isolated
/// worktree (sandboxed exactly as a normal run); seeds the agent with the
/// instruction plus the current PRD, asking it to edit the file in place; waits
/// for the single pass to finish; and returns `{ edit_id, agent, path, original,
/// proposed }`. No looping, no progress file. Nothing is written to the real PRD
/// here — the edit lands only through `plan_edit_apply`; until then the worktree
/// stays alive, keyed by `edit_id`.
///
/// Explicit failures (never panics): no default agent installed, the agent
/// process failing, or an unreadable result all surface as `Err`.
#[tauri::command]
async fn plan_edit(
    plan_id: String,
    instruction: String,
    state: State<'_, AppState>,
) -> Result<PlanEditProposal, String> {
    // Resolve the plan file, its owning repo, and the configured default agent.
    let (file_path, repo_path, agent) = {
        let conn = state.db.lock().unwrap();
        let (project_id, file_path): (String, String) = conn
            .query_row(
                "SELECT project_id, file_path FROM plans WHERE id = ?1",
                [&plan_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| format!("unknown plan: {plan_id}"))?;
        let repo_path = get_project(&conn, &project_id)?.repo_path;
        let agent = loopfleet_store::load_settings(&conn)
            .map_err(|e| e.to_string())?
            .default_agent;
        (file_path, repo_path, agent)
    };

    let adapter = build_adapter(&agent)
        .ok_or_else(|| format!("no default agent to edit with: unknown agent '{agent}'"))?;

    // Fail fast if the default agent's CLI isn't installed, before cutting a
    // worktree (mirrors `launch_run`; the affordance is meant to be disabled in
    // this case, but never trust the UI to have gated it).
    if let Some(spec) = loopfleet_adapters::spec_for(&agent) {
        let status = loopfleet_adapters::discover(spec).await;
        if !status.installed {
            return Err(status
                .detail
                .unwrap_or_else(|| format!("{} CLI is not available", spec.display)));
        }
    }

    // The plan file's path relative to its repo — where it lives in the worktree.
    let rel = std::path::Path::new(&file_path)
        .strip_prefix(&repo_path)
        .map_err(|_| format!("plan file {file_path} is not inside repo {repo_path}"))?
        .to_path_buf();

    let original = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("reading plan {file_path}: {e}"))?;

    // App-managed scratch, keyed by edit id (outside the repo). The worktree is a
    // fresh checkout the agent edits in isolation; the profile dir is the sandbox
    // write grant the pass needs beyond the worktree.
    let edit_id = uuid::Uuid::new_v4().to_string();
    let worktrees_root = state.data_dir.join("worktrees");
    let edit_dir = state.data_dir.join("edits").join(&edit_id);
    let profile_path = state.data_dir.join("profiles").join(format!("{edit_id}.sb"));
    std::fs::create_dir_all(&worktrees_root).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&edit_dir).map_err(|e| e.to_string())?;

    let worktree = state
        .git
        .worktree_add(
            PathBuf::from(&repo_path),
            worktrees_root,
            edit_id.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;

    // Confine writes to the worktree (+ edit dir, agent config, temp), exactly as
    // a normal run is confined.
    let mut params = RenderParams::new(&worktree.path, &edit_dir);
    params.agent_dirs = agent_dirs();
    let wrapper = confine_prefix(&params, &profile_path).map_err(|e| e.to_string())?;

    let prompt = format!(
        "{instruction}\n\nEdit the plan document at `{rel}` in this repository so it \
satisfies the instruction above, writing the full edited document back to that \
file. Change only that file.\n\n--- current {rel} ---\n{original}",
        rel = rel.display(),
    );

    let spec = RunSpec {
        cwd: worktree.path.clone(),
        prompt,
        wrapper,
        model: None,
    };

    // Drive the single pass to completion, watching for an explicit failure. On
    // any failure the scratch worktree is dropped before returning so a failed
    // edit leaves nothing behind.
    let mut handle = match adapter.start_run(&spec).await {
        Ok(h) => h,
        Err(e) => {
            let _ = state
                .git
                .worktree_remove(PathBuf::from(&repo_path), worktree.path.clone())
                .await;
            return Err(e.to_string());
        }
    };
    let mut failure: Option<String> = None;
    while let Some(ev) = handle.events.recv().await {
        if let NormalizedEvent::Failed { reason } = ev {
            failure = Some(reason);
        }
    }
    if let Some(reason) = failure {
        let _ = state
            .git
            .worktree_remove(PathBuf::from(&repo_path), worktree.path.clone())
            .await;
        return Err(format!("the {agent} edit pass failed: {reason}"));
    }

    // Read what the agent produced. Same relative path, inside the worktree.
    let proposed = std::fs::read_to_string(worktree.path.join(&rel))
        .map_err(|e| format!("reading the edited plan: {e}"))?;

    state.edits.lock().unwrap().insert(
        edit_id.clone(),
        PendingEdit {
            repo_path: PathBuf::from(&repo_path),
            worktree_path: worktree.path.clone(),
            file_path: PathBuf::from(&file_path),
            original: original.clone(),
            proposed: proposed.clone(),
        },
    );

    Ok(PlanEditProposal {
        edit_id,
        agent,
        path: file_path,
        original,
        proposed,
    })
}

/// Accept a proposed AI plan edit: write the proposed markdown to the real PRD
/// file and drop the scratch worktree. Idempotent against double-accept (an
/// unknown/already-resolved `edit_id` is an error, not a panic) and safe against
/// a since-changed source — if the file on disk no longer matches what was
/// proposed against, it refuses rather than clobbering, keeping the edit pending
/// so the user can discard and retry.
#[tauri::command]
async fn plan_edit_apply(edit_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let pending = state
        .edits
        .lock()
        .unwrap()
        .remove(&edit_id)
        .ok_or_else(|| format!("unknown or already-resolved edit: {edit_id}"))?;

    let current = std::fs::read_to_string(&pending.file_path)
        .map_err(|e| format!("reading plan {}: {e}", pending.file_path.display()))?;
    if current != pending.original {
        // Someone changed the file since the edit was proposed. Keep it pending
        // so the user can discard and re-run rather than lose their scratch.
        state.edits.lock().unwrap().insert(edit_id, pending);
        return Err(
            "the plan changed on disk since this edit was proposed — discard and re-run".into(),
        );
    }

    std::fs::write(&pending.file_path, &pending.proposed)
        .map_err(|e| format!("writing plan {}: {e}", pending.file_path.display()))?;
    let _ = state
        .git
        .worktree_remove(pending.repo_path, pending.worktree_path)
        .await;
    Ok(())
}

/// Discard a proposed AI plan edit: drop the scratch worktree, writing nothing.
/// Idempotent — an unknown/already-resolved `edit_id` is a no-op.
#[tauri::command]
async fn plan_edit_discard(edit_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let pending = state.edits.lock().unwrap().remove(&edit_id);
    if let Some(pending) = pending {
        let _ = state
            .git
            .worktree_remove(pending.repo_path, pending.worktree_path)
            .await;
    }
    Ok(())
}

/// Launch `max_iterations` looping passes of `agent` against the task anchored at
/// `task_anchor` in the given project's plan, confined by a rendered Seatbelt
/// profile. Returns the new run id immediately; the loop runs in the background
/// and its progress is persisted to the store (status, iterations, events) and
/// streamed live to the UI (`run_event`/`run_status` Tauri events).
#[tauri::command]
async fn launch_run(
    project_id: String,
    task_anchor: String,
    agent: String,
    model: Option<String>,
    max_iterations: u32,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    // The command is a thin wrapper over `spawn_run`, which owns clones of the
    // shared state so a scheduled re-run (rate limits) can call it again.
    spawn_run(
        project_id,
        task_anchor,
        agent,
        model,
        max_iterations,
        app,
        state.db.clone(),
        state.git.clone(),
        state.data_dir.clone(),
        state.stops.clone(),
        state.unacknowledged_runs.clone(),
        state.scheduled_resumes.clone(),
        state.scheduled_auto_merges.clone(),
        0,
    )
    .await
}

/// Cut a worktree, insert a run row, and drive the looping run in the background
/// (see [`launch_run`]). Takes owned clones of the shared app state rather than a
/// Tauri `State`, so it can be called both from the `launch_run` command and from
/// a scheduled re-run after a rate limit. Returns a type-erased [`RunFuture`] so
/// that self-rescheduling doesn't make the future infinitely sized.
#[allow(clippy::too_many_arguments)]
fn spawn_run(
    project_id: String,
    task_anchor: String,
    agent: String,
    model: Option<String>,
    max_iterations: u32,
    app: AppHandle,
    db: Arc<Mutex<Connection>>,
    git: GitActor,
    data_dir: PathBuf,
    stops: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    unacknowledged: Arc<AtomicI64>,
    scheduled_resumes: Arc<Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>>,
    scheduled_auto_merges: Arc<Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>>,
    // How many prior resumes led to this run: 0 for a fresh manual launch,
    // otherwise the attempt number of the pending resume that spawned it.
    // Carried forward (and capped at `MAX_RESUME_ATTEMPTS`) so a chain of
    // rate-limited resumes for the same original run eventually gives up.
    resume_attempt: u32,
) -> RunFuture {
    Box::pin(async move {
    let adapter = build_adapter(&agent).ok_or_else(|| format!("unknown agent: {agent}"))?;

    // Fail fast if the agent CLI isn't installed, before cutting a worktree or
    // inserting a run record — otherwise the run would spawn, die mid-loop, and
    // leave an orphan worktree behind (M6: graceful errors when a CLI is missing).
    if let Some(spec) = loopfleet_adapters::spec_for(&agent) {
        let status = loopfleet_adapters::discover(spec).await;
        if !status.installed {
            return Err(status
                .detail
                .unwrap_or_else(|| format!("{} CLI is not available", spec.display)));
        }
    }

    // Resolve the bound task's text and stable plan id. plan_overview also syncs
    // the plan + tasks into the store, so the run's FK resolves on insert. Also
    // enforce the concurrency cap (M6 settings) and read the project's sandbox
    // write overrides — all under one lock.
    let (project, plan_id, task_text, extra_writes) = {
        let conn = db.lock().unwrap();

        let settings = loopfleet_store::load_settings(&conn).map_err(|e| e.to_string())?;
        if settings.concurrency_cap > 0 {
            let active = loopfleet_store::count_active_runs(&conn).map_err(|e| e.to_string())?;
            if active >= settings.concurrency_cap {
                return Err(format!(
                    "concurrency cap reached ({active}/{}); stop a run or raise the cap in Settings",
                    settings.concurrency_cap
                ));
            }
        }

        let project = get_project(&conn, &project_id)?;
        let views = loopfleet_core::plan_overview(&conn, &project).map_err(|e| e.to_string())?;
        let (plan_id, task_text) = views
            .iter()
            .find_map(|v| {
                v.tasks
                    .iter()
                    .find(|t| t.anchor == task_anchor)
                    .map(|t| (v.plan_id.clone(), t.text.clone()))
            })
            .ok_or_else(|| format!("no task anchored at '{task_anchor}'"))?;
        let extra_writes = loopfleet_store::project_sandbox_writes(&conn, &project_id)
            .map_err(|e| e.to_string())?;
        (project, plan_id, task_text, extra_writes)
    };

    // App-managed paths, keyed by run id (outside the repo).
    let run_id = uuid::Uuid::new_v4().to_string();
    let worktrees_root = data_dir.join("worktrees");
    let progress_dir = data_dir.join("progress").join(&run_id);
    let progress_path = progress_dir.join("progress.md");
    let profile_path = data_dir.join("profiles").join(format!("{run_id}.sb"));
    std::fs::create_dir_all(&worktrees_root).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&progress_dir).map_err(|e| e.to_string())?;

    // Cut the per-run worktree through the serialized git actor.
    let worktree = git
        .worktree_add(
            PathBuf::from(&project.repo_path),
            worktrees_root,
            run_id.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;

    // Render the Seatbelt boundary and turn it into the opaque wrapper prefix the
    // adapter prepends — writes confined to the worktree + progress dir + agent
    // config dirs + temp.
    let mut params = RenderParams::new(&worktree.path, &progress_dir);
    params.agent_dirs = agent_dirs();
    params.extra_writes = extra_writes.into_iter().map(PathBuf::from).collect();
    let wrapper = confine_prefix(&params, &profile_path).map_err(|e| e.to_string())?;

    // Keep the launch inputs for a possible rate-limit re-run (`task_anchor` and
    // `agent` are moved into the run row just below).
    let rerun = (project_id, task_anchor.clone(), agent.clone(), model.clone(), max_iterations);
    // Rate limits are the agent's, not the run's, so the headroom snapshot this
    // run observes is filed under the agent name (see `record_agent_limit`).
    let ev_agent = agent.clone();

    {
        let conn = db.lock().unwrap();
        loopfleet_store::insert_run(
            &conn,
            &NewRun {
                id: run_id.clone(),
                plan_id,
                task_anchor,
                agent,
                model: model.clone(),
                worktree_path: worktree.path.to_string_lossy().into_owned(),
                branch: worktree.branch.clone(),
                sb_profile: profile_path.to_string_lossy().into_owned(),
                progress_path: progress_path.to_string_lossy().into_owned(),
                max_iterations,
                status: RunState::Running.as_str().into(),
            },
        )
        .map_err(|e| e.to_string())?;
    }

    let worktree_path = worktree.path.clone();
    let cfg = LoopConfig {
        run_id: run_id.clone(),
        repo: PathBuf::from(&project.repo_path),
        worktree: worktree.path,
        progress_path,
        task_text,
        max_iterations,
        wrapper,
        model: model.clone(),
    };

    // Register a cancel channel so the live-run Stop button can signal this run.
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    stops.lock().unwrap().insert(run_id.clone(), cancel_tx);

    // Drive the loop off the command's response: it may run for minutes. Progress
    // is persisted on the shared single-writer connection and streamed to the UI.
    // The clones let the background task keep its own handles (and hand fresh ones
    // to a scheduled re-run) while the outer future returns the run id now.
    let db = db.clone();
    let git = git.clone();
    let stops = stops.clone();
    let unacknowledged = unacknowledged.clone();
    let scheduled_resumes = scheduled_resumes.clone();
    let scheduled_auto_merges = scheduled_auto_merges.clone();
    let sched = (
        app.clone(),
        db.clone(),
        git.clone(),
        data_dir.clone(),
        stops.clone(),
        unacknowledged.clone(),
        scheduled_resumes.clone(),
        scheduled_auto_merges.clone(),
    );
    tauri::async_runtime::spawn(async move {
        // Watch the worktree for file changes (the app-sourced `FileChanged`
        // lane) and stream them alongside the agent's events. Polls git status
        // once a second; aborted when the loop ends.
        let poller = {
            let db = db.clone();
            let app = app.clone();
            let run_id = cfg.run_id.clone();
            let worktree = worktree_path;
            tauri::async_runtime::spawn(async move {
                let mut seen = std::collections::HashSet::new();
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    if let Ok(changed) = loopfleet_gitx::worktree_changes(&worktree) {
                        for path in changed {
                            if seen.insert(path.clone()) {
                                record_event(
                                    &db,
                                    &app,
                                    &run_id,
                                    &NormalizedEvent::FileChanged { path: path.into() },
                                );
                            }
                        }
                    }
                }
            })
        };

        let ev_db = db.clone();
        let ev_app = app.clone();
        let ev_id = cfg.run_id.clone();
        // Per-pass upper event boundary: the `seq` of that pass's last event, so
        // the timeline can partition the flat log back into iterations. Captured
        // under the same lock as the insert, so `last_insert_rowid` is that event.
        let offsets: Arc<Mutex<HashMap<u32, i64>>> = Arc::new(Mutex::new(HashMap::new()));
        let ev_offsets = offsets.clone();
        let mut on_event = move |pass: u32, ev: &NormalizedEvent| {
            // Every rate-limit notice refreshes this agent's headroom, not just
            // the one that ends the run: an agent can report a limit mid-pass
            // and carry on, and that observation is still the latest thing we
            // know about its standing.
            if let NormalizedEvent::RateLimited { reset_at, message } = ev {
                record_agent_limit(
                    &ev_db,
                    &ev_app,
                    &ev_agent,
                    reset_at.as_deref(),
                    message.as_deref(),
                );
            }
            if let Some(seq) = record_event(&ev_db, &ev_app, &ev_id, ev) {
                ev_offsets.lock().unwrap().insert(pass, seq);
            }
        };

        let outcome = run_loop(adapter.as_ref(), &git, &cfg, &mut cancel_rx, &mut on_event).await;
        poller.abort();
        stops.lock().unwrap().remove(&cfg.run_id);

        if let Ok(conn) = db.lock() {
            let offsets = offsets.lock().unwrap();
            for it in &outcome.iterations {
                let _ = loopfleet_store::insert_iteration(
                    &conn,
                    &cfg.run_id,
                    it.n,
                    &it.shadow_ref,
                    offsets.get(&it.n).copied(),
                );
            }
            let _ = loopfleet_store::update_run_status(&conn, &cfg.run_id, outcome.state.as_str());
        }

        // Tell the live view the run reached a terminal state.
        let _ = app.emit(
            "run_status",
            RunStatusPayload {
                run_id: cfg.run_id.clone(),
                status: outcome.state.as_str().to_string(),
            },
        );

        // Arm the auto-merge countdown when the terminal state, acceptance, and
        // snapshot presence all line up (`should_auto_merge`, PRD: Autopilot).
        // When the countdown elapses (and hasn't been cancelled via
        // `cancel_auto_merge`, which aborts this handle first) it performs the
        // merge through the exact same `merge_and_accept_run` path `use_run`
        // uses, into the run's current branch (no custom target).
        let accepted = db
            .lock()
            .ok()
            .and_then(|conn| loopfleet_store::load_run(&conn, &cfg.run_id).ok().flatten())
            .map(|detail| detail.accepted)
            .unwrap_or(false);
        let has_snapshot = outcome.iterations.iter().any(|it| !it.shadow_ref.is_empty());
        let merge_in_progress = loopfleet_gitx::merge_in_progress(&cfg.repo).unwrap_or(false);
        let settings = db
            .lock()
            .ok()
            .and_then(|conn| loopfleet_store::load_settings(&conn).ok())
            .unwrap_or_default();
        let decision =
            should_auto_merge(outcome.state, accepted, has_snapshot, merge_in_progress, &settings);
        if let AutoMergeDecision::Blocked(reason) = decision {
            if !matches!(
                reason,
                AutoMergeBlockedReason::Disabled | AutoMergeBlockedReason::RunNotCompleted
            ) {
                eprintln!(
                    "auto-merge not armed for run {}: {reason:?}",
                    cfg.run_id
                );
            }
        }
        if let AutoMergeDecision::Arm { delay_seconds } = decision {
            let merge_at = OffsetDateTime::now_utc() + time::Duration::seconds(delay_seconds as i64);
            let _ = app.emit(
                "auto_merge_armed",
                AutoMergeArmedPayload {
                    run_id: cfg.run_id.clone(),
                    task_anchor: rerun.1.clone(),
                    target_branch: String::new(),
                    merge_at: merge_at.format(&Rfc3339).unwrap_or_else(|_| merge_at.to_string()),
                },
            );
            let auto_merge_run_id = cfg.run_id.clone();
            let auto_merges_for_task = scheduled_auto_merges.clone();
            let auto_merge_db = db.clone();
            let auto_merge_git = git.clone();
            let auto_merge_data_dir = data_dir.clone();
            let auto_merge_app = app.clone();
            let auto_advance_settings = settings.clone();
            let handle = tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(delay_seconds as u64)).await;
                auto_merges_for_task.lock().unwrap().remove(&auto_merge_run_id);
                match merge_and_accept_run(
                    &auto_merge_run_id,
                    None,
                    &auto_merge_db,
                    &auto_merge_git,
                    &auto_merge_data_dir,
                )
                .await
                {
                    Err(e) => {
                        eprintln!("auto-merge failed for run {auto_merge_run_id}: {e}");
                        let _ = auto_merge_app.emit(
                            "auto_merge_failed",
                            AutoMergeFailedPayload {
                                run_id: auto_merge_run_id.clone(),
                                reason: e,
                            },
                        );
                    }
                    Ok(_) => {
                        // Auto-advance (Autopilot): once this run's branch is
                        // merged, queue the plan's next not-started task through
                        // the same `schedule_launch` a manual "schedule for
                        // later" uses, carrying forward this run's agent, model,
                        // and pass count so the chain keeps the user's choices.
                        if auto_advance_settings.auto_advance_enabled {
                            let next = {
                                let conn = auto_merge_db.lock().unwrap();
                                loopfleet_store::load_run(&conn, &auto_merge_run_id)
                                    .ok()
                                    .flatten()
                                    .and_then(|detail| {
                                        let project_id =
                                            loopfleet_store::project_id_for_run(&conn, &auto_merge_run_id)
                                                .ok()
                                                .flatten()?;
                                        let project = get_project(&conn, &project_id).ok()?;
                                        let views = loopfleet_core::plan_overview(&conn, &project).ok()?;
                                        let plan =
                                            views.into_iter().find(|v| v.plan_id == detail.plan_id)?;
                                        let task = loopfleet_core::autopilot::next_task(&plan.tasks)?;
                                        Some((
                                            detail.plan_id,
                                            task.anchor.clone(),
                                            detail.agent,
                                            detail.model,
                                            detail.max_iterations,
                                        ))
                                    })
                            };
                            if let Some((plan_id, task_anchor, agent, model, max_iterations)) = next {
                                let launch_at = OffsetDateTime::now_utc()
                                    + time::Duration::seconds(
                                        auto_advance_settings.auto_advance_delay_seconds as i64,
                                    );
                                let launch_at_str = launch_at
                                    .format(&Rfc3339)
                                    .unwrap_or_else(|_| launch_at.to_string());
                                let advance_app = auto_merge_app.clone();
                                let state = auto_merge_app.state::<AppState>();
                                let _ = schedule_launch(
                                    plan_id,
                                    task_anchor,
                                    agent,
                                    model,
                                    max_iterations,
                                    launch_at_str,
                                    Some("auto_advance".to_string()),
                                    advance_app,
                                    state,
                                );
                            }
                        }
                    }
                }
            });
            scheduled_auto_merges.lock().unwrap().insert(cfg.run_id.clone(), handle);
        }

        // The UI already reflects terminal state for a focused window, so the
        // OS-level nudge is only for when the user isn't looking: a system
        // notification naming the task and outcome, falling back to the prior
        // attention-request nudge when notifications are undeliverable (no
        // permission, or the platform lacks the plugin). The dock badge still
        // tracks the unacknowledged count either way.
        if let Some(window) = app.get_webview_window("main") {
            if !window.is_focused().unwrap_or(false) {
                if !notify_run_terminal(&app, &cfg.task_text, outcome.state) {
                    let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
                }
                #[cfg(target_os = "macos")]
                {
                    let count = unacknowledged.fetch_add(1, Ordering::SeqCst) + 1;
                    let _ = window.set_badge_count(Some(count));
                }
            }
        }

        // A run that ended limit-reached waits out the rate limit: if the agent
        // gave a reset time still in the future, schedule a fresh re-run of the
        // same task at that time. Persisted to `pending_resumes` (not just held
        // in memory) so a crash or restart during the wait doesn't lose the
        // schedule — `rearm_pending_resumes` re-creates this same in-memory
        // task from that row at startup. No (or already-past) reset time means
        // we can't know it is safe to retry, so we leave it for the user. Each
        // resume in the chain bumps `resume_attempt`; past `MAX_RESUME_ATTEMPTS`
        // we stop scheduling and leave the run for the user. A resumed run that
        // ends in any other state never reaches here, so the count implicitly
        // resets: the next time this task is launched (manually) it starts back
        // at attempt 0.
        let next_attempt = resume_attempt + 1;
        if outcome.state == RunState::LimitReached && next_attempt <= MAX_RESUME_ATTEMPTS {
            let buffer = resume_buffer(next_attempt);
            if let Some(delay) = delay_until(outcome.reset_at.as_deref(), OffsetDateTime::now_utc(), buffer) {
                let (app, db, git, data_dir, stops, unacknowledged, scheduled_resumes, scheduled_auto_merges) =
                    sched;
                let (project_id, task_anchor, agent, model, max_iterations) = rerun;
                let resume_run_id = cfg.run_id.clone();
                let resume_at = OffsetDateTime::now_utc() + delay;
                let resume_at_millis = (resume_at.unix_timestamp_nanos() / 1_000_000) as i64;
                if let Ok(conn) = db.lock() {
                    let _ = loopfleet_store::insert_pending_resume(
                        &conn,
                        &loopfleet_store::NewPendingResume {
                            run_id: resume_run_id.clone(),
                            task_anchor: task_anchor.clone(),
                            agent: agent.clone(),
                            model: model.clone(),
                            pass_count: max_iterations,
                            resume_at: resume_at_millis,
                            attempt: next_attempt,
                        },
                    );
                }
                let _ = app.emit(
                    "scheduled_resume",
                    ScheduledResumePayload {
                        run_id: resume_run_id.clone(),
                        resume_at: resume_at
                            .format(&Rfc3339)
                            .unwrap_or_else(|_| resume_at.to_string()),
                    },
                );
                let resumes_for_task = scheduled_resumes.clone();
                let resume_db = db.clone();
                let handle = tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let _ = spawn_run(
                        project_id, task_anchor, agent, model, max_iterations,
                        app, db, git, data_dir, stops, unacknowledged, resumes_for_task.clone(),
                        scheduled_auto_merges, next_attempt,
                    )
                    .await;
                    resumes_for_task.lock().unwrap().remove(&resume_run_id);
                    if let Ok(conn) = resume_db.lock() {
                        let _ = loopfleet_store::delete_pending_resume(&conn, &resume_run_id);
                    }
                });
                scheduled_resumes.lock().unwrap().insert(cfg.run_id.clone(), handle);
            }
        }
    });

    Ok(run_id)
    })
}

/// Re-create every persisted `pending_resumes` row as a live scheduled resume,
/// called once at startup. A pending resume survives a crash or quit (unlike an
/// active run, which `fail_interrupted_runs` gives up on) because it's already
/// past its task and just waiting out a rate limit — so recovering it, rather
/// than dropping it, is both safe and the whole point of persisting it. Each
/// entry is re-armed through the same buffered `delay_until` path a live
/// schedule uses: still in the future, it's slept out again (with the same
/// safety buffer); already due, it fires right away. Either way the
/// `scheduled_resume` event is re-emitted so the frontend's resume chip and
/// Cancel action reattach exactly as if the wait had never been interrupted.
fn rearm_pending_resumes(app: &AppHandle) {
    let state = app.state::<AppState>();
    let pending = {
        let conn = state.db.lock().unwrap();
        loopfleet_store::list_pending_resumes(&conn).unwrap_or_default()
    };

    for resume in pending {
        let project_id = {
            let conn = state.db.lock().unwrap();
            loopfleet_store::project_id_for_run(&conn, &resume.run_id).unwrap_or(None)
        };
        // The original run vanished (shouldn't happen given the FK cascade,
        // but guards against a row the cascade somehow missed) — nothing to
        // resume.
        let Some(project_id) = project_id else {
            let conn = state.db.lock().unwrap();
            let _ = loopfleet_store::delete_pending_resume(&conn, &resume.run_id);
            continue;
        };

        let resume_at = OffsetDateTime::from_unix_timestamp(resume.resume_at / 1000)
            .unwrap_or_else(|_| OffsetDateTime::now_utc());
        let resume_at_str = resume_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| resume_at.to_string());
        let _ = app.emit(
            "scheduled_resume",
            ScheduledResumePayload {
                run_id: resume.run_id.clone(),
                resume_at: resume_at_str.clone(),
            },
        );

        // Reuses `delay_until`, the same buffered wait a live rate-limit
        // schedule computes (with the buffer for this row's attempt number):
        // `Some(delay)` for a still-future resume_at, `None` once it's already
        // due — which the loop below treats as "fire now".
        let delay = delay_until(
            Some(&resume_at_str),
            OffsetDateTime::now_utc(),
            resume_buffer(resume.attempt),
        );

        let db = state.db.clone();
        let git = state.git.clone();
        let data_dir = state.data_dir.clone();
        let stops = state.stops.clone();
        let unacknowledged = state.unacknowledged_runs.clone();
        let scheduled_resumes = state.scheduled_resumes.clone();
        let resumes_for_task = scheduled_resumes.clone();
        let scheduled_auto_merges = state.scheduled_auto_merges.clone();
        let resume_db = db.clone();
        let app = app.clone();
        let resume_run_id = resume.run_id.clone();
        let task_anchor = resume.task_anchor.clone();
        let agent = resume.agent.clone();
        let model = resume.model.clone();
        let max_iterations = resume.pass_count;
        let attempt = resume.attempt;

        let handle = tauri::async_runtime::spawn(async move {
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            let _ = spawn_run(
                project_id, task_anchor, agent, model, max_iterations,
                app, db, git, data_dir, stops, unacknowledged, resumes_for_task.clone(),
                scheduled_auto_merges, attempt,
            )
            .await;
            resumes_for_task.lock().unwrap().remove(&resume_run_id);
            if let Ok(conn) = resume_db.lock() {
                let _ = loopfleet_store::delete_pending_resume(&conn, &resume_run_id);
            }
        });
        scheduled_resumes
            .lock()
            .unwrap()
            .insert(resume.run_id.clone(), handle);
    }
}

/// Post a system notification for a run's terminal state, titled with the
/// bound task's summary and bodied with the outcome. Requests notification
/// permission on first use (a one-time OS prompt); returns `false` without
/// showing anything if permission is denied, the request errors, or the
/// notification fails to post, so the caller can fall back to the existing
/// dock-badge/attention-request signal instead of losing the nudge entirely.
fn notify_run_terminal(app: &AppHandle, task_text: &str, state: RunState) -> bool {
    let outcome = match state {
        RunState::Completed => "completed",
        RunState::Failed => "failed",
        RunState::Stopped => "stopped",
        RunState::LimitReached => "hit a rate limit",
        RunState::Queued | RunState::Running => return false,
    };
    let notifier = app.notification();
    let granted = match notifier.permission_state() {
        Ok(PermissionState::Granted) => true,
        Ok(PermissionState::Denied) => false,
        // Unknown/not yet asked (or the check itself failed): ask once. A
        // denied or errored request degrades silently to the dock signal.
        _ => matches!(notifier.request_permission(), Ok(PermissionState::Granted)),
    };
    if !granted {
        return false;
    }
    let title = task_title(task_text);
    let body = format!("Run {outcome}.");

    // The tauri notification plugin's `.show()` discards the OS handle needed
    // to observe a banner click, so on macOS we go straight to `notify-rust`
    // (the plugin's own backend) to keep that handle and wire the click to
    // the same focus/acknowledge path a real window-focus event takes.
    #[cfg(target_os = "macos")]
    {
        show_clickable_notification(app, &title, &body)
    }
    #[cfg(not(target_os = "macos"))]
    {
        notifier.builder().title(title).body(body).show().is_ok()
    }
}

/// macOS-only: show a notification via `notify-rust` directly and, on a
/// background thread, block for the user's response. A click (any response
/// other than dismissal) brings the main window to front and clears the
/// unseen-runs signal through [`acknowledge_runs`] — the same path the native
/// window-focus handler and the frontend's manual acknowledge calls use — so
/// a banner click behaves exactly like focusing the app.
#[cfg(target_os = "macos")]
fn show_clickable_notification(app: &AppHandle, title: &str, body: &str) -> bool {
    // Without an explicit sending application, mac-notification-sys resolves
    // one from the placeholder name "use_default", which makes Launch Services
    // pop a "Where is use_default?" chooser on every notification. Pin the
    // bundle id the same way tauri-plugin-notification does (Terminal in dev,
    // where our own bundle isn't registered); repeat calls return an
    // AlreadySet error we can ignore.
    let _ = notify_rust::set_application(if tauri::is_dev() {
        "com.apple.Terminal"
    } else {
        &app.config().identifier
    });
    let mut notification = notify_rust::Notification::new();
    notification.summary(title).body(body);
    match notification.show() {
        Ok(handle) => {
            let app = app.clone();
            std::thread::spawn(move || {
                handle.wait_for_action(move |action| {
                    if action == "__closed" {
                        return;
                    }
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.unminimize();
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    let state = app.state::<AppState>();
                    let _ = acknowledge_runs(app.clone(), state);
                });
            });
            true
        }
        Err(_) => false,
    }
}

/// A notification title from a task's bound text: its first line, capped so
/// long task descriptions don't overflow the OS notification chrome.
fn task_title(task_text: &str) -> String {
    const MAX_CHARS: usize = 80;
    let first_line = task_text.lines().next().unwrap_or(task_text).trim();
    if first_line.chars().count() <= MAX_CHARS {
        return first_line.to_string();
    }
    let truncated: String = first_line.chars().take(MAX_CHARS).collect();
    format!("{truncated}…")
}

/// How long to wait before re-running a rate-limited run: the gap between `now`
/// and the agent-reported `reset_at` (ISO-8601 / RFC 3339), plus `buffer`.
/// `None` when there is no reset time, it doesn't parse, or it (even with the
/// buffer added) is already in the past — i.e. "don't auto-reschedule". We only
/// retry when we know a future instant the limit lifts, so a re-run never
/// hammers a still-exhausted limit.
fn delay_until(
    reset_at: Option<&str>,
    now: OffsetDateTime,
    buffer: time::Duration,
) -> Option<std::time::Duration> {
    let reset = OffsetDateTime::parse(reset_at?, &Rfc3339).ok()?;
    let target = reset + buffer;
    // `TryFrom<time::Duration>` fails for a negative span, so a past reset → None.
    std::time::Duration::try_from(target - now).ok()
}

/// A resumed run is retried at most this many times before we give up and leave
/// it for the user — a limit that keeps repeatedly-rate-limited work from
/// scheduling itself forever.
const MAX_RESUME_ATTEMPTS: u32 = 3;

/// Safety buffer added past the agent-reported reset time for a given resume
/// attempt (1 = first resume): 60s, doubling each attempt, since providers'
/// reset clocks aren't always exact and a repeatedly-limited task likely needs
/// more room to clear.
fn resume_buffer(attempt: u32) -> time::Duration {
    time::Duration::seconds(60 * 2i64.pow(attempt.saturating_sub(1)))
}

/// A scheduled launch found the agent still exhausted at most this many times
/// before the schedule is dropped rather than pushed back again — the launch
/// counterpart of `MAX_RESUME_ATTEMPTS`, so a launch scheduled against a
/// persistently-limited agent doesn't push itself back forever.
const MAX_LAUNCH_RESCHEDULES: u32 = 3;

/// Resolve one agent's current usage snapshot fresh, right before a scheduled
/// launch is about to fire — the snapshot the UI was last told about
/// (`published_snapshot`) can be minutes stale, so this re-probes rather than
/// trusting it. Same three-tier fallback `agent_usage` sweeps over every known
/// agent with: a live adapter probe, then the last stored rate-limit
/// observation, then `unknown`.
async fn resolve_agent_usage(db: &Mutex<Connection>, agent: &str) -> UsageSnapshot {
    let now = now_ms();
    let probed = match build_adapter(agent) {
        Some(adapter) => adapter.usage_snapshot(now).await.ok(),
        None => None,
    };
    probed
        .filter(|s| s.source != UsageSource::Unknown)
        .or_else(|| {
            let conn = db.lock().ok()?;
            let usage = loopfleet_store::load_agent_usage(&conn, agent).ok()??;
            let notice = rate_limit_notice(agent, usage.reset_at.as_deref(), usage.observed_at);
            Some(fold_rate_limit(None, &notice))
        })
        .unwrap_or_else(|| UsageSnapshot::unknown(agent, now))
}

/// A scheduled launch is dropped after `MAX_LAUNCH_RESCHEDULES` pushbacks,
/// pushed with the reason so the UI can surface a notice rather than let the
/// schedule silently vanish.
#[derive(Clone, serde::Serialize)]
struct ScheduledLaunchDroppedPayload {
    id: i64,
    plan_id: String,
    task_anchor: String,
    reason: String,
}

/// Clear the OS-level "finished runs waiting" signal — dock badge count and any
/// pending attention request. The manual counterpart to the native window-focus
/// handler installed in `run()`: that handler only fires on an OS-level focus
/// change, but the frontend also clears its in-app `unseen` marker on a JS
/// `focus` event and when a specific finished run is opened, neither of which
/// necessarily coincides with a native focus transition. Called from those same
/// two paths so the OS signal clears exactly when the in-app marker does.
#[tauri::command]
fn acknowledge_runs(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.unacknowledged_runs.store(0, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        let _ = window.set_badge_count(None);
        let _ = window.request_user_attention(None);
    }
    Ok(())
}

/// Request a stop of an active run. Signals the run's cancel channel; the loop
/// stops at the current pass boundary (SIGTERMing the agent's process group) and
/// finalizes its status (`stopped`). Errors if the run is not active.
#[tauri::command]
fn stop_run(run_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let stops = state.stops.lock().unwrap();
    match stops.get(&run_id) {
        Some(tx) => {
            let _ = tx.send(true);
            Ok(())
        }
        None => Err(format!("run is not active: {run_id}")),
    }
}

/// Delete a finished run's on-disk footprint: its worktree (via the gitx
/// `reap`, through the serialized `GitActor`), its sandbox profile
/// (`profiles/<run_id>.sb`), and its progress directory
/// (`progress/<run_id>/`); then stamps `runs.reaped_at`.
///
/// Refuses runs still `queued`/`running` (they're using their worktree) and
/// runs with a row in `pending_resumes` (a scheduled resume needs the
/// worktree to still be there when it fires). Idempotent: a worktree/profile/
/// progress dir already gone is not an error.
///
/// Also refuses (without erroring) a worktree that's some live process's
/// current working directory — e.g. a user with a shell or editor open in
/// it — since deleting out from under that process would break it. Detected
/// via `lsof` just before the delete; the run is left for the next sweep
/// pass rather than reaped now.
///
/// When the `cleanup_after_merge` setting is on, also deletes the run's
/// `agent/<run-id>` branch and prunes the now-stale worktree administrative
/// metadata (`git worktree prune`) once the worktree itself is gone. The
/// run's shadow refs (`refs/agentapp/run-<id>/iter-*`) are never touched —
/// they're the durable record of the run's diff history. A failed branch
/// delete is logged as a warning rather than failing the whole reap, since
/// the worktree and on-disk footprint are already cleaned up by that point.
async fn reap_run(
    db: &Arc<Mutex<Connection>>,
    git: &GitActor,
    data_dir: &std::path::Path,
    run_id: &str,
) -> Result<(), String> {
    let (repo_path, worktree_path, cleanup_after_merge) = {
        let conn = db.lock().unwrap();
        let detail = loopfleet_store::load_run(&conn, run_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("run not found: {run_id}"))?;
        if matches!(detail.status.as_str(), "queued" | "running") {
            return Err(format!("cannot reap an active run: {run_id}"));
        }
        if loopfleet_store::has_pending_resume(&conn, run_id).map_err(|e| e.to_string())? {
            return Err(format!("cannot reap a run with a pending resume: {run_id}"));
        }
        let cleanup_after_merge = loopfleet_store::load_settings(&conn)
            .map(|s| s.cleanup_after_merge)
            .unwrap_or(true);
        (PathBuf::from(detail.repo_path), detail.worktree_path, cleanup_after_merge)
    };

    if let Some(worktree_path) = worktree_path {
        if worktree_in_use(&worktree_path) {
            eprintln!(
                "reap_run: deferring run {run_id}, worktree {worktree_path} is a live process's cwd"
            );
            return Ok(());
        }
        let worktrees_root = data_dir.join("worktrees");
        git.reap(repo_path.clone(), worktrees_root, PathBuf::from(worktree_path))
            .await
            .map_err(|e| e.to_string())?;

        if cleanup_after_merge {
            let branch = loopfleet_gitx::worktree::branch_for(run_id);
            if let Err(e) = git.delete_branch(repo_path.clone(), branch).await {
                eprintln!("reap_run: failed to delete branch for run {run_id}: {e}");
            }
            if let Err(e) = git.cleanup_orphans(repo_path).await {
                eprintln!("reap_run: failed to prune worktree metadata for run {run_id}: {e}");
            }
        }
    }

    let profile_path = data_dir.join("profiles").join(format!("{run_id}.sb"));
    if profile_path.exists() {
        std::fs::remove_file(&profile_path).map_err(|e| e.to_string())?;
    }

    let progress_dir = data_dir.join("progress").join(run_id);
    if progress_dir.exists() {
        std::fs::remove_dir_all(&progress_dir).map_err(|e| e.to_string())?;
    }

    let conn = db.lock().unwrap();
    loopfleet_store::mark_run_reaped(&conn, run_id).map_err(|e| e.to_string())
}

/// Whether some live process currently has `worktree_path` as its working
/// directory (or otherwise has it open), per `lsof`. Used to avoid deleting a
/// worktree out from under, e.g., a shell or editor a user left open in it.
///
/// Fails open: if `lsof` is missing or errors, we can't tell either way, so
/// treat the worktree as not in use rather than pile up runs that never get
/// reaped because of an environment quirk.
fn worktree_in_use(worktree_path: &str) -> bool {
    std::process::Command::new("lsof")
        .arg(worktree_path)
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

/// Reap every finished run's on-disk footprint once it's no longer worth
/// keeping around, plus any worktree directory with no matching run row at
/// all (e.g. left behind by a run whose row was later deleted).
///
/// A terminal run is swept once it's `accepted` (its diff already landed, so
/// there's no reason to keep the checkout) or once it's older than the
/// user's `worktree_retention_hours` setting (default 48h), measured from
/// `finished_at` — falling back to the worktree directory's mtime when
/// `finished_at` is NULL (e.g. a row from before that column existed). `0`
/// means reap immediately (any age qualifies); `-1` means never age out (only
/// `accepted` runs and orphan directories are swept). Runs still active or
/// with a pending resume are skipped by `reap_run`'s own guards; sweep just
/// ignores the error. Wired to run once at startup after `cleanup_orphans`
/// and then hourly.
async fn sweep_worktrees(
    db: &Arc<Mutex<Connection>>,
    git: &GitActor,
    data_dir: &std::path::Path,
) -> SweepResult {
    let worktrees_root = data_dir.join("worktrees");
    let now = std::time::SystemTime::now();
    let mut result = SweepResult {
        removed: 0,
        bytes_reclaimed: 0,
    };

    let (candidates, retention_hours) = {
        let conn = db.lock().unwrap();
        let candidates = loopfleet_store::list_sweep_candidates(&conn).unwrap_or_default();
        let retention_hours = loopfleet_store::load_settings(&conn)
            .map(|s| s.worktree_retention_hours)
            .unwrap_or(48);
        (candidates, retention_hours)
    };

    for candidate in candidates {
        let eligible = candidate.accepted
            || match retention_hours {
                -1 => false,
                hours => {
                    let retention =
                        std::time::Duration::from_secs((hours.max(0) as u64) * 60 * 60);
                    match candidate.finished_at {
                        Some(finished_at_millis) => {
                            let finished_at = std::time::UNIX_EPOCH
                                + std::time::Duration::from_millis(
                                    finished_at_millis.max(0) as u64
                                );
                            now.duration_since(finished_at).unwrap_or_default() >= retention
                        }
                        None => candidate
                            .worktree_path
                            .as_ref()
                            .and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
                            .map(|mtime| now.duration_since(mtime).unwrap_or_default() >= retention)
                            // No worktree on disk and no finished_at to fall back on:
                            // nothing to measure age against, so sweep it (its
                            // footprint is already gone anyway, just needs
                            // `reaped_at` stamped).
                            .unwrap_or(true),
                    }
                }
            };

        if eligible {
            let worktree_path = candidate.worktree_path.as_ref().map(std::path::Path::new);
            let bytes_before = worktree_path.map(dir_size).unwrap_or(0);
            match reap_run(db, git, data_dir, &candidate.id).await {
                // `reap_run` also returns `Ok(())` without removing anything when
                // the worktree is some live process's cwd (deferred to next
                // sweep) — only count it once the directory is actually gone.
                Ok(()) if worktree_path.is_none_or(|p| !p.exists()) => {
                    result.removed += 1;
                    result.bytes_reclaimed += bytes_before;
                }
                Ok(()) => {}
                Err(e) => {
                    eprintln!("sweep_worktrees: failed to reap run {}: {e}", candidate.id);
                }
            }
        }
    }

    let known_run_ids: std::collections::HashSet<String> = {
        let conn = db.lock().unwrap();
        loopfleet_store::all_run_ids(&conn)
            .unwrap_or_default()
            .into_iter()
            .collect()
    };

    let Ok(entries) = std::fs::read_dir(&worktrees_root) else {
        return result;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if known_run_ids.contains(&name) {
            continue;
        }
        let bytes_before = dir_size(&entry.path());
        match std::fs::remove_dir_all(entry.path()) {
            Ok(()) => {
                result.removed += 1;
                result.bytes_reclaimed += bytes_before;
            }
            Err(e) => {
                eprintln!(
                    "sweep_worktrees: failed to remove orphan worktree dir {}: {e}",
                    entry.path().display()
                );
            }
        }
    }
    result
}

/// Total on-disk size of everything under `path` (recursing into
/// subdirectories), in bytes. Best-effort: unreadable entries are skipped
/// rather than failing the whole measurement, since this only feeds an
/// informational byte count (sweep bytes-reclaimed) rather than anything
/// correctness-critical.
fn dir_size(path: &std::path::Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_symlink() {
        return 0;
    }
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| dir_size(&entry.path()))
        .sum()
}

/// The result of a worktree sweep pass: how many stale worktrees/runs were
/// removed and how many bytes their on-disk footprint reclaimed. Returned by
/// `sweep_worktrees_now` for the UI to toast.
#[derive(Clone, serde::Serialize)]
struct SweepResult {
    removed: u32,
    bytes_reclaimed: u64,
}

/// Run a worktree sweep pass immediately, outside the hourly schedule, for the
/// settings panel's "Clean up now" control. Same eligibility rules as the
/// background sweep (accepted runs, aged-out runs per
/// `worktree_retention_hours`, and orphan worktree directories) — this just
/// triggers a pass on demand and hands the UI something to toast.
#[tauri::command]
async fn sweep_worktrees_now(state: State<'_, AppState>) -> Result<SweepResult, String> {
    Ok(sweep_worktrees(&state.db, &state.git, &state.data_dir).await)
}

/// Abort a pending rate-limit re-run before it fires, keyed by the original
/// run's id. Emits `scheduled_resume_cancelled` so the UI can drop the
/// "resuming at…" indicator.
#[tauri::command]
fn cancel_scheduled_resume(
    run_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let handle = state.scheduled_resumes.lock().unwrap().remove(&run_id);
    match handle {
        Some(handle) => {
            handle.abort();
            if let Ok(conn) = state.db.lock() {
                let _ = loopfleet_store::delete_pending_resume(&conn, &run_id);
            }
            let _ = app.emit(
                "scheduled_resume_cancelled",
                ScheduledResumeCancelledPayload {
                    run_id: run_id.clone(),
                },
            );
            Ok(())
        }
        None => Err(format!("no scheduled resume for run: {run_id}")),
    }
}

/// Schedule a run of `task_anchor` in `plan_id`'s project to launch later, at
/// `launch_at` (RFC 3339). The schedule is persisted (`scheduled_launches`)
/// before returning, so a crash or quit before it fires is recovered at the
/// next startup (see `rearm_scheduled_launches`); a sleeping task is then
/// spawned that calls the same `spawn_run` path `launch_run` uses once the
/// time comes. Returns the schedule's row id — the handle
/// `cancel_scheduled_launch` needs.
#[tauri::command]
fn schedule_launch(
    plan_id: String,
    task_anchor: String,
    agent: String,
    model: Option<String>,
    max_iterations: u32,
    launch_at: String,
    origin: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let origin = origin.unwrap_or_else(|| "manual".to_string());
    let launch_at_dt =
        OffsetDateTime::parse(&launch_at, &Rfc3339).map_err(|e| format!("invalid launch_at: {e}"))?;
    let launch_at_millis = (launch_at_dt.unix_timestamp_nanos() / 1_000_000) as i64;

    let id = {
        let conn = state.db.lock().unwrap();
        loopfleet_store::insert_scheduled_launch(
            &conn,
            &loopfleet_store::NewScheduledLaunch {
                plan_id: plan_id.clone(),
                task_anchor: task_anchor.clone(),
                agent: agent.clone(),
                model: model.clone(),
                pass_count: max_iterations,
                launch_at: launch_at_millis,
                origin: origin.clone(),
            },
        )
        .map_err(|e| e.to_string())?
    };

    let _ = app.emit(
        "scheduled_launch",
        ScheduledLaunchPayload {
            id,
            plan_id: plan_id.clone(),
            task_anchor: task_anchor.clone(),
            launch_at: launch_at_dt
                .format(&Rfc3339)
                .unwrap_or_else(|_| launch_at.clone()),
            origin: origin.clone(),
        },
    );

    let delay = std::time::Duration::try_from(launch_at_dt - OffsetDateTime::now_utc())
        .unwrap_or(std::time::Duration::ZERO);
    arm_scheduled_launch(
        app,
        state.db.clone(),
        state.git.clone(),
        state.data_dir.clone(),
        state.stops.clone(),
        state.unacknowledged_runs.clone(),
        state.scheduled_resumes.clone(),
        state.scheduled_launches.clone(),
        state.scheduled_auto_merges.clone(),
        id,
        plan_id,
        task_anchor,
        agent,
        model,
        max_iterations,
        delay,
        0,
        origin,
    );

    Ok(id)
}

/// Abort a scheduled launch before it fires, keyed by its `scheduled_launches`
/// row id. Emits `scheduled_launch_cancelled` so the UI can drop the
/// "launching at…" indicator.
#[tauri::command]
fn cancel_scheduled_launch(id: i64, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let handle = state.scheduled_launches.lock().unwrap().remove(&id);
    match handle {
        Some(handle) => {
            handle.abort();
            if let Ok(conn) = state.db.lock() {
                let _ = loopfleet_store::delete_scheduled_launch(&conn, id);
            }
            let _ = app.emit(
                "scheduled_launch_cancelled",
                ScheduledLaunchCancelledPayload { id },
            );
            Ok(())
        }
        None => Err(format!("no scheduled launch: {id}")),
    }
}

/// Abort a run's armed auto-merge countdown before it fires, keyed by the
/// run's id. The run itself is left exactly as it landed — completed,
/// unaccepted — since cancelling the merge is not the same as rejecting the
/// run. Emits `auto_merge_cancelled` so the UI can drop the "merging at…"
/// indicator.
#[tauri::command]
fn cancel_auto_merge(run_id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let handle = state.scheduled_auto_merges.lock().unwrap().remove(&run_id);
    match handle {
        Some(handle) => {
            handle.abort();
            let _ = app.emit(
                "auto_merge_cancelled",
                AutoMergeCancelledPayload {
                    run_id: run_id.clone(),
                },
            );
            Ok(())
        }
        None => Err(format!("no armed auto-merge for run: {run_id}")),
    }
}

/// Spawn the sleep-then-launch task behind one scheduled launch and register
/// its handle, so `cancel_scheduled_launch` can abort it before it fires.
/// Shared by `schedule_launch` (a fresh schedule, `reschedule_count = 0`) and
/// `rearm_scheduled_launches` (recovering one across a restart, carrying
/// forward whatever count was persisted) — the only other difference between
/// them is `delay`, computed relative to whenever each is called.
///
/// Before actually launching, the agent's usage is re-checked: the schedule
/// may have been set minutes or hours ago, and the snapshot the UI last saw
/// can be stale by the time it fires. If the agent is still exhausted and
/// reports a reset time later than now, the launch is pushed back to that
/// reset time instead — up to `MAX_LAUNCH_RESCHEDULES` times, after which the
/// schedule is dropped and `scheduled_launch_dropped` is emitted so the UI can
/// tell the user rather than let it vanish silently.
///
/// Once it actually fires, the project id is resolved fresh from `plan_id`
/// (rather than carried since scheduling) since a scheduled launch, unlike a
/// run, has no project id of its own to remember — and the plan may no longer
/// exist by firing time (e.g. deleted after the schedule was set). The launch
/// then runs through the exact same `spawn_run` path `launch_run` uses, as a
/// fresh attempt (`resume_attempt = 0`) independent of any rate-limit resume
/// chain; `spawn_run` re-validates the task anchor still resolves in the
/// project's plans, the project is still attached, and the agent CLI is still
/// installed. A missing plan or a `spawn_run` failure both drop the schedule
/// and emit `scheduled_launch_dropped` with the reason, the same
/// user-visible path used when reschedules run out, rather than launching
/// nothing and leaving the "launching at…" indicator to vanish unexplained.
/// The schedule row and this handle's entry are cleared once the launch
/// either fires, is pushed back, or is dropped, since in every case there is
/// nothing left for this armed wait to do.
#[allow(clippy::too_many_arguments)]
fn arm_scheduled_launch(
    app: AppHandle,
    db: Arc<Mutex<Connection>>,
    git: GitActor,
    data_dir: PathBuf,
    stops: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    unacknowledged: Arc<AtomicI64>,
    scheduled_resumes: Arc<Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>>,
    scheduled_launches: Arc<Mutex<HashMap<i64, tauri::async_runtime::JoinHandle<()>>>>,
    scheduled_auto_merges: Arc<Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>>,
    id: i64,
    plan_id: String,
    task_anchor: String,
    agent: String,
    model: Option<String>,
    max_iterations: u32,
    delay: std::time::Duration,
    reschedule_count: u32,
    origin: String,
) {
    let launches_for_task = scheduled_launches.clone();
    let launch_db = db.clone();
    let spawn_db = db.clone();
    let app_for_spawn = app.clone();
    let handle = tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;

        let snapshot = resolve_agent_usage(&launch_db, &agent).await;
        let now = now_ms();
        let still_exhausted = resolve_display(&snapshot, now, UsageThresholds::default())
            == UsageDisplay::Exhausted;
        let later_reset = snapshot.reset_at_ms.filter(|reset_at_ms| *reset_at_ms > now);

        if still_exhausted {
            if let Some(reset_at_ms) = later_reset {
                let next_count = reschedule_count + 1;
                if next_count <= MAX_LAUNCH_RESCHEDULES {
                    if let Ok(conn) = launch_db.lock() {
                        let _ = loopfleet_store::reschedule_launch(&conn, id, reset_at_ms, next_count);
                    }
                    let reset_at = OffsetDateTime::from_unix_timestamp(reset_at_ms / 1000)
                        .unwrap_or_else(|_| OffsetDateTime::now_utc());
                    let _ = app.emit(
                        "scheduled_launch",
                        ScheduledLaunchPayload {
                            id,
                            plan_id: plan_id.clone(),
                            task_anchor: task_anchor.clone(),
                            launch_at: reset_at.format(&Rfc3339).unwrap_or_else(|_| reset_at.to_string()),
                            origin: origin.clone(),
                        },
                    );
                    let next_delay = std::time::Duration::try_from(reset_at - OffsetDateTime::now_utc())
                        .unwrap_or(std::time::Duration::ZERO);
                    arm_scheduled_launch(
                        app,
                        db,
                        git,
                        data_dir,
                        stops,
                        unacknowledged,
                        scheduled_resumes,
                        launches_for_task,
                        scheduled_auto_merges,
                        id,
                        plan_id,
                        task_anchor,
                        agent,
                        model,
                        max_iterations,
                        next_delay,
                        next_count,
                        origin,
                    );
                    return;
                }

                launches_for_task.lock().unwrap().remove(&id);
                if let Ok(conn) = launch_db.lock() {
                    let _ = loopfleet_store::delete_scheduled_launch(&conn, id);
                }
                let _ = app.emit(
                    "scheduled_launch_dropped",
                    ScheduledLaunchDroppedPayload {
                        id,
                        plan_id,
                        task_anchor,
                        reason: format!(
                            "{agent} is still rate-limited after {next_count} reschedule(s); \
                             this scheduled launch was dropped — launch it manually once it clears"
                        ),
                    },
                );
                return;
            }
        }

        let project_id = {
            let conn = match launch_db.lock() {
                Ok(c) => c,
                Err(_) => return,
            };
            conn.query_row(
                "SELECT project_id FROM plans WHERE id = ?1",
                [&plan_id],
                |r| r.get::<_, String>(0),
            )
            .ok()
        };

        // `spawn_run` re-validates the task anchor, the project's attachment,
        // and the agent CLI's presence itself — all of which may have changed
        // in the time between scheduling and firing. Either a missing plan
        // (caught here) or a `spawn_run` failure (caught below) must drop the
        // schedule with a visible reason rather than launch nothing and stay
        // silent, since by this point there's no "launching at…" indicator
        // left in the UI to explain the absence.
        let launch_result = match project_id {
            Some(project_id) => spawn_run(
                project_id,
                task_anchor.clone(),
                agent,
                model,
                max_iterations,
                app_for_spawn,
                spawn_db,
                git,
                data_dir,
                stops,
                unacknowledged,
                scheduled_resumes,
                scheduled_auto_merges,
                0,
            )
            .await
            .map_err(|e| format!("scheduled launch of '{task_anchor}' failed: {e}")),
            None => Err(format!(
                "scheduled launch of '{task_anchor}' failed: plan no longer exists"
            )),
        };

        match launch_result {
            Ok(run_id) => {
                let _ = app.emit("scheduled_launch_fired", ScheduledLaunchFiredPayload { id, run_id });
            }
            Err(reason) => {
                let _ = app.emit(
                    "scheduled_launch_dropped",
                    ScheduledLaunchDroppedPayload {
                        id,
                        plan_id,
                        task_anchor,
                        reason,
                    },
                );
            }
        }

        launches_for_task.lock().unwrap().remove(&id);
        if let Ok(conn) = launch_db.lock() {
            let _ = loopfleet_store::delete_scheduled_launch(&conn, id);
        }
    });
    scheduled_launches.lock().unwrap().insert(id, handle);
}

/// Re-create every persisted `scheduled_launches` row as a live scheduled
/// launch, called once at startup — the launch-side counterpart of
/// `rearm_pending_resumes`. A scheduled launch survives a crash or quit because
/// it's just waiting for its time to arrive, so recovering it is both safe and
/// the whole point of persisting it. Each entry is re-armed with whatever delay
/// remains until its `launch_at` (already-due fires right away), and the
/// `scheduled_launch` event is re-emitted so the frontend's indicator and
/// Cancel action reattach exactly as if the wait had never been interrupted.
fn rearm_scheduled_launches(app: &AppHandle) {
    let state = app.state::<AppState>();
    let pending = {
        let conn = state.db.lock().unwrap();
        loopfleet_store::list_scheduled_launches(&conn).unwrap_or_default()
    };

    for launch in pending {
        let launch_at = OffsetDateTime::from_unix_timestamp(launch.launch_at / 1000)
            .unwrap_or_else(|_| OffsetDateTime::now_utc());
        let launch_at_str = launch_at.format(&Rfc3339).unwrap_or_else(|_| launch_at.to_string());
        let _ = app.emit(
            "scheduled_launch",
            ScheduledLaunchPayload {
                id: launch.id,
                plan_id: launch.plan_id.clone(),
                task_anchor: launch.task_anchor.clone(),
                launch_at: launch_at_str,
                origin: launch.origin.clone(),
            },
        );

        let delay = std::time::Duration::try_from(launch_at - OffsetDateTime::now_utc())
            .unwrap_or(std::time::Duration::ZERO);

        arm_scheduled_launch(
            app.clone(),
            state.db.clone(),
            state.git.clone(),
            state.data_dir.clone(),
            state.stops.clone(),
            state.unacknowledged_runs.clone(),
            state.scheduled_resumes.clone(),
            state.scheduled_launches.clone(),
            state.scheduled_auto_merges.clone(),
            launch.id,
            launch.plan_id,
            launch.task_anchor,
            launch.agent,
            launch.model,
            launch.pass_count,
            delay,
            launch.reschedule_count,
            launch.origin,
        );
    }
}

/// Every run bound to any task in `plan_id`. The plan view groups these by
/// `task_anchor` so each task can list its runs and open their timelines.
#[tauri::command]
fn plan_runs(plan_id: String, state: State<'_, AppState>) -> Result<Vec<RunSummary>, String> {
    let conn = state.db.lock().unwrap();
    loopfleet_store::list_runs_for_plan(&conn, &plan_id).map_err(|e| e.to_string())
}

/// A run's timeline: its iterations as rows, the events that occurred during
/// each, and each iteration's diff (read-only over the app-owned shadow refs).
#[tauri::command]
fn run_timeline(run_id: String, state: State<'_, AppState>) -> Result<RunTimeline, String> {
    let conn = state.db.lock().unwrap();
    loopfleet_core::run_timeline(&conn, &run_id).map_err(|e| e.to_string())
}

/// The compare view for a task: every run bound to it, side by side, each with
/// its final-ref cumulative diff (read-only over the app-owned shadow refs).
#[tauri::command]
fn compare_task(
    plan_id: String,
    task_anchor: String,
    state: State<'_, AppState>,
) -> Result<CompareView, String> {
    let conn = state.db.lock().unwrap();
    loopfleet_core::compare_view(&conn, &plan_id, &task_anchor).map_err(|e| e.to_string())
}

/// The result of "use this run": which branch the run was merged into and how.
#[derive(serde::Serialize)]
struct UseRunResult {
    target_branch: String,
    /// The squashed commit this merge created on the target branch — the sha the
    /// user can find in their own history, not the app-internal shadow commit it
    /// was squashed from. An up-to-date merge reports the target's existing tip.
    merged_commit: String,
    created: bool,
    up_to_date: bool,
    /// Set when the merge succeeded but the immediate post-merge cleanup (run
    /// reaping, gated on the `cleanup_after_merge` setting) hit a problem —
    /// the merge itself is not rolled back or failed for this, since the
    /// user's work already landed; they're just told cleanup didn't finish.
    cleanup_error: Option<String>,
}

/// "Use this run": merge the run's final state into a target branch and mark
/// the run accepted. `target_branch = None` (or empty) merges into the repo's
/// currently checked-out branch — the default, landing the run's work where the
/// user is working as a single squashed commit carrying the run's own commit
/// message plus a `Co-authored-by: loopfleet` trailer. A non-empty `target_branch`
/// names a custom branch (created if absent). The merge runs through the
/// serialized git actor; the current-branch default merges in the main worktree
/// (guarded by a clean tree), a custom target uses a throwaway worktree so the
/// user's own checkout is never touched.
///
/// Shared by the `use_run` command (a user-initiated merge) and the auto-merge
/// countdown spawned when a run's terminal state, acceptance, and snapshot
/// presence line up (`should_auto_merge`) — an automatic merge is not allowed
/// to diverge from a manual one, so both call this one path, including
/// `set_run_accepted` and the post-merge cleanup reap.
async fn merge_and_accept_run(
    run_id: &str,
    target_branch: Option<String>,
    db: &Arc<Mutex<Connection>>,
    git: &GitActor,
    data_dir: &std::path::Path,
) -> Result<UseRunResult, String> {
    let target = target_branch
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());

    // Resolve the run's parent repo, its final shadow ref, and enough of its
    // task binding to compose a fallback commit message for it.
    let (repo_path, source_ref, task_text, agent, pass_count) = {
        let conn = db.lock().unwrap();
        let detail = loopfleet_store::load_run(&conn, run_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("unknown run: {run_id}"))?;
        let iterations =
            loopfleet_store::load_iterations(&conn, run_id).map_err(|e| e.to_string())?;
        let pass_count = iterations.len() as u32;
        let source_ref = iterations
            .into_iter()
            .rev()
            .find_map(|it| it.shadow_ref)
            .ok_or_else(|| "run has no snapshot to use".to_string())?;
        let task_text = loopfleet_store::load_task(&conn, &detail.plan_id, &detail.task_anchor)
            .map_err(|e| e.to_string())?
            .map(|t| t.text)
            .unwrap_or(detail.task_anchor);
        (detail.repo_path, source_ref, task_text, detail.agent, pass_count)
    };

    // The run's own progress file, read for its `SUMMARY:` line — durable state
    // the agent wrote across passes, not something reconstructed here. Used only
    // as a fallback message; a run whose worktree carries its own commit wording
    // has that wording win instead (see `loopfleet_gitx::merge::merge_run`).
    let progress_path = data_dir.join("progress").join(run_id).join("progress.md");
    let summary = std::fs::read_to_string(&progress_path)
        .ok()
        .and_then(|c| loopfleet_core::progress::summary_from_contents(&c))
        .unwrap_or_default();
    let fallback_message =
        loopfleet_core::compose_commit_message(&summary, &task_text, run_id, &agent, pass_count);

    let scratch_root = data_dir.join("worktrees");
    let merge = git
        .merge_run(
            PathBuf::from(&repo_path),
            source_ref,
            target,
            scratch_root,
            Some(fallback_message),
        )
        .await
        .map_err(|e| e.to_string())?;

    let cleanup_after_merge = {
        let conn = db.lock().unwrap();
        loopfleet_store::set_run_accepted(&conn, run_id).map_err(|e| e.to_string())?;
        loopfleet_store::load_settings(&conn)
            .map(|s| s.cleanup_after_merge)
            .unwrap_or(true)
    };

    let cleanup_error = if cleanup_after_merge {
        reap_run(db, git, data_dir, run_id).await.err()
    } else {
        None
    };

    Ok(UseRunResult {
        target_branch: merge.target_branch,
        merged_commit: merge.merged_commit,
        created: merge.created,
        up_to_date: merge.up_to_date,
        cleanup_error,
    })
}

/// "Use this run": the `use_run` command is a thin wrapper over
/// [`merge_and_accept_run`] — see that function for the merge/accept/cleanup
/// behavior itself.
#[tauri::command]
async fn use_run(
    run_id: String,
    target_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<UseRunResult, String> {
    merge_and_accept_run(&run_id, target_branch, &state.db, &state.git, &state.data_dir).await
}

/// Load one project by id.
fn get_project(conn: &Connection, id: &str) -> Result<Project, String> {
    conn.query_row(
        "SELECT id, repo_path, plan_convention FROM projects WHERE id = ?1",
        [id],
        |r| {
            Ok(Project {
                id: r.get(0)?,
                repo_path: r.get(1)?,
                plan_convention: r.get(2)?,
            })
        },
    )
    .map_err(|_| format!("unknown project: {id}"))
}

/// A filename-safe version of a title: strip characters the filesystem (or a
/// zip/email attachment step downstream) would choke on, collapse whitespace,
/// and cap the length so a long task line doesn't produce an unwieldy path.
fn report_file_stem(title: &str) -> String {
    const MAX_CHARS: usize = 80;
    let cleaned: String = title
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let stem: String = collapsed.chars().take(MAX_CHARS).collect();
    if stem.is_empty() {
        "report".to_string()
    } else {
        stem
    }
}

/// Write `html` to a path the user chooses via a native save dialog (defaulting
/// to `default_name`), then reveal the saved file in Finder. Returns the chosen
/// path, or `None` if the user cancelled the dialog.
fn save_report(app: &AppHandle, default_name: &str, html: &str) -> Result<Option<String>, String> {
    let chosen = app
        .dialog()
        .file()
        .set_file_name(default_name)
        .add_filter("HTML", &["html"])
        .blocking_save_file();
    let Some(file_path) = chosen else {
        return Ok(None);
    };
    let path = file_path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, html).map_err(|e| format!("writing report: {e}"))?;

    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(&path).spawn();

    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Export one task's stored data (its runs, events, and diffs) as a standalone
/// HTML report, saved via a native save dialog defaulting to a name built from
/// the task's text, then revealed in Finder. Returns the saved path, or `None`
/// if the user cancelled the dialog.
// Async so the command leaves the main thread: `blocking_save_file` parks the
// calling thread while the dialog runs on the main event loop, so calling it
// from a sync (main-thread) command deadlocks the whole app.
#[tauri::command]
async fn export_task_report(
    app: AppHandle,
    plan_id: String,
    task_anchor: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let report = {
        let conn = state.db.lock().unwrap();
        loopfleet_core::task_report(&conn, &plan_id, &task_anchor).map_err(|e| e.to_string())?
    };
    let default_name = format!("{}.html", report_file_stem(&report.text));
    let html = loopfleet_core::render_task_report(&report);
    save_report(&app, &default_name, &html)
}

/// Export a whole plan's stored data (every task, its runs, events, and diffs)
/// as a standalone HTML report, saved via a native save dialog defaulting to a
/// name built from the plan's title, then revealed in Finder. Returns the saved
/// path, or `None` if the user cancelled the dialog.
// Async for the same reason as `export_task_report`: the blocking save dialog
// must not run on the main thread.
#[tauri::command]
async fn export_plan_report(
    app: AppHandle,
    plan_id: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let report = {
        let conn = state.db.lock().unwrap();
        loopfleet_core::plan_report(&conn, &plan_id).map_err(|e| e.to_string())?
    };
    let default_name = format!(
        "{}.html",
        report_file_stem(report.title.as_deref().unwrap_or("Plan report"))
    );
    let html = loopfleet_core::render_plan_report(&report);
    save_report(&app, &default_name, &html)
}

/// Discover the v1 agent CLIs: which are installed, their detected version, and
/// whether it matches the version the adapter was tested against. Lets the UI
/// show availability up front and warn on version drift (PRD Risks).
#[tauri::command]
async fn agent_status() -> Vec<loopfleet_adapters::AgentStatus> {
    loopfleet_adapters::discover_all().await
}

/// Every known agent's current limit headroom, one [`UsageSnapshot`] per entry
/// in `KNOWN_AGENTS` — the list is exhaustive and in a stable order, so a UI can
/// render a row per agent without cross-referencing `agent_status`.
///
/// Each agent's snapshot is resolved by preference:
///
/// 1. A **fresh adapter probe** ([`AgentAdapter::usage_snapshot`]). This is the
///    only source that reflects usage the app never saw — headroom spent by the
///    user's own terminal sessions, not just by runs launched here — so it wins
///    whenever it actually knows something. Adapters that cannot probe answer
///    `UsageUnsupported`, and `claude`'s probe degrades to an
///    [`UsageSource::Unknown`] snapshot rather than failing; both mean "no
///    answer" and fall through.
/// 2. The **stored observation** — the latest `RateLimited` notice the app saw
///    from that agent, folded into the same normalized shape. Older and
///    coarser (a limit notice says "spent", never "63% spent"), but it is real
///    evidence and outlives the run that produced it.
/// 3. An [`UsageSnapshot::unknown`], which says exactly that. Never a
///    zero-used snapshot dressed up as headroom.
///
/// Every resolved snapshot goes through [`publish_usage`], so calling this
/// primes the `agent_usage` event stream: a caller can invoke once for the
/// initial paint and then listen, and any later change — a probe here, or a
/// limit observed mid-run — arrives without polling.
#[tauri::command]
async fn agent_usage(app: AppHandle) -> Result<Vec<UsageSnapshot>, String> {
    // One read of the stored observations for the whole sweep, resolved through
    // `spec_for` so an observation filed under the `cursor-agent` alias still
    // answers for `cursor`. The store lists newest-observed first, so the first
    // entry per key is the one to keep.
    let stored: HashMap<String, loopfleet_store::AgentUsage> = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|_| "database lock poisoned")?;
        let mut by_key: HashMap<String, loopfleet_store::AgentUsage> = HashMap::new();
        for usage in loopfleet_store::list_agent_usage(&conn).map_err(|e| e.to_string())? {
            if let Some(spec) = loopfleet_adapters::spec_for(&usage.agent) {
                by_key.entry(spec.key.to_string()).or_insert(usage);
            }
        }
        by_key
    };

    let mut out = Vec::with_capacity(loopfleet_adapters::KNOWN_AGENTS.len());
    for spec in loopfleet_adapters::KNOWN_AGENTS {
        let now = now_ms();
        // Probes are sequential, like `discover_all`'s: at v1's three agents
        // only `claude` spawns anything, and its probe is bounded by its own
        // timeout.
        let probed = match build_adapter(spec.key) {
            Some(adapter) => adapter.usage_snapshot(now).await.ok(),
            None => None,
        };
        let snapshot = probed
            .filter(|s| s.source != UsageSource::Unknown)
            .or_else(|| {
                let usage = stored.get(spec.key)?;
                let notice =
                    rate_limit_notice(spec.key, usage.reset_at.as_deref(), usage.observed_at);
                Some(fold_rate_limit(None, &notice))
            })
            .unwrap_or_else(|| UsageSnapshot::unknown(spec.key, now));
        out.push(publish_usage(&app, snapshot));
    }
    Ok(out)
}

/// The result of [`check_agent_usage`]: the resolved snapshot alongside
/// whether it's safe to launch that agent right now.
#[derive(Clone, serde::Serialize)]
struct AgentUsageCheck {
    snapshot: UsageSnapshot,
    decision: LaunchDecision,
}

/// Probe a single named agent's current usage and say whether a launch should
/// proceed — the on-demand counterpart to [`agent_usage`]'s full sweep, for a
/// caller (e.g. the launch dialog) that only cares about one agent and wants
/// an answer without waiting on every other adapter's probe.
///
/// Resolved through the same three-tier fallback as `agent_usage` and
/// `resolve_agent_usage`: a fresh adapter probe (bounded by that adapter's own
/// timeout), then the stored rate-limit observation, then `unknown`. The
/// resolved snapshot is published on `agent_usage` like every other resolved
/// snapshot, so any listener sees it too — this command isn't a side channel.
#[tauri::command]
async fn check_agent_usage(agent: String, app: AppHandle) -> Result<AgentUsageCheck, String> {
    let now = now_ms();
    let probed = match build_adapter(&agent) {
        Some(adapter) => adapter.usage_snapshot(now).await.ok(),
        None => None,
    };
    let stored = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|_| "database lock poisoned")?;
        loopfleet_store::load_agent_usage(&conn, &agent).map_err(|e| e.to_string())?
    };
    let snapshot = probed
        .filter(|s| s.source != UsageSource::Unknown)
        .or_else(|| {
            let usage = stored?;
            let notice = rate_limit_notice(&agent, usage.reset_at.as_deref(), usage.observed_at);
            Some(fold_rate_limit(None, &notice))
        })
        .unwrap_or_else(|| UsageSnapshot::unknown(&agent, now));
    let snapshot = publish_usage(&app, snapshot);
    let decision = launch_decision(Some(&snapshot), now, UsageThresholds::default());
    Ok(AgentUsageCheck { snapshot, decision })
}

/// The v1 agents, dispatched by name. Boxed so the loop holds a `dyn` adapter.
fn build_adapter(agent: &str) -> Option<Box<dyn AgentAdapter>> {
    match agent {
        "claude" => Some(Box::new(ClaudeAdapter)),
        "pi" => Some(Box::new(PiAdapter)),
        "cursor" | "cursor-agent" => Some(Box::new(CursorAdapter)),
        _ => None,
    }
}

/// The `$HOME` dirs the v1 agent CLIs write to (config, cache, session state).
/// Granted in the sandbox so a confined agent can start. A superset across the
/// v1 agents; nonexistent subpaths are harmless in a Seatbelt grant.
fn agent_dirs() -> Vec<PathBuf> {
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return Vec::new(),
    };
    [".claude", ".claude.json", ".config", ".cache", ".pi", ".cursor"]
        .iter()
        .map(|d| home.join(d))
        .collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // A Finder/Dock-launched .app inherits launchd's minimal PATH, which hides
    // the agent CLIs that a terminal-launched dev run finds. Repair PATH before
    // anything spawns, so discovery and the runs themselves see the same
    // binaries the user's shell does.
    path_env::repair();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = loopfleet_store::open(dir.join("loopfleet.db"))?;

            // Crash recovery: runs don't survive an app restart in v1, so any run
            // still marked queued/running was interrupted by a prior crash or
            // quit — its background task and agent process are gone. Mark them
            // failed (shadow refs are kept). Then prune orphan worktree metadata
            // for each project (worktrees whose checkout vanished on the crash).
            let interrupted = loopfleet_store::fail_interrupted_runs(&conn).unwrap_or_default();
            if !interrupted.is_empty() {
                eprintln!(
                    "crash recovery: marked {} interrupted run(s) failed",
                    interrupted.len()
                );
            }
            let repos: Vec<String> = loopfleet_store::list_projects(&conn)
                .map(|ps| ps.into_iter().map(|p| p.repo_path).collect())
                .unwrap_or_default();

            let git = GitActor::spawn();
            let db = Arc::new(Mutex::new(conn));

            // After orphaned worktree *metadata* is pruned per-repo, sweep
            // finished runs' on-disk footprint (worktree/profile/progress dir)
            // and any worktree directory with no run row at all. Then keep
            // sweeping hourly for the life of the app.
            let sweep_git = git.clone();
            let sweep_db = db.clone();
            let sweep_dir = dir.clone();
            tauri::async_runtime::spawn(async move {
                for repo in repos {
                    let _ = sweep_git.cleanup_orphans(PathBuf::from(repo)).await;
                }
                sweep_worktrees(&sweep_db, &sweep_git, &sweep_dir).await;

                let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
                ticker.tick().await; // first tick fires immediately; already swept above
                loop {
                    ticker.tick().await;
                    sweep_worktrees(&sweep_db, &sweep_git, &sweep_dir).await;
                }
            });

            app.manage(AppState {
                db,
                git,
                data_dir: dir,
                stops: Arc::new(Mutex::new(HashMap::new())),
                edits: Arc::new(Mutex::new(HashMap::new())),
                unacknowledged_runs: Arc::new(AtomicI64::new(0)),
                scheduled_resumes: Arc::new(Mutex::new(HashMap::new())),
                scheduled_launches: Arc::new(Mutex::new(HashMap::new())),
                scheduled_auto_merges: Arc::new(Mutex::new(HashMap::new())),
                published_usage: Arc::new(Mutex::new(HashMap::new())),
            });

            // Recover any rate-limit resume a crash or quit interrupted mid-wait
            // (see `rearm_pending_resumes`), so the resume chip and its Cancel
            // action reappear exactly as they were before the restart.
            rearm_pending_resumes(&app.handle().clone());
            // Same recovery for user-scheduled launches (see
            // `rearm_scheduled_launches`).
            rearm_scheduled_launches(&app.handle().clone());

            // Regaining focus counts as acknowledging any runs that finished
            // while the user was away — clear the dock badge along with it.
            if let Some(window) = app.get_webview_window("main") {
                let unacknowledged = app.state::<AppState>().unacknowledged_runs.clone();
                let badge_window = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(true) = event {
                        unacknowledged.store(0, Ordering::SeqCst);
                        #[cfg(target_os = "macos")]
                        let _ = badge_window.set_badge_count(None);
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            register_project,
            list_projects,
            remove_project,
            project_removal_preview,
            agent_status,
            agent_usage,
            check_agent_usage,
            get_settings,
            save_settings,
            project_sandbox_writes,
            set_project_sandbox_writes,
            plan_overview,
            plan_document,
            plan_edit,
            plan_edit_apply,
            plan_edit_discard,
            launch_run,
            plan_runs,
            run_timeline,
            stop_run,
            sweep_worktrees_now,
            cancel_scheduled_resume,
            schedule_launch,
            cancel_scheduled_launch,
            cancel_auto_merge,
            acknowledge_runs,
            compare_task,
            use_run,
            export_task_report,
            export_plan_report
        ])
        .run(tauri::generate_context!())
        .expect("error while running loopfleet");
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- agent usage ---

    const NOW_MS: i64 = 1_700_000_000_000;

    #[test]
    fn reset_at_parses_rfc3339_into_epoch_millis() {
        assert_eq!(reset_at_ms(Some("1970-01-01T00:00:01Z")), Some(1_000));
        assert_eq!(
            reset_at_ms(Some("2025-01-15T10:00:00Z")),
            Some(1_736_935_200_000)
        );
    }

    /// An agent writes the reset string itself, so unparseable is expected, not
    /// exceptional: it degrades to "limited, reset time unknown".
    #[test]
    fn an_unparseable_or_absent_reset_is_simply_unknown() {
        assert_eq!(reset_at_ms(None), None);
        assert_eq!(reset_at_ms(Some("whenever")), None);
        assert_eq!(reset_at_ms(Some("")), None);
    }

    /// Our agents report the fact of a limit, never a fraction — so the notice
    /// carries none, and folding reads it as inferred-exhausted rather than
    /// claiming the agent gave us a number.
    #[test]
    fn a_limit_notice_carries_no_fraction_and_folds_to_inferred_exhausted() {
        let notice = rate_limit_notice("claude", Some("2025-01-15T10:00:00Z"), NOW_MS);
        assert_eq!(notice.agent_key, "claude");
        assert_eq!(notice.used_fraction, None);
        assert_eq!(notice.reset_at_ms, Some(1_736_935_200_000));
        assert_eq!(notice.observed_at_ms, NOW_MS);

        let folded = fold_rate_limit(None, &notice);
        assert_eq!(folded.source, UsageSource::Inferred);
        assert_eq!(folded.used_fraction, 1.0);
        assert_eq!(folded.reset_at_ms, Some(1_736_935_200_000));
    }

    #[test]
    fn a_notice_without_a_usable_reset_still_marks_the_agent_limited() {
        let folded = fold_rate_limit(None, &rate_limit_notice("pi", Some("soon"), NOW_MS));
        assert_eq!(folded.reset_at_ms, None);
        assert_eq!(folded.source, UsageSource::Inferred);
        assert_eq!(folded.used_fraction, 1.0);
    }

    /// Re-probing an agent whose standing has not moved must not wake the UI:
    /// only `observed_at_ms` differs, and that is not news.
    #[test]
    fn a_restamped_but_identical_snapshot_is_not_a_change() {
        let before = UsageSnapshot::reported("claude", 0.42, NOW_MS);
        let after = UsageSnapshot::reported("claude", 0.42, NOW_MS + 60_000);
        assert!(!usage_changed(&before, &after));
    }

    #[test]
    fn every_rendered_field_counts_as_a_change() {
        let before = UsageSnapshot::reported("claude", 0.42, NOW_MS)
            .with_model("opus")
            .with_limit_window("5h")
            .with_reset_at(NOW_MS + 60_000);

        let cases = [
            UsageSnapshot::reported("codex", 0.42, NOW_MS)
                .with_model("opus")
                .with_limit_window("5h")
                .with_reset_at(NOW_MS + 60_000),
            before.clone().with_model("sonnet"),
            before.clone().with_limit_window("weekly"),
            UsageSnapshot::reported("claude", 0.43, NOW_MS)
                .with_model("opus")
                .with_limit_window("5h")
                .with_reset_at(NOW_MS + 60_000),
            before.clone().with_reset_at(NOW_MS + 120_000),
            UsageSnapshot::unknown("claude", NOW_MS),
        ];
        for after in cases {
            assert!(
                usage_changed(&before, &after),
                "{after:?} should be a change"
            );
        }
    }

    /// The distinction the meter hangs on: a zero-used `Unknown` snapshot and a
    /// genuine zero-used `Reported` one must not be conflated.
    #[test]
    fn unknown_and_reported_zero_are_different_snapshots() {
        let unknown = UsageSnapshot::unknown("claude", NOW_MS);
        let reported = UsageSnapshot::reported("claude", 0.0, NOW_MS);
        assert_eq!(unknown.used_fraction, reported.used_fraction);
        assert!(usage_changed(&unknown, &reported));
    }

    // --- rate-limit resume scheduling ---

    #[test]
    fn no_reschedule_without_a_parseable_reset_time() {
        let now = OffsetDateTime::now_utc();
        let buffer = resume_buffer(1);
        assert!(delay_until(None, now, buffer).is_none());
        assert!(delay_until(Some("whenever"), now, buffer).is_none());
    }

    #[test]
    fn no_reschedule_when_the_reset_is_already_past() {
        let now = OffsetDateTime::parse("2025-01-15T10:00:00Z", &Rfc3339).unwrap();
        // Even with the one-minute buffer added, this reset is still in the past.
        assert!(delay_until(Some("2025-01-15T09:58:00Z"), now, resume_buffer(1)).is_none());
    }

    #[test]
    fn delay_is_the_gap_to_a_future_reset_plus_a_one_minute_buffer() {
        let now = OffsetDateTime::parse("2025-01-15T10:00:00Z", &Rfc3339).unwrap();
        let delay = delay_until(Some("2025-01-15T10:05:00Z"), now, resume_buffer(1)).unwrap();
        assert_eq!(delay, std::time::Duration::from_secs(300 + 60));
    }

    #[test]
    fn resume_buffer_doubles_per_attempt() {
        assert_eq!(resume_buffer(1), time::Duration::seconds(60));
        assert_eq!(resume_buffer(2), time::Duration::seconds(120));
        assert_eq!(resume_buffer(3), time::Duration::seconds(240));
    }

    /// How long a test's stand-in process stays parked in its directory. Short,
    /// since every test that spawns one also waits for it to exit; the checks
    /// against it run within a few hundred milliseconds of the spawn.
    const PARKED_LIFETIME_SECS: u64 = 3;

    /// A child process parked in `dir` for `PARKED_LIFETIME_SECS` — stands in
    /// for the shell or editor a user left open inside a worktree.
    struct ProcessIn(std::process::Child);

    impl ProcessIn {
        fn new(dir: &std::path::Path) -> Self {
            let child = std::process::Command::new("sleep")
                .arg(PARKED_LIFETIME_SECS.to_string())
                .current_dir(dir)
                .spawn()
                .expect("spawn sleep");
            // Give the child a moment to actually be running before anyone
            // asks `lsof` to find it.
            std::thread::sleep(std::time::Duration::from_millis(200));
            Self(child)
        }

        /// Block until the child is gone. We let it time out on its own rather
        /// than signal it: sending signals can be denied (e.g. under a sandbox),
        /// and a blocking `wait` on a child we failed to kill would hang.
        fn wait_until_gone(mut self) {
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_secs(PARKED_LIFETIME_SECS + 10);
            while std::time::Instant::now() < deadline {
                match self.0.try_wait() {
                    Ok(Some(_)) => return,
                    _ => std::thread::sleep(std::time::Duration::from_millis(100)),
                }
            }
            panic!("parked process outlived its sleep");
        }
    }

    #[test]
    fn an_idle_worktree_is_not_in_use() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!worktree_in_use(dir.path().to_str().unwrap()));
    }

    #[test]
    fn a_worktree_that_is_a_live_process_cwd_is_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let parked = ProcessIn::new(dir.path());
        assert!(worktree_in_use(dir.path().to_str().unwrap()));
        parked.wait_until_gone();
    }

    #[test]
    fn a_worktree_is_free_again_once_the_process_exits() {
        let dir = tempfile::tempdir().unwrap();
        let parked = ProcessIn::new(dir.path());
        assert!(worktree_in_use(dir.path().to_str().unwrap()));
        parked.wait_until_gone();
        assert!(!worktree_in_use(dir.path().to_str().unwrap()));
    }

    #[test]
    fn a_worktree_already_gone_is_not_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap().to_string();
        drop(dir);
        assert!(!worktree_in_use(&path));
    }
}
