use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use loopfleet_adapters::{ClaudeAdapter, CursorAdapter, PiAdapter};
use loopfleet_core::{
    run_loop, AgentAdapter, LoopConfig, NormalizedEvent, PlanView, RunState, RunTimeline,
};
use loopfleet_gitx::GitActor;
use loopfleet_sandbox::{confine_prefix, RenderParams};
use loopfleet_store::{Connection, NewRun, Project, RunSummary};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::watch;

/// App-owned state shared across commands. The connection is behind
/// `Arc<Mutex<…>>` so a background launch task can persist run progress on the
/// same single writer the commands use (SQLite is single-writer by design). The
/// git actor serializes all mutating git ops; `data_dir` roots the app-managed
/// worktrees, progress files, and sandbox profiles. `stops` holds a cancel
/// sender per active run so the live-run Stop button can signal it.
struct AppState {
    db: Arc<Mutex<Connection>>,
    git: GitActor,
    data_dir: PathBuf,
    stops: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
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

/// The plan overview for a project: its plan(s) with a derived `TaskStatus`
/// overlay per task. Syncs plan + tasks into the store as a side effect (so runs
/// can bind to them); never edits the frozen plan file.
#[tauri::command]
fn plan_overview(project_id: String, state: State<'_, AppState>) -> Result<Vec<PlanView>, String> {
    let conn = state.db.lock().unwrap();
    let project = get_project(&conn, &project_id)?;
    loopfleet_core::plan_overview(&conn, &project).map_err(|e| e.to_string())
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
    let adapter = build_adapter(&agent).ok_or_else(|| format!("unknown agent: {agent}"))?;

    // Resolve the bound task's text and stable plan id. plan_overview also syncs
    // the plan + tasks into the store, so the run's FK resolves on insert.
    let (project, plan_id, task_text) = {
        let conn = state.db.lock().unwrap();
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
        (project, plan_id, task_text)
    };

    // App-managed paths, keyed by run id (outside the repo).
    let run_id = uuid::Uuid::new_v4().to_string();
    let worktrees_root = state.data_dir.join("worktrees");
    let progress_dir = state.data_dir.join("progress").join(&run_id);
    let progress_path = progress_dir.join("progress.md");
    let profile_path = state.data_dir.join("profiles").join(format!("{run_id}.sb"));
    std::fs::create_dir_all(&worktrees_root).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&progress_dir).map_err(|e| e.to_string())?;

    // Cut the per-run worktree through the serialized git actor.
    let worktree = state
        .git
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
    let wrapper = confine_prefix(&params, &profile_path).map_err(|e| e.to_string())?;

    {
        let conn = state.db.lock().unwrap();
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
    state.stops.lock().unwrap().insert(run_id.clone(), cancel_tx);

    // Drive the loop off the command's response: it may run for minutes. Progress
    // is persisted on the shared single-writer connection and streamed to the UI.
    let db = state.db.clone();
    let git = state.git.clone();
    let stops = state.stops.clone();
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
    });

    Ok(run_id)
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
            app.manage(AppState {
                db: Arc::new(Mutex::new(conn)),
                git: GitActor::spawn(),
                data_dir: dir,
                stops: Arc::new(Mutex::new(HashMap::new())),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            register_project,
            list_projects,
            plan_overview,
            launch_run,
            plan_runs,
            run_timeline,
            stop_run
        ])
        .run(tauri::generate_context!())
        .expect("error while running loopfleet");
}
