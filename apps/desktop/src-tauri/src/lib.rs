#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod commands;
pub mod state;
pub mod tray;

use std::fs::OpenOptions;
use std::io::Write;

use chrono::Utc;
use tauri::{Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use crate::state::{default_db_path, AppState};
use crate::tray::setup_tray;

fn log_startup_error(context: &str, err: &dyn std::fmt::Display) {
    eprintln!("{context}: {err}");
    if let Some(dir) = default_db_path().parent() {
        let _ = std::fs::create_dir_all(dir);
        let log_path = dir.join("startup-error.log");
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
            let _ = writeln!(file, "[{}] {context}: {err}", Utc::now().to_rfc3339());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let app_state = match AppState::initialize() {
        Ok(s) => s,
        Err(err) => {
            log_startup_error("TokenTracker initialization failed", &err);
            return;
        }
    };

    let builder = tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap_state,
            commands::get_overview,
            commands::get_trends,
            commands::get_monthly,
            commands::get_models_pricing,
            commands::upsert_model_pricing_override,
            commands::delete_model_pricing_override,
            commands::recompute_all_costs,
            commands::get_subscriptions,
            commands::upsert_subscription_plan,
            commands::get_speed_latency,
            commands::get_tray_summary,
            commands::get_provider_configs,
            commands::update_provider_config,
            commands::get_ingestion_cursors,
            commands::trigger_scan_now,
            commands::set_provider_api_key,
            commands::delete_provider_api_key,
            commands::reconcile_provider_official_api,
            commands::get_preferences,
            commands::update_preferences,
            commands::export_monthly_report,
            commands::preview_sync_envelope_v1,
            commands::open_main_workspace,
            commands::quit_application,
        ])
        .setup(|app| {
            if let Err(err) = setup_tray(app.handle()) {
                log_startup_error("TokenTracker tray setup failed", &err);
                return Err(err.into());
            }
            let state = app.state::<AppState>();
            state.start_background_services(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    let state = window.state::<AppState>();
                    let minimize = state
                        .storage
                        .get_preferences()
                        .map(|p| p.minimize_to_tray)
                        .unwrap_or(true);
                    if minimize {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
        });

    let app = match builder.build(tauri::generate_context!()) {
        Ok(a) => a,
        Err(err) => {
            log_startup_error("Failed to build TokenTracker Tauri runtime", &err);
            return;
        }
    };

    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            let state = app_handle.state::<AppState>();
            state.shutdown_background_services();
        }
    });
}
