use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tokio::sync::watch;
use tokentracker_collectors::{
    start_loopback_otlp_server, sync_catalog_and_rate, CollectorCoordinator, CredentialManager,
    OsKeyringStore, OtlpServerHandle,
};
use tokentracker_core::domain::CatalogSyncResult;
use tokentracker_core::{CoreResult, Storage};

/// Resolve `%LOCALAPPDATA%\TokenTracker\tokentracker.db` (PLAN.md §4).
pub fn default_db_path() -> PathBuf {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("TokenTracker")
            .join("tokentracker.db");
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home)
            .join(".tokentracker")
            .join("tokentracker.db");
    }
    PathBuf::from("tokentracker.db")
}

/// Shared Tauri managed application state.
pub struct AppState {
    pub db_path: PathBuf,
    pub storage: Storage,
    pub coordinator: CollectorCoordinator,
    pub credentials: Arc<dyn CredentialManager>,
    pub otlp_handle: Mutex<Option<OtlpServerHandle>>,
    pub watcher_shutdown_tx: Mutex<Option<watch::Sender<bool>>>,
    pub catalog_sync_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AppState {
    pub fn initialize() -> Result<Self, String> {
        let db_path = default_db_path();
        let storage = Storage::open_file(&db_path)
            .map_err(|e| format!("Failed to open TokenTracker SQLite database: {e}"))?;
        let coordinator = CollectorCoordinator::new(storage.clone());
        let credentials: Arc<dyn CredentialManager> = Arc::new(OsKeyringStore::new());

        Ok(Self {
            db_path,
            storage,
            coordinator,
            credentials,
            otlp_handle: Mutex::new(None),
            watcher_shutdown_tx: Mutex::new(None),
            catalog_sync_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    pub async fn run_catalog_sync(&self, app: &AppHandle, force: bool) -> CoreResult<CatalogSyncResult> {
        run_catalog_sync_shared(&self.storage, &self.catalog_sync_lock, app, force).await
    }

    pub fn start_background_services(&self, app_handle: AppHandle) {
        let (tx, rx) = watch::channel(false);
        let mut catalog_rx = tx.subscribe();
        let watcher_app = app_handle.clone();
        let coord = self.coordinator.clone();
        tauri::async_runtime::spawn(async move {
            coord
                .run_background_watcher_with_hook(rx, move |report| {
                    if report.events_inserted > 0 {
                        let _ = watcher_app.emit("usage-updated", report);
                    }
                })
                .await;
        });

        let sync_storage = self.storage.clone();
        let sync_lock = self.catalog_sync_lock.clone();
        tauri::async_runtime::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                _ = catalog_rx.changed() => {
                    return;
                }
            }

            let _ = run_catalog_sync_shared(&sync_storage, &sync_lock, &app_handle, false).await;

            let mut interval = tokio::time::interval(Duration::from_secs(6 * 3_600));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Consume immediate initial tick since startup sync just ran
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let _ = run_catalog_sync_shared(&sync_storage, &sync_lock, &app_handle, false).await;
                    }
                    _ = catalog_rx.changed() => {
                        if *catalog_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        if let Ok(mut guard) = self.watcher_shutdown_tx.lock() {
            *guard = Some(tx);
        }

        if let Ok(prefs) = self.storage.get_preferences() {
            if prefs.otlp_enabled {
                if let Err(err) = self.sync_otlp_listener_blocking(true, prefs.otlp_port) {
                    tracing::warn!("OTLP listener startup warning: {err}");
                }
            }
        }
    }

    /// Synchronize the loopback OTLP listener from a non-Tokio thread (such as `.setup`).
    pub fn sync_otlp_listener_blocking(
        &self,
        enabled: bool,
        port: u16,
    ) -> Result<Option<u16>, String> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(
                "sync_otlp_listener_blocking cannot run inside an active Tokio runtime".to_string(),
            );
        }
        tauri::async_runtime::block_on(self.sync_otlp_listener_async(enabled, port))
    }

    /// Synchronize the loopback OTLP listener asynchronously without holding locks across `.await`.
    pub async fn sync_otlp_listener_async(
        &self,
        enabled: bool,
        port: u16,
    ) -> Result<Option<u16>, String> {
        let existing_to_stop = {
            let mut guard = self
                .otlp_handle
                .lock()
                .map_err(|_| "OTLP handle lock poisoned".to_string())?;
            if let Some(existing) = guard.take() {
                if enabled && existing.port == port {
                    let active_port = existing.port;
                    *guard = Some(existing);
                    return Ok(Some(active_port));
                }
                Some(existing)
            } else {
                None
            }
        };

        if let Some(existing) = existing_to_stop {
            existing.stop().await;
        }

        if !enabled {
            return Ok(None);
        }

        let handle = start_loopback_otlp_server(self.storage.clone(), port)
            .await
            .map_err(|e| format!("Failed to start loopback OTLP server on 127.0.0.1:{port}: {e}"))?;

        let actual_port = handle.port;
        let mut guard = self
            .otlp_handle
            .lock()
            .map_err(|_| "OTLP handle lock poisoned".to_string())?;
        *guard = Some(handle);
        Ok(Some(actual_port))
    }

    pub fn otlp_status(&self) -> (bool, Option<u16>) {
        match self.otlp_handle.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(h) => (true, Some(h.port)),
                None => (false, None),
            },
            Err(_) => (false, None),
        }
    }

    pub fn shutdown_background_services(&self) {
        if let Ok(mut guard) = self.watcher_shutdown_tx.lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(true);
            }
        }
        if let Ok(mut guard) = self.otlp_handle.lock() {
            if let Some(handle) = guard.take() {
                handle.signal_shutdown();
            }
        }
    }
}

pub async fn run_catalog_sync_shared(
    storage: &Storage,
    lock: &tokio::sync::Mutex<()>,
    app: &AppHandle,
    force: bool,
) -> CoreResult<CatalogSyncResult> {
    let _guard = lock.lock().await;
    let res = sync_catalog_and_rate(storage, force).await?;
    if res.rate_updated {
        if let Ok(prefs) = storage.get_preferences() {
            let _ = app.emit("preferences-updated", &prefs);
        }
    }
    if res.rate_updated || res.pricing_updated {
        let _ = app.emit("usage-updated", &res);
    }
    Ok(res)
}
