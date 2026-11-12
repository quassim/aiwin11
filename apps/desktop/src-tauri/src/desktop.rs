use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

use super::{lock_core, show_main, stop_task_worker, AppState};

const LABEL: &str = "overlay";
const WIDTH: f64 = 510.0;
const HEIGHT: f64 = 430.0;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum WorkspaceTarget {
    Home,
    Dashboard,
    Models,
    Security,
    Activity,
}

#[derive(Serialize)]
pub(super) struct WorkspaceRequest {
    target: WorkspaceTarget,
    draft: Option<String>,
}

#[derive(Default)]
pub(super) struct DesktopState {
    workspace: Mutex<Option<WorkspaceRequest>>,
    opening: Mutex<Option<Instant>>,
    palette_shortcut: AtomicBool,
    stop_shortcut: AtomicBool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OverlayStatus {
    task: aiwin11_model_ipc::TaskPresence,
    palette_shortcut: bool,
    stop_shortcut: bool,
}

pub(super) fn install(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let overlay = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
        .title("Palette AiWin11")
        .inner_size(WIDTH, HEIGHT)
        .visible(false)
        .focused(false)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .build()?;
    let window = overlay.clone();
    overlay.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window.hide();
        }
    });

    let open = MenuItem::with_id(app, "open", "Ouvrir AiWin11", true, None::<&str>)?;
    let palette = MenuItem::with_id(app, "palette", "Ouvrir la palette", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "Masquer AiWin11", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stoppe toutes les tâches", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &palette, &hide, &stop, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main").tooltip("AiWin11").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id.as_ref() {
        "open" => show_main(app),
        "palette" => {
            let _ = open_palette(app);
        }
        "hide" => {
            let _ = overlay_dismiss(app.clone());
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
        }
        "stop" => stop_tasks(app),
        "quit" => app.exit(0),
        _ => {}
    })
    .on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            show_main(tray.app_handle());
        }
    })
    .build(app)?;

    let modifiers = Some(Modifiers::CONTROL | Modifiers::ALT);
    let palette_shortcut = Shortcut::new(modifiers, Code::Space);
    let stop_shortcut = Shortcut::new(modifiers, Code::Backspace);
    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, shortcut, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                if *shortcut == palette_shortcut {
                    if app.get_webview_window(LABEL).is_some_and(|window| window.is_visible().unwrap_or(false)) {
                        let _ = overlay_dismiss(app.clone());
                    } else {
                        let _ = open_palette(app);
                    }
                } else if *shortcut == stop_shortcut {
                    stop_tasks(app);
                }
            })
            .build(),
    )?;
    let state = app.state::<Arc<AppState>>();
    state.desktop.palette_shortcut.store(app.global_shortcut().register(palette_shortcut).is_ok(), Ordering::Release);
    state.desktop.stop_shortcut.store(app.global_shortcut().register(stop_shortcut).is_ok(), Ordering::Release);
    Ok(())
}

fn open_palette(app: &AppHandle) -> Result<(), &'static str> {
    let window = app.get_webview_window(LABEL).ok_or("OVERLAY_UNAVAILABLE")?;
    let state = app.state::<Arc<AppState>>();
    *state.desktop.opening.lock().map_err(|_| "OVERLAY_UNAVAILABLE")? = Some(Instant::now());
    if let Some(monitor) =
        window.cursor_position().ok().and_then(|cursor| window.monitor_from_point(cursor.x, cursor.y).ok().flatten())
    {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let x = (f64::from(area.size.width) - WIDTH * scale).max(0.0) / 2.0;
        let y = (f64::from(area.size.height) - HEIGHT * scale).max(0.0) / 3.0;
        let _ = window.set_position(PhysicalPosition::new(
            area.position.x + x.round() as i32,
            area.position.y + y.round() as i32,
        ));
    }
    window.show().map_err(|_| "OVERLAY_UNAVAILABLE")?;
    window.set_focus().map_err(|_| "OVERLAY_UNAVAILABLE")?;
    app.emit_to(LABEL, "desktop-palette-opened", ()).map_err(|_| "OVERLAY_UNAVAILABLE")
}

