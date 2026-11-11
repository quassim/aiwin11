#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{fs, sync::{Arc, Mutex}};

use aiwin11_core::{
    local_files::LocalFiles,
    model_gateway::{LocalModelProvider, Ollama},
    openai,
    policy::{self, AgentAction, Effect, PolicySettings},
    task::{TaskOutcome, TaskRequest, TaskSlot, TaskUpdate},
    windows_bridge::{Capture, Inspection},
    Core,
};
use serde::Serialize;
use tauri::{ipc::Channel, Manager, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    session_id: String,
    events: Vec<Activity>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Activity {
    sequence: u64,
    kind: String,
    timestamp: String,
    task_id: Option<String>,
    code: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskEntry {
    id: String,
    state: String,
    outcome: Option<String>,
    created_at: String,
    updated_at: String,
}

/// Recent chat tasks across sessions, including tasks interrupted by a crash.
#[tauri::command]
async fn get_tasks(state: State<'_, Arc<AppState>>) -> Result<Vec<TaskEntry>, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let core = state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?;
        let tasks = core.recent_tasks(30).map_err(|_| "ACTIVITY_UNAVAILABLE")?;
        Ok(tasks.into_iter().map(|task| TaskEntry {
            id: task.id, state: task.state, outcome: task.outcome,
            created_at: task.created_at, updated_at: task.updated_at,
        }).collect())
    }).await.map_err(|_| "CORE_UNAVAILABLE")?
}

struct AppState {
    core: Mutex<Core>,
    files: Option<LocalFiles>,
    task: TaskSlot,
    vision: Mutex<()>,
}

#[tauri::command]
fn local_files_root(state: State<'_, Arc<AppState>>) -> Result<String, &'static str> {
    let path = state.files.as_ref().ok_or("FILE_UNAVAILABLE")?.root().to_string_lossy();
    Ok(path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned())
}

#[tauri::command]
async fn preview_local_text(state: State<'_, Arc<AppState>>, name: String) -> Result<String, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let files = state.files.as_ref().ok_or("FILE_UNAVAILABLE")?;
        let mut core = state.core.try_lock().map_err(|_| "CORE_BUSY")?;
        files.preview(&mut core, &name)
    }).await.map_err(|_| "FILE_UNAVAILABLE")?
}

#[tauri::command]
async fn copy_local_text(
    state: State<'_, Arc<AppState>>, name: String, destination: String,
) -> Result<(), &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let files = state.files.as_ref().ok_or("FILE_UNAVAILABLE")?;
        let mut core = state.core.try_lock().map_err(|_| "CORE_BUSY")?;
        files.copy(&mut core, &name, &destination)
    }).await.map_err(|_| "FILE_UNAVAILABLE")?
}

#[tauri::command]
async fn get_snapshot(state: State<'_, Arc<AppState>>) -> Result<Snapshot, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || snapshot(&state))
        .await.map_err(|_| "CORE_UNAVAILABLE")?
}

fn snapshot(state: &AppState) -> Result<Snapshot, &'static str> {
    let core = state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?;
    let events = core.activity().map_err(|_| "ACTIVITY_UNAVAILABLE")?;
    Ok(Snapshot {
        session_id: core.session_id().into(),
        events: events
            .into_iter()
            .map(|event| Activity {
                sequence: event.sequence,
                kind: event.kind,
                timestamp: event.timestamp,
                task_id: event.task_id,
                code: event.code,
            })
            .collect(),
    })
}

#[tauri::command]
async fn discover_models() -> Result<Vec<String>, &'static str> {
    tauri::async_runtime::spawn_blocking(|| {
        Ollama::connect()
            .and_then(|provider| provider.discover())
            .map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")
    }).await.map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?
}

#[tauri::command]
async fn inspect_foreground(state: State<'_, Arc<AppState>>) -> Result<Inspection, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        state.core.try_lock().map_err(|_| "CORE_BUSY")?
            .inspect_foreground().map_err(|_| "INSPECTION_UNAVAILABLE")
    }).await.map_err(|_| "CORE_UNAVAILABLE")?
}

