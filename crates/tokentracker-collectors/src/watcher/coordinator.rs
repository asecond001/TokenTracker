use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use chrono::Utc;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokentracker_core::domain::ProviderConfig;
use tokentracker_core::Storage;
use walkdir::WalkDir;

use crate::error::CollectorResult;
use crate::providers::antigravity::AntigravityParser;
use crate::providers::claude_code::ClaudeCodeParser;
use crate::providers::codex::CodexParser;
use crate::providers::ProviderLogParser;
use crate::watcher::tailer::tail_file_incremental;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanReport {
    pub files_scanned: usize,
    pub events_inserted: usize,
    pub events_deduplicated: usize,
    pub errors: Vec<String>,
    pub scanned_at_ms: i64,
}

/// Coordinates file discovery, incremental tailing, and live FS watching across all enabled providers.
#[derive(Clone)]
pub struct CollectorCoordinator {
    storage: Storage,
    parsers: Vec<Arc<dyn ProviderLogParser>>,
    scan_lock: Arc<Mutex<()>>,
    last_report: Arc<RwLock<Option<ScanReport>>>,
    watcher_running: Arc<RwLock<bool>>,
    dirty_notify: Arc<Notify>,
}

impl CollectorCoordinator {
    pub fn new(storage: Storage) -> Self {
        let parsers: Vec<Arc<dyn ProviderLogParser>> = vec![
            Arc::new(ClaudeCodeParser::new()),
            Arc::new(CodexParser::new()),
            Arc::new(AntigravityParser::new()),
        ];
        Self {
            storage,
            parsers,
            scan_lock: Arc::new(Mutex::new(())),
            last_report: Arc::new(RwLock::new(None)),
            watcher_running: Arc::new(RwLock::new(false)),
            dirty_notify: Arc::new(Notify::new()),
        }
    }

    pub fn is_watching(&self) -> bool {
        self.watcher_running
            .read()
            .map(|g| *g)
            .unwrap_or(false)
    }

    pub fn last_scan_report(&self) -> Option<ScanReport> {
        self.last_report.read().ok().and_then(|g| g.clone())
    }

    /// Execute an immediate synchronous incremental scan across all enabled providers.
    pub fn scan_now(&self) -> CollectorResult<ScanReport> {
        let _guard = self
            .scan_lock
            .lock()
            .map_err(|_| crate::error::CollectorError::Compliance("Scan lock poisoned".into()))?;

        let device_id = self.storage.local_device()?.device_id;
        let configs = self.storage.list_provider_configs()?;
        let mut report = ScanReport {
            scanned_at_ms: Utc::now().timestamp_millis(),
            ..Default::default()
        };

        for parser in &self.parsers {
            let cfg = configs
                .iter()
                .find(|c| c.provider_id == parser.provider_id());
            if let Some(c) = cfg {
                if !c.enabled {
                    continue;
                }
            }

            let files = discover_files_for_parser(parser.as_ref(), cfg);
            let mut provider_err: Option<String> = None;

            for file_path in files {
                match tail_file_incremental(&self.storage, parser.as_ref(), &file_path, &device_id)
                {
                    Ok(out) => {
                        report.files_scanned += 1;
                        report.events_inserted += out.events_inserted;
                        report.events_deduplicated += out.events_deduplicated;
                    }
                    Err(err) => {
                        let msg = format!(
                            "[{}] {}: {}",
                            parser.provider_id(),
                            file_path.display(),
                            err
                        );
                        provider_err = Some(msg.clone());
                        report.errors.push(msg);
                    }
                }
            }

            if let Some(err_msg) = provider_err {
                let _ = self.storage.set_provider_sync_status(
                    parser.provider_id(),
                    Some(report.scanned_at_ms),
                    Some(&err_msg),
                );
            } else {
                let _ = self.storage.set_provider_sync_status(
                    parser.provider_id(),
                    Some(report.scanned_at_ms),
                    None,
                );
            }
        }

        if let Ok(mut guard) = self.last_report.write() {
            *guard = Some(report.clone());
        }
        Ok(report)
    }

