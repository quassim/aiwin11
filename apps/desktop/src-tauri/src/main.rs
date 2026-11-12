#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    fs,
    sync::{Arc, Mutex},
};

use aiwin11_core::{
    grants::Grant,
    local_files::LocalFiles,
    manifest::{ToolManifest, MANIFESTS},
    model_gateway::{LocalModelProvider, LocalStrategy, Ollama},
    openai,
    policy::{self, AgentAction, Effect, PolicySettings},
    task::{StopStatus, TaskOutcome, TaskRequest, TaskSlot, TaskUpdate},
    windows_bridge::{Capture, Inspection},
    Core, ModelPreferences,
};
use serde::Serialize;
use tauri::{ipc::Channel, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

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
        let core = lock_core(&state.core);
        let tasks = core.recent_tasks(30).map_err(|_| "ACTIVITY_UNAVAILABLE")?;
        Ok(tasks
            .into_iter()
            .map(|task| TaskEntry {
                id: task.id,
                state: task.state,
                outcome: task.outcome,
                created_at: task.created_at,
                updated_at: task.updated_at,
            })
            .collect())
    })
    .await
    .map_err(|_| "CORE_UNAVAILABLE")?
}

/// Locks the core even after a panic in another command (a poisoned mutex): a failed command
/// must not take the whole application down. SQLite rolls back any unfinished transaction.
fn lock_core(core: &Mutex<Core>) -> std::sync::MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct AppState {
    core: Mutex<Core>,
    files: Option<LocalFiles>,
    task: TaskSlot,
    vision: Mutex<()>,
    video: Mutex<VideoJob>,
}

#[derive(Clone, Default, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
enum VideoJob {
    #[default]
    Idle,
    Submitting,
    Queued,
    Completed {
        filename: String,
    },
    Failed {
        code: &'static str,
    },
}

#[tauri::command]
fn video_status(state: State<'_, Arc<AppState>>) -> Result<VideoJob, &'static str> {
    state.video.lock().map(|job| job.clone()).map_err(|_| "VIDEO_UNAVAILABLE")
}

#[tauri::command]
fn start_video(
    state: State<'_, Arc<AppState>>,
    port: u16,
    workflow: String,
    prompt: String,
) -> Result<(), &'static str> {
    let request = aiwin11_core::video::VideoRequest::new(port, &workflow, &prompt)?;
    let state = Arc::clone(state.inner());
    let mut job = state.video.lock().map_err(|_| "VIDEO_UNAVAILABLE")?;
    if matches!(*job, VideoJob::Submitting | VideoJob::Queued) {
        return Err("VIDEO_BUSY");
    }
    let task = lock_core(&state.core).authorize_video()?;
    *job = VideoJob::Submitting;
    drop(job);
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = request.run(|| {
            if let Ok(mut job) = state.video.lock() {
                *job = VideoJob::Queued;
            }
        });
        let audited = lock_core(&state.core).finish_video(task.as_deref(), &outcome);
        if let Ok(mut job) = state.video.lock() {
            *job = match audited.and(outcome) {
                Ok(filename) => VideoJob::Completed { filename },
                Err(code) => VideoJob::Failed { code },
            };
        }
    });
    Ok(())
}

const UPDATE_KEY: Option<&str> = option_env!("AIWIN11_UPDATE_PUBLIC_KEY");
const UPDATE_FEED: &str = "https://github.com/quassim/aiwin11/releases/latest/download/latest.json";

#[tauri::command]
fn updates_configured() -> bool {
    UPDATE_KEY.is_some_and(|key| !key.trim().is_empty())
}

