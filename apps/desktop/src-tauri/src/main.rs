#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop;

use std::{
    fs,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

use aiwin11_core::{
    backup,
    folders::FolderSet,
    grants::Grant,
    local_files::LocalFiles,
    manifest::{ToolManifest, MANIFESTS},
    model_gateway::{LocalModelProvider, LocalStrategy, ModelResource, Ollama},
    openai,
    policy::{self, AgentAction, Effect, PolicySettings},
    task::{SessionTurn, StopStatus, TaskRequest, TaskSlot, MAX_SESSION_TURNS},
    windows_bridge::{Capture, Inspection},
    CloudCost, Core, ModelFacts, ModelManifest, ModelPreferences, ToolCalling,
};
use serde::Serialize;
use tauri::{ipc::Channel, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    session_id: String,
    events: Vec<Activity>,
    replay: bool,
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
    /// The agent's folders; the desktop Files page works on the primary one.
    files: Option<FolderSet>,
    task: TaskSlot,
    task_worker: aiwin11_model_ipc::TaskSupervisor,
    desktop: desktop::DesktopState,
    vision: Mutex<()>,
    video: Mutex<VideoJob>,
    /// Set by "Annuler la génération"; the follower stops at its next poll.
    video_cancel: std::sync::atomic::AtomicBool,
    /// Process start, and the time the interface first reported itself rendered (T-001).
    launched: Instant,
    ui_ready_ms: OnceLock<u64>,
    /// Path of the SQLite store; backups live next to it.
    store: std::path::PathBuf,
    /// Session memory: the last exchanges of this run of AiWin11, in memory only (never stored).
    turns: Mutex<Vec<SessionTurn>>,
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
        #[serde(rename = "canResume")]
        can_resume: bool,
    },
}

#[tauri::command]
fn video_status(state: State<'_, Arc<AppState>>) -> Result<VideoJob, &'static str> {
    let state = Arc::clone(state.inner());
    let mut job = state.video.lock().map_err(|_| "VIDEO_UNAVAILABLE")?;
    // A job the agent submitted (after the user's approval) is followed as soon as it is seen.
    if matches!(*job, VideoJob::Idle | VideoJob::Completed { .. } | VideoJob::Failed { can_resume: false, .. }) {
        let pending = {
            let mut core = lock_core(&state.core);
            match core.video_ticket() {
                Ok(Some(ticket)) => core.authorize_video().ok().map(|task| (ticket, task)),
                _ => None,
            }
        };
        if let Some((ticket, task)) = pending {
            *job = VideoJob::Queued;
            let follower = Arc::clone(&state);
            tauri::async_runtime::spawn_blocking(move || follow_video(&follower, task.as_deref(), ticket, true));
        }
    }
    Ok(job.clone())
}

/// Main window only: the workflow and port the agent may use, and the daily quota.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VideoSettings {
    engine: Option<aiwin11_core::video::VideoEngine>,
    quota: u32,
    used: u32,
}

#[tauri::command]
fn get_video_settings(state: State<'_, Arc<AppState>>) -> VideoSettings {
    let core = lock_core(&state.core);
    let (quota, used) = core.video_quota();
    VideoSettings { engine: core.video_engine(), quota, used }
}

#[tauri::command]
fn set_video_engine(
    state: State<'_, Arc<AppState>>,
    engine: Option<aiwin11_core::video::VideoEngine>,
) -> Result<(), &'static str> {
    lock_core(&state.core).set_video_engine(engine)
}

#[tauri::command]
fn set_video_quota(state: State<'_, Arc<AppState>>, quota: u32) -> Result<(), &'static str> {
    lock_core(&state.core).set_video_quota(quota)
}

/// Main window only: cancels the followed job in ComfyUI (only that job).
#[tauri::command]
async fn cancel_video(state: State<'_, Arc<AppState>>) -> Result<(), &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let ticket = lock_core(&state.core).video_ticket()?.ok_or("VIDEO_UNAVAILABLE")?;
        let result = ticket.cancel();
        state.video_cancel.store(true, std::sync::atomic::Ordering::Release);
        let _ = lock_core(&state.core).journal("video.cancel.requested", None);
        result
    })
    .await
    .map_err(|_| "VIDEO_UNAVAILABLE")?
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
    let task = {
        let mut core = lock_core(&state.core);
        if core.video_ticket()?.is_some() {
            return Err("VIDEO_TRACKING_PENDING");
        }
        let task = core.authorize_video()?;
        core.reserve_video_submission()?;
        task
    };
    *job = VideoJob::Submitting;
    drop(job);
    tauri::async_runtime::spawn_blocking(move || match request.submit() {
        Ok(ticket) => {
            let saved = lock_core(&state.core).set_video_ticket(Some(&ticket)).is_ok();
            follow_video(&state, task.as_deref(), ticket, saved);
        }
        Err(code) => finish_video(&state, task.as_deref(), Err(code), false),
    });
    Ok(())
}