#[tauri::command]
pub(super) fn show_palette(app: AppHandle) -> Result<(), &'static str> {
    open_palette(&app)
}

#[tauri::command]
pub(super) fn overlay_status(state: State<'_, Arc<AppState>>) -> OverlayStatus {
    OverlayStatus {
        task: state.task_worker.presence(),
        palette_shortcut: state.desktop.palette_shortcut.load(Ordering::Acquire),
        stop_shortcut: state.desktop.stop_shortcut.load(Ordering::Acquire),
    }
}

#[tauri::command]
pub(super) fn overlay_dismiss(app: AppHandle) -> Result<(), &'static str> {
    let window = app.get_webview_window(LABEL).ok_or("OVERLAY_UNAVAILABLE")?;
    window.hide().map_err(|_| "OVERLAY_UNAVAILABLE")
}

#[tauri::command]
pub(super) fn overlay_open_workspace(
    app: AppHandle,
    target: WorkspaceTarget,
    draft: Option<String>,
) -> Result<(), &'static str> {
    if let Some(text) = draft.as_deref() {
        if !matches!(target, WorkspaceTarget::Home)
            || text.chars().count() > aiwin11_core::task::MAX_PROMPT_CHARS
            || text.contains('\0')
        {
            return Err("PROMPT_INVALID");
        }
    }
    let state = app.state::<Arc<AppState>>();
    if draft.is_some() && !matches!(state.task_worker.presence(), aiwin11_model_ipc::TaskPresence::Idle) {
        return Err("TASK_BUSY");
    }
    *state.desktop.workspace.lock().map_err(|_| "OVERLAY_UNAVAILABLE")? = Some(WorkspaceRequest { target, draft });
    overlay_dismiss(app.clone())?;
    show_main(&app);
    app.emit_to("main", "desktop-workspace-request", ()).map_err(|_| "OVERLAY_UNAVAILABLE")
}

#[tauri::command]
pub(super) fn take_workspace_request(state: State<'_, Arc<AppState>>) -> Option<WorkspaceRequest> {
    state.desktop.workspace.lock().ok()?.take()
}

#[tauri::command]
pub(super) async fn overlay_rendered(state: State<'_, Arc<AppState>>) -> Result<(), &'static str> {
    let start = state.desktop.opening.lock().map_err(|_| "OVERLAY_UNAVAILABLE")?.take();
    if let Some(start) = start {
        let elapsed = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
        let state = Arc::clone(state.inner());
        tauri::async_runtime::spawn_blocking(move || {
            lock_core(&state.core).record_palette_ready(elapsed).map_err(|_| "ACTIVITY_UNAVAILABLE")
        })
        .await
        .map_err(|_| "ACTIVITY_UNAVAILABLE")??;
    }
    Ok(())
}

fn stop_tasks(app: &AppHandle) {
    let state = Arc::clone(app.state::<Arc<AppState>>().inner());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(status) = stop_task_worker(&state).await {
            match status {
                aiwin11_core::task::StopStatus::Stopped => notify(&app, "Tâche arrêtée"),
                aiwin11_core::task::StopStatus::Requested => notify(&app, "Arrêt demandé"),
                aiwin11_core::task::StopStatus::Idle => {}
            }
        }
    });
}

pub(super) fn notify(app: &AppHandle, message: &'static str) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(format!("AiWin11 · {message}")));
    }
    if ["main", LABEL]
        .into_iter()
        .any(|label| app.get_webview_window(label).is_some_and(|window| window.is_focused().unwrap_or(false)))
    {
        return;
    }
    let _ = app.notification().builder().title("AiWin11").body(message).show();
}