async fn find_update(app: &tauri::AppHandle, timeout: u64) -> Result<Option<Update>, &'static str> {
    let key = UPDATE_KEY.filter(|key| !key.trim().is_empty()).ok_or("UPDATE_NOT_CONFIGURED")?;
    let endpoint = UPDATE_FEED.parse().map_err(|_| "UPDATE_UNAVAILABLE")?;
    let update = app
        .updater_builder()
        .pubkey(key.trim())
        .timeout(std::time::Duration::from_secs(timeout))
        .endpoints(vec![endpoint])
        .and_then(|builder| builder.build())
        .map_err(|_| "UPDATE_UNAVAILABLE")?
        .check()
        .await
        .map_err(|_| "UPDATE_UNAVAILABLE")?;
    if update.as_ref().is_some_and(|update| {
        let url = &update.download_url;
        url.scheme() != "https"
            || url.host_str() != Some("github.com")
            || !url.path().starts_with("/quassim/aiwin11/releases/download/")
    }) {
        return Err("UPDATE_UNAVAILABLE");
    }
    Ok(update)
}

#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Result<Option<String>, &'static str> {
    Ok(find_update(&app, 15).await?.map(|update| update.version))
}

#[tauri::command]
async fn install_update(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    version: String,
) -> Result<(), &'static str> {
    if state.task.is_running() {
        return Err("UPDATE_TASK_RUNNING");
    }
    let update = find_update(&app, 120).await?.ok_or("UPDATE_CHANGED")?;
    if update.version != version {
        return Err("UPDATE_CHANGED");
    }
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|_| "UPDATE_DOWNLOAD_FAILED")?;
    state.task.while_idle(|| update.install(bytes)).ok_or("UPDATE_TASK_RUNNING")?.map_err(|_| "UPDATE_INSTALL_FAILED")
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
    })
    .await
    .map_err(|_| "FILE_UNAVAILABLE")?
}

#[tauri::command]
async fn copy_local_text(
    state: State<'_, Arc<AppState>>,
    name: String,
    destination: String,
) -> Result<(), &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let files = state.files.as_ref().ok_or("FILE_UNAVAILABLE")?;
        let mut core = state.core.try_lock().map_err(|_| "CORE_BUSY")?;
        files.copy(&mut core, &name, &destination)
    })
    .await
    .map_err(|_| "FILE_UNAVAILABLE")?
}

#[tauri::command]
async fn move_local_text(
    state: State<'_, Arc<AppState>>,
    name: String,
    destination: String,
) -> Result<(), &'static str> {
    with_local_files(state, move |files, core| files.move_file(core, &name, &destination)).await
}

#[tauri::command]
async fn trash_local_text(state: State<'_, Arc<AppState>>, name: String) -> Result<(), &'static str> {
    with_local_files(state, move |files, core| files.trash_file(core, &name)).await
}

#[tauri::command]
async fn list_local_text(state: State<'_, Arc<AppState>>, query: Option<String>) -> Result<Vec<String>, &'static str> {
    with_local_files(state, move |files, core| files.list_files(core, query.as_deref().unwrap_or(""))).await
}

#[tauri::command]
async fn list_trash(state: State<'_, Arc<AppState>>) -> Result<Vec<(String, String)>, &'static str> {
    with_local_files(state, LocalFiles::list_trash).await
}

#[tauri::command]
async fn restore_local_text(state: State<'_, Arc<AppState>>, id: String, name: String) -> Result<(), &'static str> {
    with_local_files(state, move |files, core| files.restore(core, &id, &name)).await
}

async fn with_local_files<T: Send + 'static>(
    state: State<'_, Arc<AppState>>,
    execute: impl FnOnce(&LocalFiles, &mut Core) -> Result<T, &'static str> + Send + 'static,
) -> Result<T, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let files = state.files.as_ref().ok_or("FILE_UNAVAILABLE")?;
        let mut core = state.core.try_lock().map_err(|_| "CORE_BUSY")?;
        execute(files, &mut core)
    })
    .await
    .map_err(|_| "FILE_UNAVAILABLE")?
}

#[tauri::command]
async fn get_snapshot(state: State<'_, Arc<AppState>>) -> Result<Snapshot, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || snapshot(&state)).await.map_err(|_| "CORE_UNAVAILABLE")?
}

fn snapshot(state: &AppState) -> Result<Snapshot, &'static str> {
    let core = lock_core(&state.core);
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
        Ollama::connect().and_then(|provider| provider.discover()).map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")
    })
    .await
    .map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?
}