    /// Start background filesystem watcher (with 300ms debounce) + 15s fallback sweep.
    ///
    /// Must be called from within an active Tokio runtime context; callers outside a Tokio
    /// context should spawn [`Self::run_background_watcher`] onto their runtime handle.
    pub fn start_background_watcher(
        &self,
        shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> JoinHandle<()> {
        tokio::spawn(self.clone().run_background_watcher(shutdown_rx))
    }

    /// Run the background filesystem watcher loop (300ms debounce + 15s sweep) until shutdown.
    pub async fn run_background_watcher(self, shutdown_rx: tokio::sync::watch::Receiver<bool>) {
        self.run_background_watcher_with_hook(shutdown_rx, |_| {})
            .await;
    }

    /// Run the background filesystem watcher loop and invoke `on_scan` after each successful scan.
    pub async fn run_background_watcher_with_hook<F>(
        self,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
        on_scan: F,
    ) where
        F: Fn(&ScanReport) + Send + Sync + 'static,
    {
        let on_scan = Arc::new(on_scan);

        // Perform initial scan on startup without blocking UI boot.
        let c0 = self.clone();
        if let Ok(Ok(report)) = tokio::task::spawn_blocking(move || c0.scan_now()).await {
            on_scan(&report);
        }

        let notify_tx = self.dirty_notify.clone();
        let mut watcher_opt = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if res.is_ok() {
                    notify_tx.notify_one();
                }
            },
            notify::Config::default(),
        )
        .ok();

        if let Some(ref mut watcher) = watcher_opt {
            for root in self.collect_watch_roots() {
                if root.exists() {
                    let _ = watcher.watch(&root, RecursiveMode::Recursive);
                }
            }
            if let Ok(mut w) = self.watcher_running.write() {
                *w = true;
            }
        }

        let mut sweep_interval = tokio::time::interval(Duration::from_secs(15));
        sweep_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
                _ = self.dirty_notify.notified() => {
                    // Coalesce rapid append writes (300ms debounce) before running incremental scan.
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let c = self.clone();
                    if let Ok(Ok(report)) = tokio::task::spawn_blocking(move || c.scan_now()).await {
                        on_scan(&report);
                    }
                }
                _ = sweep_interval.tick() => {
                    if let Some(ref mut watcher) = watcher_opt {
                        for root in self.collect_watch_roots() {
                            if root.exists() {
                                let _ = watcher.watch(&root, RecursiveMode::Recursive);
                            }
                        }
                    }
                    let c = self.clone();
                    if let Ok(Ok(report)) = tokio::task::spawn_blocking(move || c.scan_now()).await {
                        on_scan(&report);
                    }
                }
            }
        }

        if let Ok(mut w) = self.watcher_running.write() {
            *w = false;
        }
    }

    fn collect_watch_roots(&self) -> Vec<PathBuf> {
        let configs = self.storage.list_provider_configs().unwrap_or_default();
        let mut roots = BTreeSet::new();
        for parser in &self.parsers {
            for r in parser.default_roots() {
                roots.insert(r);
            }
            if let Some(cfg) = configs
                .iter()
                .find(|c| c.provider_id == parser.provider_id())
            {
                for custom in &cfg.custom_log_paths {
                    let p = PathBuf::from(custom);
                    if p.is_file() {
                        if let Some(parent) = p.parent() {
                            roots.insert(parent.to_path_buf());
                        }
                    } else {
                        roots.insert(p);
                    }
                }
            }
        }
        roots.into_iter().collect()
    }
}

/// Discover matching log files for a provider across default and user-configured paths.
pub fn discover_files_for_parser(
    parser: &dyn ProviderLogParser,
    config: Option<&ProviderConfig>,
) -> Vec<PathBuf> {
    let mut roots = parser.default_roots();
    if let Some(cfg) = config {
        for p in &cfg.custom_log_paths {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                roots.push(PathBuf::from(trimmed));
            }
        }
    }

    let mut matched = BTreeSet::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        if root.is_file() {
            if parser.matches_file(&root) {
                matched.insert(root);
            }
            continue;
        }
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() && parser.matches_file(entry.path()) {
                matched.insert(entry.into_path());
            }
        }
    }

    matched.into_iter().collect()
}

/// Helper to test whether a single path would be accepted by any Phase 1 parser.
pub fn path_matches_provider(parser: &dyn ProviderLogParser, path: &Path) -> bool {
    parser.matches_file(path)
}
