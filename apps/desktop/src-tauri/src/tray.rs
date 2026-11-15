use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

use crate::state::AppState;

const TRAY_ID: &str = "tokentracker-tray";
const TRAY_POPUP_LABEL: &str = "tray-popup";
const MENU_OPEN: &str = "tray_open_main";
const MENU_SCAN: &str = "tray_scan_now";
const MENU_QUIT: &str = "tray_quit";

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, MENU_OPEN, "打开工作台", true, None::<&str>)?;
    let scan_item = MenuItem::with_id(app, MENU_SCAN, "立即增量扫描", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "退出 TokenTracker", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &scan_item, &quit_item])?;

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

    let builder = WebviewWindowBuilder::new(
        app,
        TRAY_POPUP_LABEL,
        WebviewUrl::App("index.html?view=tray".into()),
    )
    .title("TokenTracker Quick View")
    .inner_size(340.0, 410.0)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false);

    if let Ok(popup) = builder.build() {
        let popup_clone = popup.clone();
        popup.on_window_event(move |ev| {
            if let WindowEvent::Focused(false) = ev {
                let _ = popup_clone.hide();
            }
        });
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
        .unwrap_or(tauri::PhysicalSize::new(340, 410));
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
