use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use loopfleet_adapters::{ClaudeAdapter, CursorAdapter, PiAdapter};
use loopfleet_core::{
    run_loop, AgentAdapter, CompareView, LoopConfig, NormalizedEvent, PlanView, RunSpec, RunState,
    RunTimeline,
};
use loopfleet_gitx::GitActor;
use loopfleet_sandbox::{confine_prefix, RenderParams};
use loopfleet_store::{Connection, NewRun, Project, RunSummary};
use tauri::{AppHandle, Emitter, Manager, State};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::sync::watch;

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

/// The global app settings (default agent, default iteration count, concurrency
/// cap). Unset fields fall back to code defaults.
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
        max_iterations,
        app,
        state.db.clone(),
        state.git.clone(),
        state.data_dir.clone(),
        state.stops.clone(),
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
    max_iterations: u32,
    app: AppHandle,
    db: Arc<Mutex<Connection>>,
    git: GitActor,
    data_dir: PathBuf,
    stops: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
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
    let rerun = (project_id, task_anchor.clone(), agent.clone(), max_iterations);

    {
        let conn = db.lock().unwrap();
        loopfleet_store::insert_run(
            &conn,
            &NewRun {
                id: run_id.clone(),
                plan_id,
                task_anchor,
                agent,
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
    let sched = (app.clone(), db.clone(), git.clone(), data_dir.clone(), stops.clone());
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

        // A run that ended limit-reached waits out the rate limit: if the agent
        // gave a reset time still in the future, schedule a fresh re-run of the
        // same task at that time. Held only in memory — like every run, a pending
        // re-run does not survive an app restart. No (or already-past) reset time
        // means we can't know it is safe to retry, so we leave it for the user.
        if outcome.state == RunState::LimitReached {
            if let Some(delay) = delay_until(outcome.reset_at.as_deref(), OffsetDateTime::now_utc()) {
                let (app, db, git, data_dir, stops) = sched;
                let (project_id, task_anchor, agent, max_iterations) = rerun;
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let _ = spawn_run(
                        project_id, task_anchor, agent, max_iterations,
                        app, db, git, data_dir, stops,
                    )
                    .await;
                });
            }
        }
    });

    Ok(run_id)
    })
}

/// How long to wait before re-running a rate-limited run: the gap between `now`
/// and the agent-reported `reset_at` (ISO-8601 / RFC 3339). `None` when there is
/// no reset time, it doesn't parse, or it is already in the past — i.e. "don't
/// auto-reschedule". We only retry when we know a future instant the limit lifts,
/// so a re-run never hammers a still-exhausted limit.
fn delay_until(reset_at: Option<&str>, now: OffsetDateTime) -> Option<std::time::Duration> {
    let reset = OffsetDateTime::parse(reset_at?, &Rfc3339).ok()?;
    // `TryFrom<time::Duration>` fails for a negative span, so a past reset → None.
    std::time::Duration::try_from(reset - now).ok()
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
    merged_commit: String,
    created: bool,
    up_to_date: bool,
}

/// "Use this run": merge the run's final state into a target branch and mark
/// the run accepted. `target_branch = None` (or empty) merges into the repo's
/// currently checked-out branch — the default, landing the run's work where the
/// user is working under a descriptive merge commit. A non-empty `target_branch`
/// names a custom branch (created if absent). The merge runs through the
/// serialized git actor; the current-branch default merges in the main worktree
/// (guarded by a clean tree), a custom target uses a throwaway worktree so the
/// user's own checkout is never touched.
#[tauri::command]
async fn use_run(
    run_id: String,
    target_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<UseRunResult, String> {
    let target = target_branch
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());

    // Resolve the run's parent repo, its final shadow ref, and the identity
    // pieces that make the merge commit message descriptive.
    let (repo_path, source_ref, agent, task_anchor) = {
        let conn = state.db.lock().unwrap();
        let detail = loopfleet_store::load_run(&conn, &run_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("unknown run: {run_id}"))?;
        let source_ref = loopfleet_store::load_iterations(&conn, &run_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .rev()
            .find_map(|it| it.shadow_ref)
            .ok_or_else(|| "run has no snapshot to use".to_string())?;
        (detail.repo_path, source_ref, detail.agent, detail.task_anchor)
    };

    // A nice merge commit message: subject names the run and agent, body carries
    // the task so the history reads as what the run accomplished.
    let short = &run_id[..run_id.len().min(8)];
    let message = format!("Apply loopfleet run {short} ({agent})\n\n{task_anchor}");

    let scratch_root = state.data_dir.join("worktrees");
    let merge = state
        .git
        .merge_run(
            PathBuf::from(&repo_path),
            source_ref,
            target,
            message,
            scratch_root,
        )
        .await
        .map_err(|e| e.to_string())?;

    {
        let conn = state.db.lock().unwrap();
        loopfleet_store::set_run_accepted(&conn, &run_id).map_err(|e| e.to_string())?;
    }

    Ok(UseRunResult {
        target_branch: merge.target_branch,
        merged_commit: merge.merged_commit,
        created: merge.created,
        up_to_date: merge.up_to_date,
    })
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

/// Discover the v1 agent CLIs: which are installed, their detected version, and
/// whether it matches the version the adapter was tested against. Lets the UI
/// show availability up front and warn on version drift (PRD Risks).
#[tauri::command]
async fn agent_status() -> Vec<loopfleet_adapters::AgentStatus> {
    loopfleet_adapters::discover_all().await
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
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
            let prune_git = git.clone();
            tauri::async_runtime::spawn(async move {
                for repo in repos {
                    let _ = prune_git.cleanup_orphans(PathBuf::from(repo)).await;
                }
            });

            app.manage(AppState {
                db: Arc::new(Mutex::new(conn)),
                git,
                data_dir: dir,
                stops: Arc::new(Mutex::new(HashMap::new())),
                edits: Arc::new(Mutex::new(HashMap::new())),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            register_project,
            list_projects,
            agent_status,
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
            compare_task,
            use_run
        ])
        .run(tauri::generate_context!())
        .expect("error while running loopfleet");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_reschedule_without_a_parseable_reset_time() {
        let now = OffsetDateTime::now_utc();
        assert!(delay_until(None, now).is_none());
        assert!(delay_until(Some("whenever"), now).is_none());
    }

    #[test]
    fn no_reschedule_when_the_reset_is_already_past() {
        let now = OffsetDateTime::parse("2025-01-15T10:00:00Z", &Rfc3339).unwrap();
        assert!(delay_until(Some("2025-01-15T09:59:00Z"), now).is_none());
    }

    #[test]
    fn delay_is_the_gap_to_a_future_reset() {
        let now = OffsetDateTime::parse("2025-01-15T10:00:00Z", &Rfc3339).unwrap();
        let delay = delay_until(Some("2025-01-15T10:05:00Z"), now).unwrap();
        assert_eq!(delay, std::time::Duration::from_secs(300));
    }
}