#[tauri::command]
fn resume_video(state: State<'_, Arc<AppState>>) -> Result<(), &'static str> {
    let state = Arc::clone(state.inner());
    let mut job = state.video.lock().map_err(|_| "VIDEO_UNAVAILABLE")?;
    if matches!(*job, VideoJob::Submitting | VideoJob::Queued) {
        return Err("VIDEO_BUSY");
    }
    let (ticket, task) = {
        let mut core = lock_core(&state.core);
        let ticket = core.video_ticket()?.ok_or("VIDEO_UNAVAILABLE")?;
        (ticket, core.authorize_video()?)
    };
    *job = VideoJob::Queued;
    drop(job);
    tauri::async_runtime::spawn_blocking(move || follow_video(&state, task.as_deref(), ticket, true));
    Ok(())
}

#[tauri::command]
fn forget_video_tracking(state: State<'_, Arc<AppState>>) -> Result<(), &'static str> {
    let mut job = state.video.lock().map_err(|_| "VIDEO_UNAVAILABLE")?;
    if matches!(*job, VideoJob::Submitting | VideoJob::Queued) {
        return Err("VIDEO_BUSY");
    }
    let mut core = lock_core(&state.core);
    core.set_video_ticket(None)?;
    core.journal("video.tracking.forgotten", None).map_err(|_| "VIDEO_AUDIT_UNAVAILABLE")?;
    *job = VideoJob::Idle;
    Ok(())
}

fn follow_video(state: &AppState, task: Option<&str>, ticket: aiwin11_core::video::VideoTicket, saved: bool) {
    if let Ok(mut job) = state.video.lock() {
        *job = VideoJob::Queued;
    }
    state.video_cancel.store(false, std::sync::atomic::Ordering::Release);
    let outcome = ticket.follow(|| state.video_cancel.load(std::sync::atomic::Ordering::Acquire));
    let terminal =
        matches!(&outcome, Ok(_) | Err("VIDEO_GENERATION_FAILED" | "VIDEO_OUTPUT_MISSING" | "VIDEO_CANCELLED"));
    let mut can_resume = saved;
    if terminal && saved {
        can_resume = lock_core(&state.core).set_video_ticket(None).is_err();
    }
    finish_video(state, task, outcome, can_resume);
}

fn finish_video(state: &AppState, task: Option<&str>, outcome: Result<String, &'static str>, can_resume: bool) {
    let audited = lock_core(&state.core).finish_video(task, &outcome);
    if let Ok(mut job) = state.video.lock() {
        *job = match audited.and(outcome) {
            Ok(filename) if !can_resume => VideoJob::Completed { filename },
            Ok(_) => VideoJob::Failed { code: "VIDEO_UNAVAILABLE", can_resume },
            Err(code) => VideoJob::Failed { code, can_resume },
        };
    }
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
        let files = state.files.as_ref().ok_or("FILE_UNAVAILABLE")?.primary();
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
        let files = state.files.as_ref().ok_or("FILE_UNAVAILABLE")?.primary();
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
async fn index_local_text(state: State<'_, Arc<AppState>>, name: String) -> Result<(), &'static str> {
    with_local_files(state, move |files, core| core.index_file(files, &name)).await
}

#[tauri::command]
fn indexed_file_names(state: State<'_, Arc<AppState>>) -> Result<Vec<String>, &'static str> {
    lock_core(&state.core).indexed_file_names()
}

#[tauri::command]
fn remove_indexed_file(state: State<'_, Arc<AppState>>, name: String) -> Result<(), &'static str> {
    lock_core(&state.core).remove_indexed_file(&name)
}

#[tauri::command]
async fn search_indexed_files(
    state: State<'_, Arc<AppState>>,
    query: String,
) -> Result<Vec<aiwin11_core::memory::IndexedFile>, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let (files, model) = {
            let core = lock_core(&state.core);
            (core.indexed_files_with_ids()?, core.embedding_model())
        };
        let entries: Vec<(&str, &str)> = files.iter().map(|(id, file)| (id.as_str(), file.excerpt.as_str())).collect();
        let vectors = model.as_deref().and_then(|model| {
            aiwin11_core::memory::semantic_vectors(&state.core, model, &query, &entries, |texts| {
                aiwin11_core::model_gateway::Ollama::connect().and_then(|ollama| ollama.embed(model, texts)).ok()
            })
        });
        let files = files.into_iter().map(|(_, file)| file).collect();
        Ok(aiwin11_core::memory::search_indexed_files(&query, files, vectors.as_deref()))
    })
    .await
    .map_err(|_| "MEMORY_UNAVAILABLE")?
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
        let files = state.files.as_ref().ok_or("FILE_UNAVAILABLE")?.primary();
        let mut core = state.core.try_lock().map_err(|_| "CORE_BUSY")?;
        execute(files, &mut core)
    })
    .await
    .map_err(|_| "FILE_UNAVAILABLE")?
}

#[tauri::command]
async fn get_snapshot(
    state: State<'_, Arc<AppState>>,
    session_id: Option<String>,
    after_sequence: Option<u64>,
) -> Result<Snapshot, &'static str> {
    let state = Arc::clone(state.inner());
    let current_session = lock_core(&state.core).session_id().to_owned();
    let after = after_sequence.filter(|_| session_id.as_deref() == Some(current_session.as_str()));
    #[cfg(windows)]
    let events = aiwin11_model_ipc::journal_page(&state.store, current_session.clone(), after).await?;
    #[cfg(not(windows))]
    let events = aiwin11_core::read_committed_activity(&state.store, &current_session, after)
        .map_err(|_| "ACTIVITY_UNAVAILABLE")?;
    Ok(Snapshot {
        session_id: current_session,
        replay: after.is_some(),
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
    #[cfg(windows)]
    return aiwin11_model_ipc::list_local_models().await;

    #[cfg(not(windows))]
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
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    mut request: TaskRequest,
    progress: Channel<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let state = Arc::clone(state.inner());
    // The context comes from this process only; the core decides what may reach a cloud model.
    request.context = state.turns.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = run_task_worker(&state, &request, |update| {
            if update.pointer("/update/type").and_then(serde_json::Value::as_str) == Some("approvalRequired") {
                desktop::notify(&app, "Un accord est demandé");
            }
            let _ = progress.send(update);
        });
        if let Ok(outcome) = &result {
            remember_turn(&state, &request, outcome);
        }
        match &result {
            Ok(_) => desktop::notify(&app, "Tâche terminée"),
            Err(code) if code == "TASK_CANCELLED" => desktop::notify(&app, "Tâche annulée"),
            Err(code) if code == "TASK_BUSY" => {}
            Err(_) => desktop::notify(&app, "Tâche en échec"),
        }
        result
    })
    .await
    .map_err(|_| "CORE_UNAVAILABLE")?
}