#[tauri::command]
async fn list_displays(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<aiwin11_core::windows_bridge::Display>, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || state.core.try_lock().map_err(|_| "CORE_BUSY")?.list_displays())
        .await
        .map_err(|_| "DISPLAYS_UNAVAILABLE")?
}

#[tauri::command]
async fn inspect_foreground(state: State<'_, Arc<AppState>>) -> Result<Inspection, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        state.core.try_lock().map_err(|_| "CORE_BUSY")?.inspect_foreground().map_err(|_| "INSPECTION_UNAVAILABLE")
    })
    .await
    .map_err(|_| "CORE_UNAVAILABLE")?
}

#[tauri::command]
async fn capture_display(
    state: State<'_, Arc<AppState>>,
    display: aiwin11_core::windows_bridge::Display,
) -> Result<Capture, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        state.core.try_lock().map_err(|_| "CORE_BUSY")?.capture_display(display).map_err(|_| "CAPTURE_UNAVAILABLE")
    })
    .await
    .map_err(|_| "CAPTURE_UNAVAILABLE")?
}

#[tauri::command]
async fn capture_foreground(state: State<'_, Arc<AppState>>) -> Result<Capture, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        state.core.try_lock().map_err(|_| "CORE_BUSY")?.capture_foreground().map_err(|_| "CAPTURE_UNAVAILABLE")
    })
    .await
    .map_err(|_| "CORE_UNAVAILABLE")?
}

#[tauri::command]
async fn enter_character_map_digit(state: State<'_, Arc<AppState>>, digit: u32) -> Result<bool, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        state.core.try_lock().map_err(|_| "CORE_BUSY")?.enter_character_map_digit(digit)
    })
    .await
    .map_err(|_| "CORE_UNAVAILABLE")?
}

#[tauri::command]
async fn discover_vision_models() -> Result<Vec<String>, &'static str> {
    tauri::async_runtime::spawn_blocking(|| {
        Ollama::connect().and_then(|provider| provider.vision_models()).map_err(|error| error.vision_code())
    })
    .await
    .map_err(|_| "VISION_UNAVAILABLE")?
}

#[tauri::command]
async fn analyze_capture(
    state: State<'_, Arc<AppState>>,
    model: String,
    jpeg: Vec<u8>,
) -> Result<String, &'static str> {
    read_capture(state, model, jpeg, Core::analyze_capture).await
}

#[tauri::command]
async fn transcribe_capture(
    state: State<'_, Arc<AppState>>,
    model: String,
    jpeg: Vec<u8>,
) -> Result<String, &'static str> {
    read_capture(state, model, jpeg, Core::transcribe_capture).await
}

type CaptureReader = fn(&Mutex<Core>, &str, &[u8]) -> Result<String, &'static str>;

async fn read_capture(
    state: State<'_, Arc<AppState>>,
    model: String,
    jpeg: Vec<u8>,
    execute: CaptureReader,
) -> Result<String, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let _analysis = state.vision.try_lock().map_err(|_| "VISION_BUSY")?;
        execute(&state.core, &model, &jpeg)
    })
    .await
    .map_err(|_| "VISION_UNAVAILABLE")?
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
    .await
    .map_err(|_| "CORE_UNAVAILABLE")?
}

