use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};

use crate::state::AppState;

const TRAY_ID: &str = "tokenvane-tray";
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
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "退出 TokenVane", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &scan_item, &float_item, &quit_item])?;

    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("TokenVane — AI Token & 配额监测")
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
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = toggle_floating_window_internal(&app_handle);
                });
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
    .title("TokenVane Quick View")
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

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(popup) = ensure_tray_popup(&app_handle) {
            position_tray_popup(&popup, click_pos);
            let _ = popup.show();
            let _ = popup.set_focus();
        }
    });
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
        let x = pos.x as i32 - w / 2;
        let y = pos.y as i32 - h - 12;
        let _ = popup.set_position(PhysicalPosition::new(x, y));
        return;
    }

    if let Ok(Some(monitor)) = popup.primary_monitor() {
        let m_pos = monitor.position();
        let m_size = monitor.size();
        let x = (m_pos.x + m_size.width as i32 - w - 16).max(m_pos.x + 12);
        let y = (m_pos.y + m_size.height as i32 - h - 56).max(m_pos.y + 12);
        let _ = popup.set_position(PhysicalPosition::new(x, y));
    }
}

fn position_floating_window(win: &tauri::WebviewWindow) {
    let size = win
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(304, 248));
    let w = size.width as i32;
    if let Ok(Some(monitor)) = win.primary_monitor() {
        let m_pos = monitor.position();
        let m_size = monitor.size();
        let x = (m_pos.x + m_size.width as i32 - w - 24).max(m_pos.x + 12);
        let y = m_pos.y + 64;
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
}

fn ensure_floating_window_on_screen(win: &tauri::WebviewWindow) {
    let Ok(pos) = win.outer_position() else {
        position_floating_window(win);
        return;
    };
    let size = win
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(304, 248));
    let w = (size.width as i32).max(148);
    let h = (size.height as i32).max(44);
    let Ok(monitors) = win.available_monitors() else {
        return;
    };
    if monitors.is_empty() {
        return;
    }
    let visible_on_any = monitors.iter().any(|m| {
        let mp = m.position();
        let ms = m.size();
        let m_right = mp.x + ms.width as i32;
        let m_bottom = mp.y + ms.height as i32;
        let overlap_w = (pos.x + w).min(m_right) - pos.x.max(mp.x);
        let overlap_h = (pos.y + h).min(m_bottom) - pos.y.max(mp.y);
        overlap_w >= 48 && overlap_h >= 32
    });
    if !visible_on_any {
        position_floating_window(win);
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
    .title("TokenVane 实时悬浮窗")
    .inner_size(304.0, 248.0)
    .min_inner_size(148.0, 44.0)
    .max_inner_size(640.0, 640.0)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
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
            let _ = win.unminimize();
            ensure_floating_window_on_screen(&win);
            let _ = win.set_always_on_top(true);
            // Rely on `.show()` result directly instead of polling `is_visible()` synchronously,
            // which races Win32/WebView2 show transitions and previously persisted `false` to SQLite.
            return win.show().is_ok();
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

pub fn toggle_floating_window_with(app: &AppHandle, visible: Option<bool>) -> bool {
    let target = match visible {
        Some(v) => v,
        None => {
            let currently_visible = app
                .get_webview_window(FLOATING_MONITOR_LABEL)
                .and_then(|w| w.is_visible().ok())
                .unwrap_or(false);
            !currently_visible
        }
    };
    set_floating_window_and_persist(app, target)
}

pub fn toggle_floating_window_internal(app: &AppHandle) -> bool {
    toggle_floating_window_with(app, None)
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
                (cw.clamp(148.0, 640.0), ch.clamp(44.0, 640.0))
            }
            _ if compact => (264.0, 48.0),
            _ => (304.0, 248.0),
        };
        let _ = win.set_size(LogicalSize::new(w, h));
    }
}