/// Keeps the exchange for the next requests of this session (session memory).
fn remember_turn(state: &AppState, request: &TaskRequest, outcome: &serde_json::Value) {
    let Some(reply) = outcome["reply"].as_str().filter(|reply| !reply.trim().is_empty()) else { return };
    let mut turns = state.turns.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    turns.push(SessionTurn {
        request: request.prompt.clone(),
        reply: reply.to_owned(),
        cloud: request.provider == aiwin11_core::task::Provider::OpenAi,
    });
    let excess = turns.len().saturating_sub(MAX_SESSION_TURNS);
    turns.drain(..excess);
}

/// Number of earlier exchanges the next request will carry.
#[tauri::command]
fn session_context_size(state: State<'_, Arc<AppState>>) -> usize {
    state.turns.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len()
}

/// "Nouvelle conversation": the next request starts without context.
#[tauri::command]
fn clear_session_context(state: State<'_, Arc<AppState>>) {
    state.turns.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
    let _ = lock_core(&state.core).journal("session.context.cleared", None);
}

fn run_task_worker(
    state: &AppState,
    request: &TaskRequest,
    progress: impl Fn(serde_json::Value),
) -> Result<serde_json::Value, String> {
    state.task.while_idle(|| run_reserved_task(state, request, progress)).ok_or("TASK_BUSY")?
}

fn run_reserved_task(
    state: &AppState,
    request: &TaskRequest,
    progress: impl Fn(serde_json::Value),
) -> Result<serde_json::Value, String> {
    let session = lock_core(&state.core).session_id().to_owned();
    let result = tauri::async_runtime::block_on(state.task_worker.run(&state.store, &session, request, progress));
    if matches!(result.as_ref().err().map(String::as_str), Some("TASK_WORKER_UNAVAILABLE" | "CORE_UNAVAILABLE")) {
        let _ = lock_core(&state.core).recover_task_worker();
    }
    result
}

