use std::sync::Mutex;

use loopfleet_store::{Connection, Project};
use tauri::{Manager, State};

/// App-owned SQLite connection. Guarded by a mutex because commands run on
/// arbitrary threads; the store is single-writer by design.
struct AppState {
    db: Mutex<Connection>,
}

/// Validate `path` is a git repo and persist it as a project. Returns the
/// stored project, or a user-facing error string.
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = loopfleet_store::open(dir.join("loopfleet.db"))?;
            app.manage(AppState { db: Mutex::new(conn) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![register_project, list_projects])
        .run(tauri::generate_context!())
        .expect("error while running loopfleet");
}