#[tauri::command]
async fn capture_foreground(state: State<'_, Arc<AppState>>) -> Result<Capture, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        state.core.try_lock().map_err(|_| "CORE_BUSY")?
            .capture_foreground().map_err(|_| "CAPTURE_UNAVAILABLE")
    }).await.map_err(|_| "CORE_UNAVAILABLE")?
}

#[tauri::command]
async fn enter_character_map_digit(
    state: State<'_, Arc<AppState>>, digit: u32,
) -> Result<bool, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        state.core.try_lock().map_err(|_| "CORE_BUSY")?.enter_character_map_digit(digit)
    }).await.map_err(|_| "CORE_UNAVAILABLE")?
}

#[tauri::command]
async fn discover_vision_models() -> Result<Vec<String>, &'static str> {
    tauri::async_runtime::spawn_blocking(|| {
        Ollama::connect().and_then(|provider| provider.vision_models())
            .map_err(|error| error.vision_code())
    }).await.map_err(|_| "VISION_UNAVAILABLE")?
}

#[tauri::command]
async fn analyze_capture(
    state: State<'_, Arc<AppState>>, model: String, jpeg: Vec<u8>,
) -> Result<String, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let _analysis = state.vision.try_lock().map_err(|_| "VISION_BUSY")?;
        Core::analyze_capture(&state.core, &model, &jpeg)
    }).await.map_err(|_| "VISION_UNAVAILABLE")?
}

#[tauri::command]
async fn run_task(
    state: State<'_, Arc<AppState>>,
    request: TaskRequest,
    progress: Channel<TaskUpdate>,
) -> Result<TaskOutcome, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        state.task.run(&state.core, state.files.as_ref(), &request, |update| {
            let _ = progress.send(update);
        })
    })
        .await.map_err(|_| "CORE_UNAVAILABLE")?
}

#[tauri::command]
fn openai_key_configured() -> Result<bool, &'static str> {
    openai::has_key()
}

#[tauri::command]
fn save_openai_key(key: String) -> Result<(), &'static str> {
    openai::save_key(key)
}

#[tauri::command]
fn delete_openai_key() -> Result<(), &'static str> {
    openai::delete_key()
}

#[tauri::command]
fn cancel_task(state: State<'_, Arc<AppState>>) -> bool {
    state.task.cancel()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyRow {
    action: AgentAction,
    effects: [Effect; 4],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyOverview {
    settings: PolicySettings,
    /// Effect of each action in the Observation, Assisted, Limited and Advanced profiles.
    table: Vec<PolicyRow>,
}

#[tauri::command]
fn get_policy_overview(state: State<'_, Arc<AppState>>) -> Result<PolicyOverview, &'static str> {
    let settings = state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.policy_settings();
    let table = policy::table(settings.simulate).into_iter()
        .map(|(action, effects)| PolicyRow { action, effects }).collect();
    Ok(PolicyOverview { settings, table })
}

/// Applies to the next task; a running task keeps the settings it started with.
#[tauri::command]
fn set_policy_settings(state: State<'_, Arc<AppState>>, settings: PolicySettings) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?
        .set_policy_settings(settings).map_err(|_| "SETTINGS_UNAVAILABLE")
}

/// The user's answer to the action the agent proposed for the running task.
#[tauri::command]
fn resolve_approval(
    state: State<'_, Arc<AppState>>, approval_id: String, approved: bool,
) -> Result<(), &'static str> {
    state.task.resolve_approval(&approval_id, approved)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            app.manage(Arc::new(AppState {
                files: LocalFiles::open(&data_dir).ok(),
                core: Mutex::new(Core::open(&data_dir.join("aiwin11.sqlite3"))?),
                task: TaskSlot::default(),
                vision: Mutex::new(()),
            }));
            // Pay the bridge's first start (runtime load, antivirus scan) now, not within a
            // command's deadline. The spare process waits idle for an authorized command.
            std::thread::spawn(aiwin11_core::windows_bridge::prewarm_bridge);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot, discover_models, run_task, cancel_task, resolve_approval,
            get_policy_overview, set_policy_settings, get_tasks,
            openai_key_configured, save_openai_key, delete_openai_key, inspect_foreground,
            capture_foreground, discover_vision_models, analyze_capture, enter_character_map_digit,
            local_files_root, preview_local_text, copy_local_text,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AiWin11");
}