#[tauri::command]
fn get_task_replay(
    state: State<'_, Arc<AppState>>,
    run_id: Option<String>,
    after_sequence: Option<usize>,
) -> aiwin11_model_ipc::TaskReplay {
    state.task_worker.replay(run_id.as_deref(), after_sequence.unwrap_or_default())
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
async fn cancel_task(state: State<'_, Arc<AppState>>) -> Result<bool, &'static str> {
    state.task_worker.control(aiwin11_model_ipc::TaskCommand::Cancel, String::new()).await.map(|result| result.0)
}

/// Pauses the running task at its next safe point (before the next model step).
#[tauri::command]
async fn pause_task(state: State<'_, Arc<AppState>>) -> Result<bool, &'static str> {
    state.task_worker.control(aiwin11_model_ipc::TaskCommand::Pause, String::new()).await.map(|result| result.0)
}

/// Resumes a paused task, or withdraws a pause not reached yet.
#[tauri::command]
async fn resume_task(state: State<'_, Arc<AppState>>) -> Result<bool, &'static str> {
    state.task_worker.control(aiwin11_model_ipc::TaskCommand::Resume, String::new()).await.map(|result| result.0)
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
    Ok(lock_core(&state.core).legacy_cloud_fallback())
}

#[tauri::command]
fn get_agent_models(state: State<'_, Arc<AppState>>) -> aiwin11_core::AgentModels {
    lock_core(&state.core).agent_models()
}

#[tauri::command]
fn get_local_runtime_preferences(state: State<'_, Arc<AppState>>) -> aiwin11_core::LocalRuntimePreferences {
    lock_core(&state.core).local_runtime_preferences()
}

#[tauri::command]
fn set_local_runtime_preferences(
    state: State<'_, Arc<AppState>>,
    preferences: aiwin11_core::LocalRuntimePreferences,
) -> Result<(), &'static str> {
    lock_core(&state.core).set_local_runtime_preferences(preferences)
}

#[tauri::command]
async fn discover_lm_studio_models() -> Result<Vec<String>, &'static str> {
    aiwin11_model_ipc::list_lm_studio_models().await
}

#[tauri::command]
fn set_agent_models(state: State<'_, Arc<AppState>>, models: aiwin11_core::AgentModels) -> Result<(), &'static str> {
    lock_core(&state.core).set_agent_models(models)
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
async fn probe_local_model(state: State<'_, Arc<AppState>>, model: String) -> Result<ModelProbe, &'static str> {
    let state = Arc::clone(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let ollama = Ollama::connect().map_err(|_| "MODEL_PROVIDER_UNAVAILABLE")?;
        let details = ollama.describe(&model).map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?;
        let (verified, tokens_per_second) =
            ollama.probe_measured(&model).map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?;
        let elapsed_ms = started.elapsed().as_millis();
        let status = match (details.tool_calling_advertised, verified) {
            (false, _) => ToolCalling::NotAdvertised,
            (true, true) => ToolCalling::Verified,
            (true, false) => ToolCalling::Failed,
        };
        let facts = ModelFacts {
            vision_advertised: details.vision_advertised,
            context_length: details.context_length,
            parameter_size: details.parameter_size.clone(),
            quantization: details.quantization.clone(),
            size_bytes: ollama.installed_size(&model).ok().flatten(),
            tokens_per_second,
            vram_bytes: ollama.loaded_vram(&model).ok().flatten(),
        };
        lock_core(&state.core).record_model_probe(
            &model,
            status,
            Some(&facts),
            u64::try_from(elapsed_ms).unwrap_or(u64::MAX),
        )?;
        Ok(ModelProbe {
            tool_call_verified: verified,
            tool_call_advertised: details.tool_calling_advertised,
            vision_advertised: details.vision_advertised,
            elapsed_ms,
        })
    })
    .await
    .map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?
}

/// Persisted model manifests: last probe result per local model.
#[tauri::command]
fn get_model_manifests(state: State<'_, Arc<AppState>>) -> Result<Vec<ModelManifest>, &'static str> {
    lock_core(&state.core).model_manifests().map_err(|_| "SETTINGS_UNAVAILABLE")
}

/// Models loaded by Ollama and their RAM/VRAM use.
#[tauri::command]
async fn get_model_resources() -> Result<Vec<ModelResource>, &'static str> {
    #[cfg(windows)]
    return aiwin11_model_ipc::list_loaded_models()
        .await
        .map(|models| models.into_iter().map(ModelResource::from).collect());

    #[cfg(not(windows))]
    tauri::async_runtime::spawn_blocking(|| {
        Ollama::connect()
            .and_then(|provider| provider.loaded_models())
            .map(|models| models.into_iter().map(ModelResource::from).collect())
            .map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")
    })
    .await
    .map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?
}

/// Called by the interface after its first paint; only the first call of a launch counts.
#[tauri::command]
fn ui_ready(state: State<'_, Arc<AppState>>) -> u64 {
    let mut first = false;
    let elapsed = *state.ui_ready_ms.get_or_init(|| {
        first = true;
        u64::try_from(state.launched.elapsed().as_millis()).unwrap_or(u64::MAX)
    });
    if first {
        let _ = lock_core(&state.core).record_ui_ready(elapsed);
    }
    elapsed
}

#[tauri::command]
async fn ui_navigation(state: State<'_, Arc<AppState>>, elapsed_ms: u32) -> Result<(), &'static str> {
    if elapsed_ms > 30_000 {
        return Err("INVALID_UI_DURATION");
    }
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        lock_core(&state.core).record_ui_navigation(elapsed_ms).map_err(|_| "ACTIVITY_UNAVAILABLE")
    })
    .await
    .map_err(|_| "CORE_UNAVAILABLE")?
}

#[tauri::command]
fn get_runtime_status(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<RuntimeStatus, &'static str> {
    let executable = std::env::current_exe().map_err(|_| "RUNTIME_UNAVAILABLE")?;
    let folder = executable.parent().ok_or("RUNTIME_UNAVAILABLE")?;
    Ok(RuntimeStatus {
        version: app.package_info().version.to_string(),
        windows_bridge: folder.join("AiWin11.Windows.exe").is_file(),
        agent_script: folder.join("aiwin11_agent.py").is_file(),
        python: aiwin11_core::task::python_available(),
        admin_helper: folder.join("AiWin11.Admin.exe").is_file(),
        ui_ready_ms: state.ui_ready_ms.get().copied(),
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
    ui_ready_ms: Option<u64>,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupOverview {
    backups: Vec<backup::BackupInfo>,
    /// Backup that will replace the data at the next launch, if any.
    pending_restore: Option<String>,
}

#[tauri::command]
fn list_backups(state: State<'_, Arc<AppState>>) -> BackupOverview {
    BackupOverview {
        backups: backup::list(&backup::backup_dir(&state.store)),
        pending_restore: backup::pending_restore(&state.store),
    }
}

#[tauri::command]
fn backup_data(state: State<'_, Arc<AppState>>) -> Result<backup::BackupInfo, &'static str> {
    lock_core(&state.core).backup(&backup::backup_dir(&state.store))
}

#[tauri::command]
fn schedule_restore(state: State<'_, Arc<AppState>>, name: String) -> Result<(), &'static str> {
    backup::schedule_restore(&state.store, &name)?;
    let _ = lock_core(&state.core).journal("data.restore.scheduled", None);
    Ok(())
}

#[tauri::command]
fn cancel_restore(state: State<'_, Arc<AppState>>) {
    backup::cancel_restore(&state.store);
    let _ = lock_core(&state.core).journal("data.restore.cancelled", None);
}

#[tauri::command]
fn get_blocked_applications(state: State<'_, Arc<AppState>>) -> Vec<&'static str> {
    lock_core(&state.core).blocked_applications()
}

#[tauri::command]
fn set_blocked_applications(state: State<'_, Arc<AppState>>, applications: Vec<String>) -> Result<(), &'static str> {
    lock_core(&state.core).set_blocked_applications(&applications)
}

#[tauri::command]
fn get_protected_windows(state: State<'_, Arc<AppState>>) -> Vec<String> {
    lock_core(&state.core).protected_windows()
}

#[tauri::command]
fn set_protected_windows(state: State<'_, Arc<AppState>>, patterns: Vec<String>) -> Result<(), &'static str> {
    lock_core(&state.core).set_protected_windows(&patterns)
}

#[tauri::command]
fn get_protected_files(state: State<'_, Arc<AppState>>) -> Vec<String> {
    lock_core(&state.core).protected_files()
}

#[tauri::command]
fn set_protected_files(state: State<'_, Arc<AppState>>, patterns: Vec<String>) -> Result<(), &'static str> {
    lock_core(&state.core).set_protected_files(&patterns)
}

#[tauri::command]
fn get_local_fallback_model(state: State<'_, Arc<AppState>>) -> Option<String> {
    lock_core(&state.core).local_fallback_model()
}

#[tauri::command]
fn set_local_fallback_model(state: State<'_, Arc<AppState>>, model: String) -> Result<(), &'static str> {
    lock_core(&state.core).set_local_fallback_model(&model)
}

#[tauri::command]
fn get_cloud_cost(state: State<'_, Arc<AppState>>) -> CloudCost {
    lock_core(&state.core).cloud_cost()
}

#[tauri::command]
fn set_cloud_cost(state: State<'_, Arc<AppState>>, cost: CloudCost) -> Result<(), &'static str> {
    lock_core(&state.core).set_cloud_cost(cost)
}

fn cloud_vendor(state: &State<'_, Arc<AppState>>) -> Result<openai::Vendor, &'static str> {
    Ok(lock_core(&state.core).cloud_vendor())
}

#[tauri::command]
fn get_cloud_vendor(state: State<'_, Arc<AppState>>) -> Result<openai::Vendor, &'static str> {
    cloud_vendor(&state)
}

#[tauri::command]
fn get_cloud_providers(state: State<'_, Arc<AppState>>) -> Vec<serde_json::Value> {
    let core = lock_core(&state.core);
    openai::CLOUD_PROVIDERS
        .iter()
        .map(|entry| {
            serde_json::json!({
                "id": entry.id, "label": entry.label, "enabled": core.cloud_vendor_enabled(entry.id)
            })
        })
        .collect()
}

#[tauri::command]
fn set_cloud_provider_enabled(
    state: State<'_, Arc<AppState>>,
    vendor: openai::Vendor,
    enabled: bool,
) -> Result<(), &'static str> {
    lock_core(&state.core).set_cloud_vendor_enabled(vendor, enabled)
}

#[tauri::command]
async fn probe_cloud_model(
    state: State<'_, Arc<AppState>>,
    vendor: openai::Vendor,
    model: String,
) -> Result<bool, &'static str> {
    if !aiwin11_core::valid_model_name(&model) {
        return Err("MODEL_REQUIRED");
    }
    let state = Arc::clone(state.inner());
    {
        let core = lock_core(&state.core);
        if core.private_mode() {
            return Err("PRIVATE_MODE");
        }
        if !core.cloud_vendor_enabled(vendor) {
            return Err("CLOUD_PROVIDER_DISABLED");
        }
        core.journal("model.cloud.probe", Some(vendor.code())).map_err(|_| "AUDIT_UNAVAILABLE")?;
    }
    let messages = [serde_json::json!({
        "role": "user",
        "content": "Synthetic tool capability test. Call launch_application exactly once with application notepad. No action will be executed."
    })];
    let name = model.clone();
    let (proposal, used) = tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(aiwin11_model_ipc::propose_cloud(vendor, &name, &messages, &mut |_| true))
    })
    .await
    .map_err(|_| "CORE_UNAVAILABLE")??;
    let verified = matches!(proposal, aiwin11_core::model_gateway::Proposal::Calls(calls) if
        calls.as_array().is_some_and(|calls| calls.len() == 1
            && calls[0].pointer("/function/name").and_then(serde_json::Value::as_str) == Some("launch_application")
            && calls[0].pointer("/function/arguments") == Some(&serde_json::json!({"application":"notepad"}))));
    let core = lock_core(&state.core);
    core.record_cloud_probe_usage(used).map_err(|_| "AUDIT_UNAVAILABLE")?;
    core.record_cloud_tool_capability(vendor, &model, verified).map_err(|_| "SETTINGS_UNAVAILABLE")?;
    Ok(verified)
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

/// Main window only: days notes and reports are kept (0: until erased).
#[tauri::command]
fn get_memory_retention(state: State<'_, Arc<AppState>>) -> u32 {
    lock_core(&state.core).memory_retention_days()
}

/// Main window only: changes the retention and returns how many entries were erased now.
#[tauri::command]
fn set_memory_retention(state: State<'_, Arc<AppState>>, days: u32) -> Result<usize, &'static str> {
    lock_core(&state.core).set_memory_retention_days(days)
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

/// Main window only: the extra folders the agent may work in (applied at the next start), and
/// the aliases of those in use now.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderOverview {
    saved: Vec<String>,
    aliases: Vec<String>,
}

#[tauri::command]
fn get_extra_folders(state: State<'_, Arc<AppState>>) -> FolderOverview {
    let saved = lock_core(&state.core).extra_folders().iter().map(|path| path.to_string_lossy().into_owned()).collect();
    let aliases = state.files.as_ref().map(|files| files.aliases().iter().map(|alias| (*alias).to_owned()).collect());
    FolderOverview { saved, aliases: aliases.unwrap_or_default() }
}

#[tauri::command]
fn set_extra_folders(state: State<'_, Arc<AppState>>, paths: Vec<String>) -> Result<Vec<String>, &'static str> {
    let mut stored = Vec::new();
    for path in paths.iter().map(|path| path.trim()).filter(|path| !path.is_empty()) {
        let canonical = LocalFiles::validate_workspace(std::path::Path::new(path))?;
        let text = canonical.to_string_lossy().into_owned();
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned();
        if !stored.contains(&text) {
            stored.push(text);
        }
    }
    lock_core(&state.core).set_extra_folders(&stored)?;
    Ok(stored)
}

/// The user clicked the microphone (or pressed Ctrl+Alt+V): one phrase, recognized offline by
/// Windows. While it listens, every window and the notification area show it.
#[tauri::command]
async fn dictate(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<String, &'static str> {
    let state = Arc::clone(state.inner());
    desktop::microphone(&app, true);
    let result =
        tauri::async_runtime::spawn_blocking(move || state.core.try_lock().map_err(|_| "CORE_BUSY")?.dictate()).await;
    desktop::microphone(&app, false);
    result.map_err(|_| "DICTATION_FAILED")?
}

#[tauri::command]
async fn get_speech_recognizers(state: State<'_, Arc<AppState>>) -> Result<serde_json::Value, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let mut core = lock_core(&state.core);
        let selected = core.speech_recognizer();
        let list = core.speech_recognizers()?;
        Ok(serde_json::json!({ "selected": selected, "recognizers": list.into_iter().map(|entry| {
            serde_json::json!({"id": entry.id, "label": entry.label, "language": entry.language})
        }).collect::<Vec<_>>() }))
    })
    .await
    .map_err(|_| "CORE_UNAVAILABLE")?
}

#[tauri::command]
async fn set_speech_recognizer(state: State<'_, Arc<AppState>>, id: String) -> Result<(), &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || lock_core(&state.core).set_speech_recognizer(&id))
        .await
        .map_err(|_| "CORE_UNAVAILABLE")?
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

/// What the agent's primary folder holds now (names, sizes, write times), as a short hash.
fn folder_stamp(state: &AppState) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let root = state.files.as_ref()?.root().to_path_buf();
    let mut entries: Vec<(String, u64, u128)> = fs::read_dir(root)
        .ok()?
        .flatten()
        .take(10_000)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let modified = metadata.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
            Some((entry.file_name().to_string_lossy().into_owned(), metadata.len(), modified))
        })
        .collect();
    entries.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    entries.hash(&mut hasher);
    Some(format!("{:016x}", hasher.finish()))
}

/// Runs saved requests at their daily time, at startup or when the agent's folder changes:
/// local model, Observation profile, result kept encrypted. Skipped while another task runs.
fn run_scheduler(state: Arc<AppState>) {
    let mut startup = true;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(30));
        while state.task.while_idle(|| {
            let (routine, model) = {
                let mut core = lock_core(&state.core);
                let model = core.model_preferences().local_model;
                if model.is_empty() {
                    return false;
                }
                let Ok(due) = core.take_due_routine(startup) else { return false };
                let due = match due {
                    Some(routine) => Some(routine),
                    None => folder_stamp(&state).and_then(|stamp| core.take_folder_routine(&stamp).ok().flatten()),
                };
                let Some(routine) = due else {
                    startup = false;
                    return false;
                };
                (routine, model)
            };
            let request = TaskRequest {
                provider: aiwin11_core::task::Provider::Ollama,
                prompt: routine.text.clone(),
                model: model.clone(),
                cloud_approved: false,
                cloud_vendor: None,
                local_runtime: aiwin11_core::model_gateway::LocalRuntime::Ollama,
                vision_model: None,
                cloud_fallback_model: None,
                scheduled: true,
                agent: aiwin11_core::policy::AgentProfile::General,
                context: Vec::new(),
                custom_agent: None,
            };
            let outcome = run_reserved_task(&state, &request, |_| {});
            let report = match outcome {
                Ok(outcome) => format!("« {} » : {}", routine.text, outcome["reply"].as_str().unwrap_or_default()),
                Err(code) => format!("« {} » : échec ({code}).", routine.text),
            };
            let _ = lock_core(&state.core).save_report(&report);
            true
        }) == Some(true)
        {}
    }
}

/// Main window only: the user's agents (definitions, never any authorization).
#[tauri::command]
fn get_custom_agents(state: State<'_, Arc<AppState>>) -> Vec<aiwin11_core::agents::AgentDefinition> {
    lock_core(&state.core).custom_agents()
}

#[tauri::command]
fn set_custom_agents(
    state: State<'_, Arc<AppState>>,
    agents: Vec<aiwin11_core::agents::AgentDefinition>,
) -> Result<(), &'static str> {
    lock_core(&state.core).set_custom_agents(agents)
}

#[derive(Serialize)]
struct AgentToolOptions {
    base: aiwin11_core::policy::AgentProfile,
    tools: Vec<&'static str>,
}

/// The tools each built-in agent may use, for the user agents' editor.
#[tauri::command]
fn agent_tool_options() -> Vec<AgentToolOptions> {
    use aiwin11_core::policy::AgentProfile as P;
    [P::Office, P::WindowsAdmin, P::Development, P::Research, P::Multimedia, P::ReadOnly]
        .into_iter()
        .map(|base| AgentToolOptions {
            base,
            tools: MANIFESTS
                .iter()
                .filter(|manifest| base.allows(manifest.action))
                .map(|manifest| manifest.tool)
                .collect(),
        })
        .collect()
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

/// Main window only: ready, disabled, or changed on disk since the user approved it.
#[tauri::command]
fn mcp_server_states(
    state: State<'_, Arc<AppState>>,
) -> std::collections::BTreeMap<String, aiwin11_core::mcp::ServerState> {
    lock_core(&state.core).mcp_server_states()
}

/// Main window only: the user accepts the version now on disk of an updated server.
#[tauri::command]
fn accept_mcp_update(state: State<'_, Arc<AppState>>, name: String) -> Result<(), &'static str> {
    lock_core(&state.core).accept_mcp_update(&name)
}

/// Main window only: started by the user, runs the saved server once to show its protocol
/// version, identity and tools. Journaled; nothing is sent to a model.
#[tauri::command]
async fn test_mcp_server(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<aiwin11_core::mcp::ServerProbe, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let server = {
            let core = lock_core(&state.core);
            let _ = core.journal("mcp.server.tested", None);
            core.mcp_servers().into_iter().find(|server| server.name == name).ok_or("MCP_SERVER_UNKNOWN")?
        };
        aiwin11_core::mcp::probe(&server)
    })
    .await
    .map_err(|_| "MCP_SERVER_UNAVAILABLE")?
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
    backend: aiwin11_core::images::ImageBackend,
    available: bool,
    checkpoint: Option<String>,
    checkpoints: Vec<aiwin11_core::images::ImageCheckpoint>,
}

#[tauri::command]
async fn get_image_engine(state: State<'_, Arc<AppState>>) -> Result<ImageEngine, &'static str> {
    let (endpoint, checkpoint, backend) = {
        let core = lock_core(&state.core);
        (core.image_endpoint().unwrap_or_default(), core.image_checkpoint(), core.image_backend())
    };
    let probe = endpoint.clone();
    let checkpoints =
        tauri::async_runtime::spawn_blocking(move || aiwin11_core::images::list_checkpoints(&probe, backend))
            .await
            .ok()
            .and_then(Result::ok);
    Ok(ImageEngine {
        endpoint,
        backend,
        available: checkpoints.is_some(),
        checkpoint,
        checkpoints: checkpoints.unwrap_or_default(),
    })
}

/// Main window only: the local image engine (loopback HTTP only), or none.
#[tauri::command]
fn set_image_engine(
    state: State<'_, Arc<AppState>>,
    endpoint: String,
    backend: Option<aiwin11_core::images::ImageBackend>,
) -> Result<(), &'static str> {
    let mut core = lock_core(&state.core);
    let backend = backend.unwrap_or_else(|| core.image_backend());
    core.set_image_endpoint(&endpoint, backend)
}

#[tauri::command]
fn set_image_checkpoint(state: State<'_, Arc<AppState>>, checkpoint: String) -> Result<(), &'static str> {
    lock_core(&state.core).set_image_checkpoint(&checkpoint)
}

/// Main window only: the last images AiWin11 generated, newest first.
#[tauri::command]
fn list_generated_images(app: tauri::AppHandle) -> Result<Vec<String>, &'static str> {
    let folder = app.path().app_local_data_dir().map_err(|_| "IMAGE_NOT_FOUND")?.join("images");
    Ok(aiwin11_core::images::list_images(&folder))
}

/// Main window only: one generated image, as PNG bytes (never another file).
#[tauri::command]
fn read_generated_image(app: tauri::AppHandle, name: String) -> Result<tauri::ipc::Response, &'static str> {
    let folder = app.path().app_local_data_dir().map_err(|_| "IMAGE_NOT_FOUND")?.join("images");
    aiwin11_core::images::read_image(&folder, &name).map(tauri::ipc::Response::new)
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
async fn stop_all_tasks(state: State<'_, Arc<AppState>>) -> Result<StopStatus, &'static str> {
    stop_task_worker(&state).await
}

async fn stop_task_worker(state: &AppState) -> Result<StopStatus, &'static str> {
    let (_, code) = state.task_worker.control(aiwin11_model_ipc::TaskCommand::Stop, String::new()).await?;
    let status = match code.as_str() {
        "stopped" => StopStatus::Stopped,
        "requested" => StopStatus::Requested,
        _ => StopStatus::Idle,
    };
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
async fn resolve_approval(state: State<'_, Arc<AppState>>, approval_id: String, approved: bool) -> Result<(), String> {
    use aiwin11_model_ipc::TaskCommand;
    let (accepted, code) = state
        .task_worker
        .control(if approved { TaskCommand::Approve } else { TaskCommand::Decline }, approval_id)
        .await?;
    if accepted {
        Ok(())
    } else {
        Err(code)
    }
}

/// Brings the main window to the front (tray, global shortcut).
fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn main() {
    #[cfg(windows)]
    if let Some(code) = aiwin11_model_ipc::run_worker() {
        std::process::exit(code);
    }
    let launched = Instant::now();
    let mut builder = tauri::Builder::default().plugin(tauri_plugin_notification::init());
    if updates_configured() {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }
    builder
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .setup(move |app| {
            let data_dir = app.path().app_local_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let store = data_dir.join("aiwin11.sqlite3");
            let session = aiwin11_model_ipc::initialize_core(&store)?;
            let mut core = Core::attach_task_worker(&store, &session, std::process::id())?;
            core.set_broker_transport(aiwin11_model_ipc::broker_dispatch);
            // The user's folder if one was chosen and is still allowed; otherwise the dedicated one.
            let files = core
                .workspace()
                .and_then(|workspace| LocalFiles::open_with_workspace(&data_dir, &workspace).ok())
                .or_else(|| LocalFiles::open(&data_dir).ok())
                .map(|primary| FolderSet::with_extra(primary, &data_dir, &core.extra_folders()));
            let video = match core.video_ticket() {
                Ok(Some(_)) => VideoJob::Failed { code: "VIDEO_TRACKING_PENDING", can_resume: true },
                Ok(None) => VideoJob::Idle,
                Err(code) => VideoJob::Failed { code, can_resume: true },
            };
            app.manage(Arc::new(AppState {
                video: Mutex::new(video),
                video_cancel: std::sync::atomic::AtomicBool::new(false),
                files,
                core: Mutex::new(core),
                task: TaskSlot::default(),
                task_worker: aiwin11_model_ipc::TaskSupervisor::default(),
                desktop: desktop::DesktopState::default(),
                vision: Mutex::new(()),
                launched,
                ui_ready_ms: OnceLock::new(),
                store,
                turns: Mutex::new(Vec::new()),
            }));
            let scheduler = Arc::clone(app.state::<Arc<AppState>>().inner());
            std::thread::spawn(move || run_scheduler(scheduler));
            desktop::install(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            desktop::show_palette,
            desktop::overlay_status,
            desktop::overlay_dismiss,
            desktop::overlay_open_workspace,
            desktop::take_workspace_request,
            desktop::overlay_rendered,
            get_snapshot,
            discover_models,
            run_task,
            get_task_replay,
            cancel_task,
            pause_task,
            resume_task,
            stop_all_tasks,
            get_grants,
            get_admin_helper,
            get_privacy,
            get_cloud_fallback,
            get_local_strategy,
            get_agent_models,
            get_local_runtime_preferences,
            set_local_runtime_preferences,
            discover_lm_studio_models,
            set_agent_models,
            set_local_strategy,
            get_memories,
            index_local_text,
            indexed_file_names,
            remove_indexed_file,
            search_indexed_files,
            get_routines,
            get_image_engine,
            set_image_engine,
            set_image_checkpoint,
            open_images_folder,
            list_generated_images,
            read_generated_image,
            set_floating,
            get_mcp_servers,
            get_custom_agents,
            set_custom_agents,
            agent_tool_options,
            set_mcp_servers,
            mcp_server_states,
            accept_mcp_update,
            test_mcp_server,
            get_reports,
            set_routine_schedule,
            dictate,
            get_speech_recognizers,
            set_speech_recognizer,
            set_workspace,
            get_extra_folders,
            session_context_size,
            clear_session_context,
            set_extra_folders,
            get_embedding_model,
            get_memory_retention,
            set_memory_retention,
            set_embedding_model,
            save_routine,
            update_routine,
            delete_memory,
            get_cloud_vendor,
            get_cloud_providers,
            set_cloud_provider_enabled,
            probe_cloud_model,
            set_cloud_vendor,
            get_cloud_budget,
            get_cloud_cost,
            get_local_fallback_model,
            get_protected_files,
            get_protected_windows,
            set_protected_windows,
            get_blocked_applications,
            set_blocked_applications,
            list_backups,
            backup_data,
            schedule_restore,
            cancel_restore,
            get_cloud_usage,
            get_runtime_status,
            ui_ready,
            ui_navigation,
            video_status,
            start_video,
            resume_video,
            get_video_settings,
            set_video_engine,
            set_video_quota,
            cancel_video,
            forget_video_tracking,
            updates_configured,
            check_update,
            install_update,
            set_cloud_budget,
            set_cloud_cost,
            set_local_fallback_model,
            set_protected_files,
            set_cloud_fallback,
            get_model_resources,
            probe_local_model,
            get_model_manifests,
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