#[tauri::command]
fn openai_key_configured(state: State<'_, Arc<AppState>>) -> Result<bool, &'static str> {
    openai::has_key(cloud_vendor(&state)?)
}

#[tauri::command]
fn save_openai_key(state: State<'_, Arc<AppState>>, key: String) -> Result<(), &'static str> {
    openai::save_key(cloud_vendor(&state)?, key)
}

#[tauri::command]
fn delete_openai_key(state: State<'_, Arc<AppState>>) -> Result<(), &'static str> {
    openai::delete_key(cloud_vendor(&state)?)
}

#[tauri::command]
fn cancel_task(state: State<'_, Arc<AppState>>) -> bool {
    state.task.cancel()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistorySettings {
    retention_days: u32,
}

#[tauri::command]
fn get_history_settings(state: State<'_, Arc<AppState>>) -> Result<HistorySettings, &'static str> {
    Ok(HistorySettings { retention_days: lock_core(&state.core).retention_days() })
}

#[tauri::command]
fn set_retention_days(state: State<'_, Arc<AppState>>, days: u32) -> Result<(), &'static str> {
    lock_core(&state.core).set_retention_days(days)
}

/// Erases past sessions. Refused while a task runs, so that its audit trail stays whole.
#[tauri::command]
fn clear_history(state: State<'_, Arc<AppState>>) -> Result<usize, &'static str> {
    if state.task.is_running() {
        return Err("TASK_BUSY");
    }
    lock_core(&state.core).clear_history()
}

/// Exports the audit journal to `%LOCALAPPDATA%\app.aiwin11.desktop\exports` and returns the path.
#[tauri::command]
async fn export_audit(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<String, &'static str> {
    let directory = app.path().app_local_data_dir().map_err(|_| "EXPORT_FAILED")?.join("exports");
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let path = lock_core(&state.core).export_audit_to(&directory)?;
        let path = path.to_string_lossy();
        Ok(path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned())
    })
    .await
    .map_err(|_| "EXPORT_FAILED")?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GrantOverview {
    active: Vec<Grant>,
    /// Actions a temporary grant may relax.
    grantable: Vec<AgentAction>,
}

#[tauri::command]
fn get_grants(state: State<'_, Arc<AppState>>) -> Result<GrantOverview, &'static str> {
    let active = lock_core(&state.core).active_grants()?;
    let grantable = AgentAction::ALL.into_iter().filter(|action| policy::grantable(*action)).collect();
    Ok(GrantOverview { active, grantable })
}

/// Main window only: the user lets one action run without asking for a limited time.
#[tauri::command]
fn create_grant(state: State<'_, Arc<AppState>>, action: AgentAction, minutes: u32) -> Result<String, &'static str> {
    lock_core(&state.core).grant_temporarily(action, minutes)
}

/// How interactive Ollama tasks pick their local model (manual, adaptive, quality, cost).
#[tauri::command]
fn get_local_strategy(state: State<'_, Arc<AppState>>) -> LocalStrategy {
    lock_core(&state.core).local_strategy()
}

#[tauri::command]
fn set_local_strategy(state: State<'_, Arc<AppState>>, strategy: LocalStrategy) -> Result<(), &'static str> {
    lock_core(&state.core).set_local_strategy(strategy)
}

#[tauri::command]
fn get_cloud_fallback(state: State<'_, Arc<AppState>>) -> Result<bool, &'static str> {
    Ok(lock_core(&state.core).cloud_fallback_enabled())
}

#[tauri::command]
fn set_cloud_fallback(state: State<'_, Arc<AppState>>, enabled: bool) -> Result<(), &'static str> {
    lock_core(&state.core).set_cloud_fallback(enabled)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelProbe {
    tool_call_verified: bool,
    tool_call_advertised: bool,
    vision_advertised: bool,
    elapsed_ms: u128,
}

#[tauri::command]
async fn probe_local_model(model: String) -> Result<ModelProbe, &'static str> {
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let ollama = Ollama::connect().map_err(|_| "MODEL_PROVIDER_UNAVAILABLE")?;
        let details = ollama.describe(&model).map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?;
        let verified = details.tool_calling_advertised
            && ollama.probe_tool_calling(&model).map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?;
        Ok(ModelProbe {
            tool_call_verified: verified,
            tool_call_advertised: details.tool_calling_advertised,
            vision_advertised: details.vision_advertised,
            elapsed_ms: started.elapsed().as_millis(),
        })
    })
    .await
    .map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?
}

/// Models loaded by Ollama and their RAM/VRAM use.
#[tauri::command]
async fn get_model_resources() -> Result<Vec<aiwin11_core::model_gateway::LoadedModel>, &'static str> {
    tauri::async_runtime::spawn_blocking(|| {
        Ollama::connect().and_then(|provider| provider.loaded_models()).map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")
    })
    .await
    .map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?
}

#[tauri::command]
fn get_runtime_status(app: tauri::AppHandle) -> Result<RuntimeStatus, &'static str> {
    let executable = std::env::current_exe().map_err(|_| "RUNTIME_UNAVAILABLE")?;
    let folder = executable.parent().ok_or("RUNTIME_UNAVAILABLE")?;
    Ok(RuntimeStatus {
        version: app.package_info().version.to_string(),
        windows_bridge: folder.join("AiWin11.Windows.exe").is_file(),
        agent_script: folder.join("aiwin11_agent.py").is_file(),
        python: aiwin11_core::task::python_available(),
        admin_helper: folder.join("AiWin11.Admin.exe").is_file(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    version: String,
    windows_bridge: bool,
    agent_script: bool,
    python: bool,
    admin_helper: bool,
}

#[tauri::command]
fn get_cloud_usage(state: State<'_, Arc<AppState>>) -> Result<aiwin11_core::CloudUsage, &'static str> {
    lock_core(&state.core).cloud_usage().map_err(|_| "USAGE_UNAVAILABLE")
}

#[tauri::command]
fn get_cloud_budget(state: State<'_, Arc<AppState>>) -> Result<u64, &'static str> {
    Ok(lock_core(&state.core).cloud_token_budget())
}

#[tauri::command]
fn set_cloud_budget(state: State<'_, Arc<AppState>>, tokens: u64) -> Result<(), &'static str> {
    lock_core(&state.core).set_cloud_token_budget(tokens)
}

fn cloud_vendor(state: &State<'_, Arc<AppState>>) -> Result<openai::Vendor, &'static str> {
    Ok(lock_core(&state.core).cloud_vendor())
}

#[tauri::command]
fn get_cloud_vendor(state: State<'_, Arc<AppState>>) -> Result<openai::Vendor, &'static str> {
    cloud_vendor(&state)
}

/// Main window only: chooses the cloud provider (each keeps its own key).
#[tauri::command]
fn set_cloud_vendor(state: State<'_, Arc<AppState>>, vendor: openai::Vendor) -> Result<(), &'static str> {
    lock_core(&state.core).set_cloud_vendor(vendor)
}

#[tauri::command]
fn get_memories(state: State<'_, Arc<AppState>>) -> Result<Vec<aiwin11_core::memory::MemoryEntry>, &'static str> {
    lock_core(&state.core).memories()
}

/// Main window only: deletes one note, or all of them without an ID.
#[tauri::command]
fn delete_memory(state: State<'_, Arc<AppState>>, id: Option<String>) -> Result<usize, &'static str> {
    lock_core(&state.core).forget(id.as_deref())
}

#[tauri::command]
fn get_embedding_model(state: State<'_, Arc<AppState>>) -> Result<String, &'static str> {
    Ok(lock_core(&state.core).embedding_model().unwrap_or_default())
}

#[tauri::command]
fn set_embedding_model(state: State<'_, Arc<AppState>>, model: String) -> Result<(), &'static str> {
    lock_core(&state.core).set_embedding_model(&model)
}

/// Main window only: chooses the agent's folder (empty: AiWin11's dedicated folder). Applies at
/// the next start.
#[tauri::command]
fn set_workspace(state: State<'_, Arc<AppState>>, path: String) -> Result<String, &'static str> {
    let stored = if path.trim().is_empty() {
        String::new()
    } else {
        let canonical = LocalFiles::validate_workspace(std::path::Path::new(path.trim()))?;
        let text = canonical.to_string_lossy().into_owned();
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    };
    lock_core(&state.core).set_workspace(&stored)?;
    Ok(stored)
}

/// The user clicked the microphone: one phrase, recognized offline by Windows.
#[tauri::command]
async fn dictate(state: State<'_, Arc<AppState>>) -> Result<String, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || state.core.try_lock().map_err(|_| "CORE_BUSY")?.dictate())
        .await
        .map_err(|_| "DICTATION_FAILED")?
}

#[tauri::command]
fn get_reports(state: State<'_, Arc<AppState>>) -> Result<Vec<aiwin11_core::memory::MemoryEntry>, &'static str> {
    lock_core(&state.core).reports()
}

/// Main window only: daily local time "HH:MM" of a saved request, or none.
#[tauri::command]
fn set_routine_schedule(
    state: State<'_, Arc<AppState>>,
    id: String,
    schedule: Option<String>,
) -> Result<(), &'static str> {
    lock_core(&state.core).set_routine_schedule(&id, schedule.as_deref())
}

/// Runs saved requests at their daily time: local model, Observation profile, result kept
/// encrypted. Skipped while another task runs.
fn run_scheduler(state: Arc<AppState>) {
    let mut startup = true;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(30));
        if state.task.is_running() {
            continue;
        }
        let (due, model) = {
            let mut core = lock_core(&state.core);
            let model = core.model_preferences().local_model;
            if model.is_empty() {
                continue;
            }
            let Ok(due) = core.take_due_routines(startup) else { continue };
            startup = false;
            (due, model)
        };
        for routine in due {
            let request = TaskRequest {
                provider: aiwin11_core::task::Provider::Ollama,
                prompt: routine.text.clone(),
                model: model.clone(),
                cloud_approved: false,
                vision_model: None,
                cloud_fallback_model: None,
                scheduled: true,
                agent: aiwin11_core::policy::AgentProfile::General,
            };
            let outcome = state.task.run(&state.core, state.files.as_ref(), &request, |_| {});
            let report = match outcome {
                Ok(outcome) => format!("« {} » : {}", routine.text, outcome.reply),
                Err(code) => format!("« {} » : échec ({code}).", routine.text),
            };
            let _ = lock_core(&state.core).save_report(&report);
        }
    }
}

#[tauri::command]
fn get_mcp_servers(state: State<'_, Arc<AppState>>) -> Result<Vec<aiwin11_core::mcp::McpServer>, &'static str> {
    Ok(lock_core(&state.core).mcp_servers())
}

/// Main window only: declares the MCP servers the agent may use (each call still asks).
#[tauri::command]
fn set_mcp_servers(
    state: State<'_, Arc<AppState>>,
    servers: Vec<aiwin11_core::mcp::McpServer>,
) -> Result<(), &'static str> {
    lock_core(&state.core).set_mcp_servers(servers)
}

/// Floating mode: a small always-on-top window in the bottom-right corner of its screen, or
/// back to the normal window.
#[tauri::command]
fn set_floating(app: tauri::AppHandle, enabled: bool) -> Result<(), &'static str> {
    use tauri::{LogicalSize, PhysicalPosition};
    let window = app.get_webview_window("main").ok_or("WINDOW_UNAVAILABLE")?;
    window.set_always_on_top(enabled).map_err(|_| "WINDOW_UNAVAILABLE")?;
    let size = if enabled { LogicalSize::new(440.0, 620.0) } else { LogicalSize::new(1060.0, 700.0) };
    window.set_size(size).map_err(|_| "WINDOW_UNAVAILABLE")?;
    if enabled {
        if let (Ok(Some(monitor)), Ok(outer)) = (window.current_monitor(), window.outer_size()) {
            let area = monitor.work_area();
            let x = area.position.x + area.size.width as i32 - outer.width as i32 - 16;
            let y = area.position.y + area.size.height as i32 - outer.height as i32 - 16;
            let _ = window.set_position(PhysicalPosition::new(x.max(area.position.x), y.max(area.position.y)));
        }
    } else {
        let _ = window.center();
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageEngine {
    endpoint: String,
    available: bool,
}

#[tauri::command]
async fn get_image_engine(state: State<'_, Arc<AppState>>) -> Result<ImageEngine, &'static str> {
    let endpoint = lock_core(&state.core).image_endpoint().unwrap_or_default();
    let probe = endpoint.clone();
    let available = tauri::async_runtime::spawn_blocking(move || aiwin11_core::images::engine_available(&probe))
        .await
        .unwrap_or(false);
    Ok(ImageEngine { endpoint, available })
}

/// Main window only: the local image engine (loopback HTTP only), or none.
#[tauri::command]
fn set_image_engine(state: State<'_, Arc<AppState>>, endpoint: String) -> Result<(), &'static str> {
    lock_core(&state.core).set_image_endpoint(&endpoint)
}

/// Opens AiWin11's images folder in Explorer (fixed executable, fixed folder).
#[tauri::command]
fn open_images_folder(app: tauri::AppHandle) -> Result<(), &'static str> {
    let folder = app.path().app_local_data_dir().map_err(|_| "IMAGE_WRITE_FAILED")?.join("images");
    fs::create_dir_all(&folder).map_err(|_| "IMAGE_WRITE_FAILED")?;
    let windows = std::env::var_os("SystemRoot").ok_or("IMAGE_WRITE_FAILED")?;
    std::process::Command::new(std::path::Path::new(&windows).join("explorer.exe"))
        .arg(&folder)
        .spawn()
        .map(|_| ())
        .map_err(|_| "IMAGE_WRITE_FAILED")
}

#[tauri::command]
fn get_routines(state: State<'_, Arc<AppState>>) -> Result<Vec<aiwin11_core::memory::MemoryEntry>, &'static str> {
    lock_core(&state.core).routines()
}

/// Main window only: saves the current request; it only runs when the user sends it.
#[tauri::command]
fn save_routine(state: State<'_, Arc<AppState>>, text: String) -> Result<String, &'static str> {
    lock_core(&state.core).save_routine(&text)
}

#[tauri::command]
fn update_routine(state: State<'_, Arc<AppState>>, id: String, text: String) -> Result<(), &'static str> {
    lock_core(&state.core).update_routine(&id, &text)
}

#[tauri::command]
fn get_privacy(state: State<'_, Arc<AppState>>) -> Result<bool, &'static str> {
    Ok(lock_core(&state.core).private_mode())
}

/// Main window only: private mode blocks every cloud request.
#[tauri::command]
fn set_privacy(state: State<'_, Arc<AppState>>, enabled: bool) -> Result<(), &'static str> {
    lock_core(&state.core).set_private_mode(enabled)
}

#[tauri::command]
fn get_admin_helper(state: State<'_, Arc<AppState>>) -> Result<bool, &'static str> {
    Ok(lock_core(&state.core).admin_helper_enabled())
}

/// Main window only: turns the elevated helper (closed list of administrator operations) on or off.
#[tauri::command]
fn set_admin_helper(state: State<'_, Arc<AppState>>, enabled: bool) -> Result<(), &'static str> {
    lock_core(&state.core).set_admin_helper_enabled(enabled)
}

#[tauri::command]
fn revoke_grant(state: State<'_, Arc<AppState>>, id: String) -> Result<(), &'static str> {
    lock_core(&state.core).revoke_grant(&id)
}

/// "Stoppe toutes les tâches": always available, journaled with its immediate result. The stop
/// is confirmed when the running task ends as cancelled (`task.cancelled`).
#[tauri::command]
fn stop_all_tasks(state: State<'_, Arc<AppState>>) -> Result<StopStatus, &'static str> {
    let status = state.task.stop_all();
    let _ = lock_core(&state.core).journal("tasks.stop.requested", Some(status.code()));
    Ok(status)
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
    /// Reviewed facts about each tool (risk, dry run, reversibility, deadline).
    manifests: &'static [ToolManifest],
}

#[tauri::command]
fn get_policy_overview(state: State<'_, Arc<AppState>>) -> Result<PolicyOverview, &'static str> {
    let settings = lock_core(&state.core).policy_settings();
    let table =
        policy::table(settings.simulate).into_iter().map(|(action, effects)| PolicyRow { action, effects }).collect();
    Ok(PolicyOverview { settings, table, manifests: &MANIFESTS })
}

/// Applies to the next task; a running task keeps the settings it started with.
#[tauri::command]
fn set_policy_settings(state: State<'_, Arc<AppState>>, settings: PolicySettings) -> Result<(), &'static str> {
    lock_core(&state.core).set_policy_settings(settings).map_err(|_| "SETTINGS_UNAVAILABLE")
}

#[tauri::command]
fn get_model_preferences(state: State<'_, Arc<AppState>>) -> Result<ModelPreferences, &'static str> {
    Ok(lock_core(&state.core).model_preferences())
}

#[tauri::command]
fn set_model_preferences(state: State<'_, Arc<AppState>>, preferences: ModelPreferences) -> Result<(), &'static str> {
    lock_core(&state.core).set_model_preferences(preferences)
}

/// The user's answer to the action the agent proposed for the running task.
#[tauri::command]
fn resolve_approval(state: State<'_, Arc<AppState>>, approval_id: String, approved: bool) -> Result<(), &'static str> {
    state.task.resolve_approval(&approval_id, approved)
}

/// Brings the main window to the front (tray, global shortcut).
fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Tray icon (open, stop all tasks, quit) and the global shortcut Ctrl+Alt+Espace that brings
/// AiWin11 to the front from any application. Nothing here can start an action.
fn desktop_surfaces(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };
    use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

    let open = MenuItem::with_id(app, "open", "Ouvrir AiWin11", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stoppe toutes les tâches", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &stop, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main").tooltip("AiWin11").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id.as_ref() {
        "open" => show_main(app),
        "stop" => {
            let state = Arc::clone(app.state::<Arc<AppState>>().inner());
            let status = state.task.stop_all();
            let _ = lock_core(&state.core).journal("tasks.stop.requested", Some(status.code()));
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            show_main(tray.app_handle());
        }
    })
    .build(app)?;

    let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, pressed, event| {
                if *pressed == shortcut && event.state() == ShortcutState::Pressed {
                    show_main(app);
                    // Command palette: the window opens on the request field, ready to type.
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.eval("window.dispatchEvent(new Event('aiwin11-palette'))");
                    }
                }
            })
            .build(),
    )?;
    // Another application may already own the shortcut: the tray still works.
    let _ = app.global_shortcut().register(shortcut);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let core = Core::open(&data_dir.join("aiwin11.sqlite3"))?;
            // The user's folder if one was chosen and is still allowed; otherwise the dedicated one.
            let files = core
                .workspace()
                .and_then(|workspace| LocalFiles::open_with_workspace(&data_dir, &workspace).ok())
                .or_else(|| LocalFiles::open(&data_dir).ok());
            app.manage(Arc::new(AppState {
                video: Mutex::new(VideoJob::Idle),
                files,
                core: Mutex::new(core),
                task: TaskSlot::default(),
                vision: Mutex::new(()),
            }));
            // Pay the bridge's first start (runtime load, antivirus scan) now, not within a
            // command's deadline. The spare process waits idle for an authorized command.
            std::thread::spawn(aiwin11_core::windows_bridge::prewarm_bridge);
            let scheduler = Arc::clone(app.state::<Arc<AppState>>().inner());
            std::thread::spawn(move || run_scheduler(scheduler));
            desktop_surfaces(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            discover_models,
            run_task,
            cancel_task,
            stop_all_tasks,
            get_grants,
            get_admin_helper,
            get_privacy,
            get_cloud_fallback,
            get_local_strategy,
            set_local_strategy,
            get_memories,
            get_routines,
            get_image_engine,
            set_image_engine,
            open_images_folder,
            set_floating,
            get_mcp_servers,
            set_mcp_servers,
            get_reports,
            set_routine_schedule,
            dictate,
            set_workspace,
            get_embedding_model,
            set_embedding_model,
            save_routine,
            update_routine,
            delete_memory,
            get_cloud_vendor,
            set_cloud_vendor,
            get_cloud_budget,
            get_cloud_usage,
            get_runtime_status,
            video_status,
            start_video,
            updates_configured,
            check_update,
            install_update,
            set_cloud_budget,
            set_cloud_fallback,
            get_model_resources,
            probe_local_model,
            set_privacy,
            set_admin_helper,
            create_grant,
            revoke_grant,
            get_history_settings,
            set_retention_days,
            clear_history,
            export_audit,
            resolve_approval,
            get_policy_overview,
            set_policy_settings,
            get_tasks,
            get_model_preferences,
            set_model_preferences,
            openai_key_configured,
            save_openai_key,
            delete_openai_key,
            inspect_foreground,
            list_displays,
            capture_foreground,
            capture_display,
            discover_vision_models,
            analyze_capture,
            transcribe_capture,
            enter_character_map_digit,
            local_files_root,
            preview_local_text,
            copy_local_text,
            move_local_text,
            trash_local_text,
            list_trash,
            list_local_text,
            restore_local_text,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AiWin11");
}
