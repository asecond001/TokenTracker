use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};

use crate::state::AppState;

const TRAY_ID: &str = "tokentracker-tray";
pub const TRAY_POPUP_LABEL: &str = "tray-popup";
pub const FLOATING_MONITOR_LABEL: &str = "floating-monitor";
const MENU_OPEN: &str = "tray_open_main";
const MENU_SCAN: &str = "tray_scan_now";
const MENU_FLOAT: &str = "tray_toggle_floating";
const MENU_QUIT: &str = "tray_quit";

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, MENU_OPEN, "打开工作台", true, None::<&str>)?;
    let scan_item = MenuItem::with_id(app, MENU_SCAN, "立即增量扫描", true, None::<&str>)?;
    let float_item = MenuItem::with_id(app, MENU_FLOAT, "切换实时悬浮窗", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "退出 TokenTracker", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &scan_item, &float_item, &quit_item])?;

    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("TokenTracker — AI Token & 配额监测")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            MENU_OPEN => {
                show_main_window(app);
            }
            MENU_SCAN => {
                let app_handle = app.clone();
                let coord = app.state::<AppState>().coordinator.clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(Ok(report)) =
                        tauri::async_runtime::spawn_blocking(move || coord.scan_now()).await
                    {
                        let _ = app_handle.emit("usage-updated", &report);
                    }
                });
            }
            MENU_FLOAT => {
                let _ = toggle_floating_window_internal(app);
            }
            MENU_QUIT => {
                let state = app.state::<AppState>();
                state.shutdown_background_services();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                let app = tray.app_handle();
                toggle_tray_popup(app, Some(position));
            }
        })
        .build(app)?;

    Ok(())
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
    if let Some(popup) = app.get_webview_window(TRAY_POPUP_LABEL) {
        let _ = popup.hide();
    }
}

fn ensure_tray_popup(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(existing) = app.get_webview_window(TRAY_POPUP_LABEL) {
        return Some(existing);
    }

    let builder = WebviewWindowBuilder::new(
        app,
        TRAY_POPUP_LABEL,
        WebviewUrl::App("index.html?view=tray".into()),
    )
    .title("TokenTracker Quick View")
    .inner_size(360.0, 480.0)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false);

    let popup = builder.build().ok()?;
    let popup_clone = popup.clone();
    popup.on_window_event(move |ev| {
        if let WindowEvent::Focused(false) = ev {
            let _ = popup_clone.hide();
        }
    });

    if let Ok(prefs) = app.state::<AppState>().storage.get_preferences() {
        let _ = popup.set_theme(crate::commands::parse_tauri_theme(&prefs.theme));
    }
    Some(popup)
}

pub fn toggle_tray_popup(app: &AppHandle, click_pos: Option<PhysicalPosition<f64>>) {
    if let Some(popup) = app.get_webview_window(TRAY_POPUP_LABEL) {
        if popup.is_visible().unwrap_or(false) {
            let _ = popup.hide();
            return;
        }
        position_tray_popup(&popup, click_pos);
        let _ = popup.show();
        let _ = popup.set_focus();
        return;
    }

    if let Some(popup) = ensure_tray_popup(app) {
        position_tray_popup(&popup, click_pos);
        let _ = popup.show();
        let _ = popup.set_focus();
    }
}

fn position_tray_popup(
    popup: &tauri::WebviewWindow,
    click_pos: Option<PhysicalPosition<f64>>,
) {
    let size = popup
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(360, 480));
    let w = size.width as i32;
    let h = size.height as i32;

    if let Some(pos) = click_pos {
        let x = (pos.x as i32 - w / 2).max(12);
        let y = (pos.y as i32 - h - 12).max(12);
        let _ = popup.set_position(PhysicalPosition::new(x, y));
        return;
    }

    if let Ok(Some(monitor)) = popup.primary_monitor() {
        let m_size = monitor.size();
        let x = (m_size.width as i32 - w - 16).max(12);
        let y = (m_size.height as i32 - h - 56).max(12);
        let _ = popup.set_position(PhysicalPosition::new(x, y));
    }
}

fn position_floating_window(win: &tauri::WebviewWindow) {
    let size = win
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(304, 248));
    let w = size.width as i32;
    if let Ok(Some(monitor)) = win.primary_monitor() {
        let m_size = monitor.size();
        let x = (m_size.width as i32 - w - 24).max(12);
        let _ = win.set_position(PhysicalPosition::new(x, 64));
    }
}

fn ensure_floating_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(existing) = app.get_webview_window(FLOATING_MONITOR_LABEL) {
        return Some(existing);
    }

    let builder = WebviewWindowBuilder::new(
        app,
        FLOATING_MONITOR_LABEL,
        WebviewUrl::App("index.html?view=floating".into()),
    )
    .title("TokenTracker")
    .inner_size(304.0, 248.0)
    .min_inner_size(220.0, 48.0)
    .decorations(false)
    .transparent(true)
    .resizable(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false);

    let win = builder.build().ok()?;
    position_floating_window(&win);
    if let Ok(prefs) = app.state::<AppState>().storage.get_preferences() {
        let _ = win.set_theme(crate::commands::parse_tauri_theme(&prefs.theme));
    }
    Some(win)
}

pub fn set_floating_window_visible(app: &AppHandle, visible: bool) -> bool {
    if visible {
        if let Some(win) = ensure_floating_window(app) {
            if !win.is_visible().unwrap_or(false) {
                let _ = win.show();
            }
            return win.is_visible().unwrap_or(true);
        }
        false
    } else {
        if let Some(win) = app.get_webview_window(FLOATING_MONITOR_LABEL) {
            let _ = win.hide();
        }
        false
    }
}

pub fn set_floating_window_and_persist(app: &AppHandle, visible: bool) -> bool {
    let next = set_floating_window_visible(app, visible);
    let storage = &app.state::<AppState>().storage;
    if let Ok(mut prefs) = storage.get_preferences() {
        if prefs.floating_window_enabled != next {
            prefs.floating_window_enabled = next;
            if storage.update_preferences(&prefs).is_ok() {
                if let Ok(updated) = storage.get_preferences() {
                    let _ = app.emit("preferences-updated", &updated);
                }
            }
        }
    }
    next
}

pub fn toggle_floating_window_internal(app: &AppHandle) -> bool {
    let currently_visible = app
        .get_webview_window(FLOATING_MONITOR_LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    set_floating_window_and_persist(app, !currently_visible)
}

pub fn resize_floating_window_internal(
    app: &AppHandle,
    compact: bool,
    width: Option<f64>,
    height: Option<f64>,
) {
    if let Some(win) = app.get_webview_window(FLOATING_MONITOR_LABEL) {
        let (w, h) = match (width, height) {
            (Some(cw), Some(ch)) if cw.is_finite() && ch.is_finite() => {
                (cw.clamp(220.0, 640.0), ch.clamp(48.0, 560.0))
            }
            _ if compact => (288.0, 52.0),
            _ => (304.0, 248.0),
        };
        let _ = win.set_size(LogicalSize::new(w, h));
    }
}
