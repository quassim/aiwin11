#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    fs,
    sync::{Arc, Mutex},
};

use aiwin11_core::{
    grants::Grant,
    local_files::LocalFiles,
    manifest::{ToolManifest, MANIFESTS},
    model_gateway::{LocalModelProvider, Ollama},
    openai,
    policy::{self, AgentAction, Effect, PolicySettings},
    task::{StopStatus, TaskOutcome, TaskRequest, TaskSlot, TaskUpdate},
    windows_bridge::{Capture, Inspection},
    Core, ModelPreferences,
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
async fn get_snapshot(state: State<'_, Arc<AppState>>) -> Result<Snapshot, &'static str> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || snapshot(&state)).await.map_err(|_| "CORE_UNAVAILABLE")?
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
        Ollama::connect().and_then(|provider| provider.discover()).map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")
    })
    .await
    .map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?
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
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let _analysis = state.vision.try_lock().map_err(|_| "VISION_BUSY")?;
        Core::analyze_capture(&state.core, &model, &jpeg)
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
    Ok(HistorySettings { retention_days: state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.retention_days() })
}

#[tauri::command]
fn set_retention_days(state: State<'_, Arc<AppState>>, days: u32) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_retention_days(days)
}

/// Erases past sessions. Refused while a task runs, so that its audit trail stays whole.
#[tauri::command]
fn clear_history(state: State<'_, Arc<AppState>>) -> Result<usize, &'static str> {
    if state.task.is_running() {
        return Err("TASK_BUSY");
    }
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.clear_history()
}

/// Exports the audit journal to `%LOCALAPPDATA%\app.aiwin11.desktop\exports` and returns the path.
#[tauri::command]
async fn export_audit(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<String, &'static str> {
    let directory = app.path().app_local_data_dir().map_err(|_| "EXPORT_FAILED")?.join("exports");
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let path = state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.export_audit_to(&directory)?;
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
    let active = state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.active_grants()?;
    let grantable = AgentAction::ALL.into_iter().filter(|action| policy::grantable(*action)).collect();
    Ok(GrantOverview { active, grantable })
}

/// Main window only: the user lets one action run without asking for a limited time.
#[tauri::command]
fn create_grant(state: State<'_, Arc<AppState>>, action: AgentAction, minutes: u32) -> Result<String, &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.grant_temporarily(action, minutes)
}

#[tauri::command]
fn get_cloud_fallback(state: State<'_, Arc<AppState>>) -> Result<bool, &'static str> {
    Ok(state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.cloud_fallback_enabled())
}

#[tauri::command]
fn set_cloud_fallback(state: State<'_, Arc<AppState>>, enabled: bool) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_cloud_fallback(enabled)
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
fn get_cloud_budget(state: State<'_, Arc<AppState>>) -> Result<u64, &'static str> {
    Ok(state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.cloud_token_budget())
}

#[tauri::command]
fn set_cloud_budget(state: State<'_, Arc<AppState>>, tokens: u64) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_cloud_token_budget(tokens)
}

fn cloud_vendor(state: &State<'_, Arc<AppState>>) -> Result<openai::Vendor, &'static str> {
    Ok(state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.cloud_vendor())
}

#[tauri::command]
fn get_cloud_vendor(state: State<'_, Arc<AppState>>) -> Result<openai::Vendor, &'static str> {
    cloud_vendor(&state)
}

/// Main window only: chooses the cloud provider (each keeps its own key).
#[tauri::command]
fn set_cloud_vendor(state: State<'_, Arc<AppState>>, vendor: openai::Vendor) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_cloud_vendor(vendor)
}

#[tauri::command]
fn get_memories(state: State<'_, Arc<AppState>>) -> Result<Vec<aiwin11_core::memory::MemoryEntry>, &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.memories()
}

/// Main window only: deletes one note, or all of them without an ID.
#[tauri::command]
fn delete_memory(state: State<'_, Arc<AppState>>, id: Option<String>) -> Result<usize, &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.forget(id.as_deref())
}

#[tauri::command]
fn get_embedding_model(state: State<'_, Arc<AppState>>) -> Result<String, &'static str> {
    Ok(state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.embedding_model().unwrap_or_default())
}

#[tauri::command]
fn set_embedding_model(state: State<'_, Arc<AppState>>, model: String) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_embedding_model(&model)
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
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_workspace(&stored)?;
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
fn get_routines(state: State<'_, Arc<AppState>>) -> Result<Vec<aiwin11_core::memory::MemoryEntry>, &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.routines()
}

/// Main window only: saves the current request; it only runs when the user sends it.
#[tauri::command]
fn save_routine(state: State<'_, Arc<AppState>>, text: String) -> Result<String, &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.save_routine(&text)
}

#[tauri::command]
fn get_privacy(state: State<'_, Arc<AppState>>) -> Result<bool, &'static str> {
    Ok(state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.private_mode())
}

/// Main window only: private mode blocks every cloud request.
#[tauri::command]
fn set_privacy(state: State<'_, Arc<AppState>>, enabled: bool) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_private_mode(enabled)
}

#[tauri::command]
fn get_admin_helper(state: State<'_, Arc<AppState>>) -> Result<bool, &'static str> {
    Ok(state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.admin_helper_enabled())
}

/// Main window only: turns the elevated helper (closed list of administrator operations) on or off.
#[tauri::command]
fn set_admin_helper(state: State<'_, Arc<AppState>>, enabled: bool) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_admin_helper_enabled(enabled)
}

#[tauri::command]
fn revoke_grant(state: State<'_, Arc<AppState>>, id: String) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.revoke_grant(&id)
}

/// "Stoppe toutes les tâches": always available, journaled with its immediate result. The stop
/// is confirmed when the running task ends as cancelled (`task.cancelled`).
#[tauri::command]
fn stop_all_tasks(state: State<'_, Arc<AppState>>) -> Result<StopStatus, &'static str> {
    let status = state.task.stop_all();
    if let Ok(core) = state.core.lock() {
        let _ = core.journal("tasks.stop.requested", Some(status.code()));
    }
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
    let settings = state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.policy_settings();
    let table =
        policy::table(settings.simulate).into_iter().map(|(action, effects)| PolicyRow { action, effects }).collect();
    Ok(PolicyOverview { settings, table, manifests: &MANIFESTS })
}

/// Applies to the next task; a running task keeps the settings it started with.
#[tauri::command]
fn set_policy_settings(state: State<'_, Arc<AppState>>, settings: PolicySettings) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_policy_settings(settings).map_err(|_| "SETTINGS_UNAVAILABLE")
}

#[tauri::command]
fn get_model_preferences(state: State<'_, Arc<AppState>>) -> Result<ModelPreferences, &'static str> {
    Ok(state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.model_preferences())
}

#[tauri::command]
fn set_model_preferences(state: State<'_, Arc<AppState>>, preferences: ModelPreferences) -> Result<(), &'static str> {
    state.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_model_preferences(preferences)
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
            if let Ok(core) = state.core.lock() {
                let _ = core.journal("tasks.stop.requested", Some(status.code()));
            };
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
                files,
                core: Mutex::new(core),
                task: TaskSlot::default(),
                vision: Mutex::new(()),
            }));
            // Pay the bridge's first start (runtime load, antivirus scan) now, not within a
            // command's deadline. The spare process waits idle for an authorized command.
            std::thread::spawn(aiwin11_core::windows_bridge::prewarm_bridge);
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
            get_memories,
            get_routines,
            dictate,
            set_workspace,
            get_embedding_model,
            set_embedding_model,
            save_routine,
            delete_memory,
            get_cloud_vendor,
            set_cloud_vendor,
            get_cloud_budget,
            set_cloud_budget,
            set_cloud_fallback,
            get_model_resources,
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
            capture_foreground,
            discover_vision_models,
            analyze_capture,
            enter_character_map_digit,
            local_files_root,
            preview_local_text,
            copy_local_text,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AiWin11");
}
