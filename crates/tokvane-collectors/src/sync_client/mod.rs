use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Mutex, Notify};
use tokvane_core::domain::{EncryptedSyncEnvelopeV1, SyncEnvelopeV1};
use tokvane_core::sync_crypto::{
    decrypt_sync_envelope_v1, encrypt_sync_envelope_v1, generate_kdf_salt_hex,
};
use tokvane_core::{CoreResult, Storage};

use crate::credentials::CredentialManager;
use crate::error::{CollectorError, CollectorResult};

pub const SYNC_BEARER_TOKEN_KEY: &str = "__cloud_sync_bearer_token__";
pub const SYNC_E2EE_PASSPHRASE_KEY: &str = "__cloud_sync_e2ee_passphrase__";
pub const SYNC_CF_API_TOKEN_KEY: &str = "__cloud_sync_cf_api_token__";

const PUSH_DEBOUNCE_SECS: u64 = 3;
const SSE_BACKOFF_SECS: [u64; 4] = [2, 5, 15, 60];
const MAX_PULL_PAGES: usize = 10;
const SSE_IDLE_TIMEOUT_SECS: u64 = 65;

pub type SyncBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Stream abstraction yielding raw SSE chunk bytes so SSE reconnect & parsing can run offline in tests.
pub trait SseByteStream: Send {
    fn next_chunk(&mut self) -> SyncBoxFuture<'_, CollectorResult<Option<Vec<u8>>>>;
}

/// Transport abstraction over Cloudflare Worker HTTP & SSE endpoints.
pub trait SyncHttpTransport: Send + Sync {
    fn post_json<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
        json_body: &'a str,
    ) -> SyncBoxFuture<'a, CollectorResult<(u16, String)>>;

    fn get_with_bearer<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> SyncBoxFuture<'a, CollectorResult<(u16, String)>>;

    fn open_sse_stream<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> SyncBoxFuture<'a, CollectorResult<(u16, Box<dyn SseByteStream>)>>;
}

struct ReqwestSseStream {
    response: reqwest::Response,
}

impl SseByteStream for ReqwestSseStream {
    fn next_chunk(&mut self) -> SyncBoxFuture<'_, CollectorResult<Option<Vec<u8>>>> {
        Box::pin(async move {
            match self.response.chunk().await {
                Ok(Some(bytes)) => Ok(Some(bytes.to_vec())),
                Ok(None) => Ok(None),
                Err(err) => Err(CollectorError::Http(err)),
            }
        })
    }
}

/// Production HTTP + SSE transport backed by `reqwest::Client`.
#[derive(Clone)]
pub struct ReqwestSyncTransport {
    http_client: reqwest::Client,
    sse_client: reqwest::Client,
}

impl ReqwestSyncTransport {
    pub fn new() -> CollectorResult<Self> {
        let http_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25))
            .user_agent(concat!("TokenVane/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let sse_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("TokenVane-SSE/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http_client,
            sse_client,
        })
    }
}

impl SyncHttpTransport for ReqwestSyncTransport {
    fn post_json<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
        json_body: &'a str,
    ) -> SyncBoxFuture<'a, CollectorResult<(u16, String)>> {
        Box::pin(async move {
            let resp = self
                .http_client
                .post(url)
                .bearer_auth(bearer_token)
                .header("Content-Type", "application/json")
                .body(json_body.to_string())
                .send()
                .await?;
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            Ok((status, text))
        })
    }

    fn get_with_bearer<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> SyncBoxFuture<'a, CollectorResult<(u16, String)>> {
        Box::pin(async move {
            let resp = self
                .http_client
                .get(url)
                .bearer_auth(bearer_token)
                .header("Accept", "application/json")
                .send()
                .await?;
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            Ok((status, text))
        })
    }

    fn open_sse_stream<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> SyncBoxFuture<'a, CollectorResult<(u16, Box<dyn SseByteStream>)>> {
        Box::pin(async move {
            let resp = self
                .sse_client
                .get(url)
                .bearer_auth(bearer_token)
                .header("Accept", "text/event-stream")
                .send()
                .await?;
            let status = resp.status().as_u16();
            Ok((status, Box::new(ReqwestSseStream { response: resp }) as Box<dyn SseByteStream>))
        })
    }
}

/// Persistent + runtime sync status DTO returned to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatusView {
    pub enabled: bool,
    pub worker_url: Option<String>,
    pub e2ee_enabled: bool,
    pub e2ee_kdf_salt_hex: Option<String>,
    pub has_bearer_token: bool,
    pub has_e2ee_passphrase: bool,
    #[serde(default)]
    pub has_cf_api_token: bool,
    pub sse_connected: bool,
    pub last_push_watermark_ms: i64,
    pub last_pull_watermark_ms: i64,
    pub last_sync_at_ms: Option<i64>,
    pub last_sync_error: Option<String>,
    pub updated_at_ms: i64,
}

/// Summary returned after one push+pull synchronization cycle.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRunReport {
    pub pushed_hourly_deltas: usize,
    pub pushed_subscriptions: usize,
    pub pulled_items: usize,
    pub applied_hourly_buckets: usize,
    pub applied_subscriptions: usize,
    pub server_seq_ms: Option<i64>,
    pub synced_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncPushRequestV1 {
    mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    envelope: Option<SyncEnvelopeV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encrypted_envelope: Option<EncryptedSyncEnvelopeV1>,
}

#[derive(Debug, Clone, Deserialize)]
struct SyncPushResponseV1 {
    ok: bool,
    server_seq_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct SyncPullItemV1 {
    mode: String,
    #[allow(dead_code)]
    seq_ms: i64,
    #[allow(dead_code)]
    device_id: String,
    envelope: Option<SyncEnvelopeV1>,
    encrypted_envelope: Option<EncryptedSyncEnvelopeV1>,
}

#[derive(Debug, Clone, Deserialize)]
struct SyncPullResponseV1 {
    ok: bool,
    server_seq_ms: i64,
    next_since_ms: i64,
    has_more: bool,
    items: Vec<SyncPullItemV1>,
}

#[derive(Debug, Clone, Deserialize)]
struct SseUpdateEvent {
    #[allow(dead_code)]
    seq_ms: i64,
    device_id: String,
}

/// Store non-empty cloud sync secrets inside the OS native credential manager.
pub fn set_sync_credentials(
    credentials: &dyn CredentialManager,
    bearer_token: Option<&str>,
    e2ee_passphrase: Option<&str>,
) -> CollectorResult<()> {
    if let Some(tok) = bearer_token.map(str::trim).filter(|s| !s.is_empty()) {
        credentials.set_api_key(SYNC_BEARER_TOKEN_KEY, tok)?;
    }
    if let Some(pass) = e2ee_passphrase.map(str::trim).filter(|s| !s.is_empty()) {
        credentials.set_api_key(SYNC_E2EE_PASSPHRASE_KEY, pass)?;
    }
    Ok(())
}

/// Remove cloud sync Bearer token, E2EE passphrase, and Cloudflare API token from the OS credential store.
pub fn clear_sync_credentials(credentials: &dyn CredentialManager) -> CollectorResult<()> {
    credentials.delete_api_key(SYNC_BEARER_TOKEN_KEY)?;
    credentials.delete_api_key(SYNC_E2EE_PASSPHRASE_KEY)?;
    credentials.delete_api_key(SYNC_CF_API_TOKEN_KEY)?;
    Ok(())
}

/// Check whether cloud sync Bearer token and E2EE passphrase exist in the OS credential store.
pub fn has_sync_credentials(credentials: &dyn CredentialManager) -> CollectorResult<(bool, bool)> {
    Ok((
        credentials.has_api_key(SYNC_BEARER_TOKEN_KEY)?,
        credentials.has_api_key(SYNC_E2EE_PASSPHRASE_KEY)?,
    ))
}

pub fn normalize_worker_url(raw: Option<&str>) -> CollectorResult<String> {
    let trimmed = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CollectorError::Compliance("Cloud sync Worker URL is not configured".to_string())
        })?;
    let parsed = reqwest::Url::parse(trimmed).map_err(|e| {
        CollectorError::Compliance(format!("Invalid Cloud sync Worker URL '{trimmed}': {e}"))
    })?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(CollectorError::Compliance(
            "Cloud sync Worker URL must start with http:// or https://".to_string(),
        ));
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

/// Build a compact `tokenvane://pair?...` deep-link URI for mobile QR/clipboard onboarding.
pub fn build_sync_pairing_uri(
    worker_url: &str,
    bearer_token: &str,
    e2ee_enabled: bool,
    e2ee_passphrase: Option<&str>,
) -> CollectorResult<String> {
    let clean_url = normalize_worker_url(Some(worker_url))?;
    let clean_token = bearer_token.trim();
    if clean_token.is_empty() {
        return Err(CollectorError::Compliance(
            "Cloud sync Bearer token is required to build pairing URI".to_string(),
        ));
    }
    let mut uri = reqwest::Url::parse("tokenvane://pair")
        .map_err(|e| CollectorError::Compliance(format!("Failed to initialize pairing URI: {e}")))?;
    {
        let mut qp = uri.query_pairs_mut();
        qp.append_pair("v", "1");
        qp.append_pair("url", &clean_url);
        qp.append_pair("token", clean_token);
        qp.append_pair("e2ee", if e2ee_enabled { "1" } else { "0" });
        if e2ee_enabled {
            if let Some(pass) = e2ee_passphrase.map(str::trim).filter(|s| !s.is_empty()) {
                qp.append_pair("key", pass);
            }
        }
    }
    Ok(uri.to_string())
}

fn extract_http_error_message(body_text: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_text) {
        if let Some(msg) = val.get("message").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
        if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
            return err.to_string();
        }
    }
    let trimmed = body_text.trim();
    if trimmed.is_empty() {
        "empty error response".to_string()
    } else {
        trimmed.chars().take(200).collect()
    }
}

/// Execute one synchronous push+pull cycle using the default production reqwest transport.
pub async fn run_sync_cycle_once(
    storage: &Storage,
    credentials: &dyn CredentialManager,
    force_push: bool,
) -> CollectorResult<SyncRunReport> {
    let transport = ReqwestSyncTransport::new()?;
    run_sync_cycle_with_transport(storage, credentials, &transport, force_push).await
}

/// Execute one synchronous push+pull cycle using a pluggable transport (live reqwest or mock).
pub async fn run_sync_cycle_with_transport(
    storage: &Storage,
    credentials: &dyn CredentialManager,
    transport: &dyn SyncHttpTransport,
    force_push: bool,
) -> CollectorResult<SyncRunReport> {
    let res = try_run_sync_cycle_with_transport(storage, credentials, transport, force_push).await;
    if let Err(ref err) = res {
        let now_ms = Utc::now().timestamp_millis();
        let _ = storage.record_sync_error(Some(&err.to_string()), now_ms);
    }
    res
}

async fn try_run_sync_cycle_with_transport(
    storage: &Storage,
    credentials: &dyn CredentialManager,
    transport: &dyn SyncHttpTransport,
    force_push: bool,
) -> CollectorResult<SyncRunReport> {
    let cfg = storage.get_sync_config_state()?;
    let worker_url = normalize_worker_url(cfg.worker_url.as_deref())?;
    let bearer_token = credentials
        .get_api_key(SYNC_BEARER_TOKEN_KEY)?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CollectorError::Compliance(
                "Cloud sync Bearer token is not configured in OS keyring".to_string(),
            )
        })?;

    let local_device = storage.local_device()?;
    let now_ms = Utc::now().timestamp_millis();

    let (e2ee_passphrase, e2ee_salt_hex) = if cfg.e2ee_enabled {
        let pass = credentials
            .get_api_key(SYNC_E2EE_PASSPHRASE_KEY)?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CollectorError::Compliance(
                    "E2EE is enabled but sync passphrase is missing in OS keyring".to_string(),
                )
            })?;
        let salt = match cfg
            .e2ee_kdf_salt_hex
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(existing) => existing.to_string(),
            None => {
                let generated = generate_kdf_salt_hex()?;
                storage.update_sync_config_state(
                    cfg.enabled,
                    Some(&worker_url),
                    true,
                    Some(&generated),
                )?;
                generated
            }
        };
        (Some(pass), Some(salt))
    } else {
        (None, None)
    };

    let mut report = SyncRunReport {
        synced_at_ms: now_ms,
        ..SyncRunReport::default()
    };

    // 1. Incremental Push Phase
    let envelope = storage.build_sync_envelope_v1(cfg.last_push_watermark_ms, now_ms)?;
    let delta_count = envelope.hourly_deltas.len();
    let sub_count = envelope.subscription_snapshots.len();
    let should_push = force_push
        || cfg.last_push_watermark_ms == 0
        || delta_count > 0
        || sub_count > 0;

    if should_push {
        let push_body = if let (Some(pass), Some(salt)) =
            (e2ee_passphrase.as_deref(), e2ee_salt_hex.as_deref())
        {
            let encrypted = encrypt_sync_envelope_v1(&envelope, pass, Some(salt), now_ms)?;
            SyncPushRequestV1 {
                mode: "e2ee".to_string(),
                envelope: None,
                encrypted_envelope: Some(encrypted),
            }
        } else {
            SyncPushRequestV1 {
                mode: "plaintext".to_string(),
                envelope: Some(envelope),
                encrypted_envelope: None,
            }
        };

        let json_body = serde_json::to_string(&push_body)?;
        let (status, text) = transport
            .post_json(&format!("{worker_url}/v1/sync/push"), &bearer_token, &json_body)
            .await?;

        if !(200..300).contains(&status) {
            return Err(CollectorError::Compliance(format!(
                "Worker sync push rejected (HTTP {status}): {}",
                extract_http_error_message(&text)
            )));
        }

        let parsed: SyncPushResponseV1 = serde_json::from_str(&text)?;
        if !parsed.ok {
            return Err(CollectorError::Compliance(
                "Worker sync push returned ok=false".to_string(),
            ));
        }
        report.server_seq_ms = Some(parsed.server_seq_ms);
        report.pushed_hourly_deltas = delta_count;
        report.pushed_subscriptions = sub_count;
        storage.record_sync_push_success(now_ms, now_ms)?;
    }

    // 2. Incremental Pull Phase
    let pull_summary = pull_remote_delta_pages(
        storage,
        credentials,
        transport,
        &worker_url,
        &bearer_token,
        &local_device.device_id,
        cfg.last_pull_watermark_ms,
        e2ee_passphrase.as_deref(),
    )
    .await?;

    report.pulled_items = pull_summary.pulled_items;
    report.applied_hourly_buckets = pull_summary.applied_hourly_buckets;
    report.applied_subscriptions = pull_summary.applied_subscriptions;
    if pull_summary.server_seq_ms.is_some() {
        report.server_seq_ms = pull_summary.server_seq_ms;
    }

    Ok(report)
}

#[derive(Debug, Default)]
struct PullSummary {
    pulled_items: usize,
    applied_hourly_buckets: usize,
    applied_subscriptions: usize,
    server_seq_ms: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
async fn pull_remote_delta_pages(
    storage: &Storage,
    credentials: &dyn CredentialManager,
    transport: &dyn SyncHttpTransport,
    worker_url: &str,
    bearer_token: &str,
    local_device_id: &str,
    initial_since_ms: i64,
    cached_passphrase: Option<&str>,
) -> CollectorResult<PullSummary> {
    let mut summary = PullSummary::default();
    let mut since = initial_since_ms.max(0);
    let mut loaded_passphrase = cached_passphrase.map(ToString::to_string);

    for _ in 0..MAX_PULL_PAGES {
        let mut pull_url = reqwest::Url::parse(&format!("{worker_url}/v1/sync/pull"))
            .map_err(|e| CollectorError::Compliance(format!("Invalid pull URL: {e}")))?;
        pull_url
            .query_pairs_mut()
            .append_pair("since", &since.to_string())
            .append_pair("exclude_device", local_device_id)
            .append_pair("limit", "100");

        let (status, text) = transport
            .get_with_bearer(pull_url.as_str(), bearer_token)
            .await?;

        if !(200..300).contains(&status) {
            return Err(CollectorError::Compliance(format!(
                "Worker sync pull failed (HTTP {status}): {}",
                extract_http_error_message(&text)
            )));
        }

        let parsed: SyncPullResponseV1 = serde_json::from_str(&text)?;
        if !parsed.ok {
            return Err(CollectorError::Compliance(
                "Worker sync pull returned ok=false".to_string(),
            ));
        }

        let now_ms = Utc::now().timestamp_millis();
        summary.server_seq_ms = Some(parsed.server_seq_ms);

        for item in parsed.items {
            summary.pulled_items += 1;
            let envelope = match item.mode.as_str() {
                "plaintext" => item.envelope.ok_or_else(|| {
                    CollectorError::Compliance(
                        "Worker returned plaintext pull item without envelope".to_string(),
                    )
                })?,
                "e2ee" => {
                    let encrypted = item.encrypted_envelope.ok_or_else(|| {
                        CollectorError::Compliance(
                            "Worker returned e2ee pull item without encrypted_envelope".to_string(),
                        )
                    })?;
                    let pass = match loaded_passphrase.as_deref() {
                        Some(p) => p.to_string(),
                        None => {
                            let from_creds = credentials
                                .get_api_key(SYNC_E2EE_PASSPHRASE_KEY)?
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .ok_or_else(|| {
                                    CollectorError::Compliance(
                                        "Received E2EE envelope but local passphrase is not configured"
                                            .to_string(),
                                    )
                                })?;
                            loaded_passphrase = Some(from_creds.clone());
                            from_creds
                        }
                    };
                    decrypt_sync_envelope_v1(&encrypted, &pass)?
                }
                other => {
                    return Err(CollectorError::Compliance(format!(
                        "Unsupported pull item mode '{other}'"
                    )));
                }
            };

            let applied = storage.apply_remote_sync_envelope_v1(&envelope, now_ms)?;
            summary.applied_hourly_buckets += applied.hourly_buckets_applied;
            summary.applied_subscriptions += applied.subscription_snapshots_applied;
        }

        storage.record_sync_pull_success(parsed.next_since_ms, now_ms)?;
        since = parsed.next_since_ms;

        if !parsed.has_more {
            break;
        }
    }

    Ok(summary)
}

/// Manages manual cloud sync, 3-second debounced local push, and real-time SSE reconnects.
#[derive(Clone)]
pub struct SyncClientHandle {
    storage: Storage,
    credentials: Arc<dyn CredentialManager>,
    transport: Arc<dyn SyncHttpTransport>,
    sse_connected: Arc<AtomicBool>,
    sync_lock: Arc<Mutex<()>>,
    debounce_notify: Arc<Notify>,
    reconfig_tx: watch::Sender<u64>,
    shutdown_tx: watch::Sender<bool>,
}

impl SyncClientHandle {
    pub fn new(storage: Storage, credentials: Arc<dyn CredentialManager>) -> CollectorResult<Self> {
        let transport: Arc<dyn SyncHttpTransport> = Arc::new(ReqwestSyncTransport::new()?);
        Ok(Self::with_transport(storage, credentials, transport))
    }

    pub fn with_transport(
        storage: Storage,
        credentials: Arc<dyn CredentialManager>,
        transport: Arc<dyn SyncHttpTransport>,
    ) -> Self {
        let (reconfig_tx, _) = watch::channel(0u64);
        let (shutdown_tx, _) = watch::channel(false);

        Self {
            storage,
            credentials,
            transport,
            sse_connected: Arc::new(AtomicBool::new(false)),
            sync_lock: Arc::new(Mutex::new(())),
            debounce_notify: Arc::new(Notify::new()),
            reconfig_tx,
            shutdown_tx,
        }
    }

    pub fn is_sse_connected(&self) -> bool {
        self.sse_connected.load(Ordering::Relaxed)
    }

    pub fn status_view(&self) -> CollectorResult<SyncStatusView> {
        read_sync_status_view(
            &self.storage,
            self.credentials.as_ref(),
            self.is_sse_connected(),
        )
        .map_err(CollectorError::from)
    }

    pub async fn trigger_now(&self) -> CollectorResult<SyncRunReport> {
        let _guard = self.sync_lock.lock().await;
        run_sync_cycle_with_transport(
            &self.storage,
            self.credentials.as_ref(),
            self.transport.as_ref(),
            true,
        )
        .await
    }

    pub fn notify_local_change(&self) {
        self.debounce_notify.notify_one();
    }

    pub fn reconfigure(&self) {
        let next = *self.reconfig_tx.borrow() + 1;
        let _ = self.reconfig_tx.send(next);
        self.debounce_notify.notify_one();
    }

    pub fn shutdown(&self) {
        self.sse_connected.store(false, Ordering::Relaxed);
        let _ = self.shutdown_tx.send(true);
    }

    pub fn start_background<F>(&self, on_remote_applied: F)
    where
        F: Fn(&SyncRunReport) + Send + Sync + 'static,
    {
        let callback = Arc::new(on_remote_applied);
        self.spawn_debounced_pusher(Arc::clone(&callback));
        self.spawn_sse_listener(callback);
    }

    fn spawn_debounced_pusher<F>(&self, on_remote_applied: Arc<F>)
    where
        F: Fn(&SyncRunReport) + Send + Sync + 'static,
    {
        let storage = self.storage.clone();
        let credentials = Arc::clone(&self.credentials);
        let transport = Arc::clone(&self.transport);
        let sync_lock = Arc::clone(&self.sync_lock);
        let debounce_notify = Arc::clone(&self.debounce_notify);
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = debounce_notify.notified() => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            return;
                        }
                    }
                }

                // Coalesce local burst writes inside a 3s debounce window
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(PUSH_DEBOUNCE_SECS)) => {
                            break;
                        }
                        _ = debounce_notify.notified() => {
                            continue;
                        }
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                return;
                            }
                        }
                    }
                }

                if matches!(storage.get_sync_config_state(), Ok(cfg) if cfg.enabled) {
                    let _guard = sync_lock.lock().await;
                    if let Ok(report) = run_sync_cycle_with_transport(
                        &storage,
                        credentials.as_ref(),
                        transport.as_ref(),
                        false,
                    )
                    .await
                    {
                        if report.applied_hourly_buckets > 0 || report.applied_subscriptions > 0 {
                            on_remote_applied(&report);
                        }
                    }
                }
            }
        });
    }

    fn spawn_sse_listener<F>(&self, on_remote_applied: Arc<F>)
    where
        F: Fn(&SyncRunReport) + Send + Sync + 'static,
    {
        let storage = self.storage.clone();
        let credentials = Arc::clone(&self.credentials);
        let transport = Arc::clone(&self.transport);
        let sse_connected = Arc::clone(&self.sse_connected);
        let sync_lock = Arc::clone(&self.sync_lock);
        let mut reconfig_rx = self.reconfig_tx.subscribe();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            let mut backoff_idx: usize = 0;

            loop {
                if *shutdown_rx.borrow() {
                    sse_connected.store(false, Ordering::Relaxed);
                    return;
                }

                let config_ready = (|| -> CollectorResult<Option<(String, String, String)>> {
                    let cfg = storage.get_sync_config_state()?;
                    if !cfg.enabled {
                        return Ok(None);
                    }
                    let worker_url = normalize_worker_url(cfg.worker_url.as_deref())?;
                    let token = credentials
                        .get_api_key(SYNC_BEARER_TOKEN_KEY)?
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    let Some(bearer) = token else {
                        return Ok(None);
                    };
                    let dev = storage.local_device()?;
                    Ok(Some((worker_url, bearer, dev.device_id)))
                })();

                let Ok(Some((worker_url, bearer, local_device_id))) = config_ready else {
                    sse_connected.store(false, Ordering::Relaxed);
                    tokio::select! {
                        _ = reconfig_rx.changed() => {}
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                return;
                            }
                        }
                    }
                    continue;
                };

                // Initial catch-up cycle upon connecting / reconfiguring
                {
                    let _guard = sync_lock.lock().await;
                    if let Ok(report) = run_sync_cycle_with_transport(
                        &storage,
                        credentials.as_ref(),
                        transport.as_ref(),
                        false,
                    )
                    .await
                    {
                        if report.applied_hourly_buckets > 0 || report.applied_subscriptions > 0 {
                            on_remote_applied(&report);
                        }
                    }
                }

                let sse_url = format!("{worker_url}/v1/sync/events");
                let connect_res = tokio::select! {
                    res = transport.open_sse_stream(&sse_url, &bearer) => Some(res),
                    _ = reconfig_rx.changed() => None,
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            sse_connected.store(false, Ordering::Relaxed);
                            return;
                        }
                        None
                    }
                };

                let Some(stream_result) = connect_res else {
                    sse_connected.store(false, Ordering::Relaxed);
                    backoff_idx = 0;
                    continue;
                };

                let mut reconfigured_mid_stream = false;
                match stream_result {
                    Ok((status, mut stream)) if (200..300).contains(&status) => {
                        sse_connected.store(true, Ordering::Relaxed);
                        backoff_idx = 0;

                        let mut line_buf = String::new();
                        let mut current_event = String::new();
                        let mut current_data = String::new();

                        loop {
                            let next_chunk = tokio::select! {
                                chunk = tokio::time::timeout(
                                    Duration::from_secs(SSE_IDLE_TIMEOUT_SECS),
                                    stream.next_chunk()
                                ) => Some(chunk),
                                _ = reconfig_rx.changed() => {
                                    reconfigured_mid_stream = true;
                                    None
                                }
                                _ = shutdown_rx.changed() => {
                                    if *shutdown_rx.borrow() {
                                        sse_connected.store(false, Ordering::Relaxed);
                                        return;
                                    }
                                    None
                                }
                            };

                            let Some(Ok(Ok(Some(bytes)))) = next_chunk else {
                                break;
                            };

                            line_buf.push_str(&String::from_utf8_lossy(&bytes));
                            while let Some(pos) = line_buf.find('\n') {
                                let raw_line = line_buf[..pos].trim_end_matches('\r').to_string();
                                line_buf.drain(..=pos);

                                if raw_line.is_empty() {
                                    if should_pull_for_sse_event(
                                        &current_event,
                                        &current_data,
                                        &local_device_id,
                                    ) {
                                        let _guard = sync_lock.lock().await;
                                        if let Ok(report) = run_sync_cycle_with_transport(
                                            &storage,
                                            credentials.as_ref(),
                                            transport.as_ref(),
                                            false,
                                        )
                                        .await
                                        {
                                            if report.applied_hourly_buckets > 0
                                                || report.applied_subscriptions > 0
                                            {
                                                on_remote_applied(&report);
                                            }
                                        }
                                    }
                                    current_event.clear();
                                    current_data.clear();
                                } else if raw_line.starts_with(':') {
                                    continue;
                                } else if let Some(ev) = raw_line.strip_prefix("event:") {
                                    current_event = ev.trim().to_string();
                                } else if let Some(data) = raw_line.strip_prefix("data:") {
                                    if !current_data.is_empty() {
                                        current_data.push('\n');
                                    }
                                    current_data.push_str(data.trim());
                                }
                            }
                        }
                    }
                    Ok((status, _)) => {
                        let _ = storage.record_sync_error(
                            Some(&format!("SSE stream rejected (HTTP {status})")),
                            Utc::now().timestamp_millis(),
                        );
                    }
                    Err(err) => {
                        let _ = storage.record_sync_error(
                            Some(&format!("SSE connection failed: {err}")),
                            Utc::now().timestamp_millis(),
                        );
                    }
                }

                sse_connected.store(false, Ordering::Relaxed);
                if reconfigured_mid_stream {
                    backoff_idx = 0;
                    continue;
                }

                let wait_secs = SSE_BACKOFF_SECS[backoff_idx.min(SSE_BACKOFF_SECS.len() - 1)];
                backoff_idx = (backoff_idx + 1).min(SSE_BACKOFF_SECS.len() - 1);

                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(wait_secs)) => {}
                    _ = reconfig_rx.changed() => {
                        backoff_idx = 0;
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            return;
                        }
                    }
                }
            }
        });
    }
}

fn should_pull_for_sse_event(event_name: &str, data: &str, local_device_id: &str) -> bool {
    if data.is_empty() || (!event_name.is_empty() && event_name != "sync_update") {
        return false;
    }
    match serde_json::from_str::<SseUpdateEvent>(data) {
        Ok(ev) => ev.device_id != local_device_id,
        Err(_) => true,
    }
}

/// Read-only helper returning current sync status without constructing a background handle.
pub fn read_sync_status_view(
    storage: &Storage,
    credentials: &dyn CredentialManager,
    sse_connected: bool,
) -> CoreResult<SyncStatusView> {
    let cfg = storage.get_sync_config_state()?;
    let (has_bearer_token, has_e2ee_passphrase) =
        has_sync_credentials(credentials).unwrap_or((false, false));
    let has_cf_api_token = credentials
        .has_api_key(SYNC_CF_API_TOKEN_KEY)
        .unwrap_or(false);
    Ok(SyncStatusView {
        enabled: cfg.enabled,
        worker_url: cfg.worker_url,
        e2ee_enabled: cfg.e2ee_enabled,
        e2ee_kdf_salt_hex: cfg.e2ee_kdf_salt_hex,
        has_bearer_token,
        has_e2ee_passphrase,
        has_cf_api_token,
        sse_connected,
        last_push_watermark_ms: cfg.last_push_watermark_ms,
        last_pull_watermark_ms: cfg.last_pull_watermark_ms,
        last_sync_at_ms: cfg.last_sync_at_ms,
        last_sync_error: cfg.last_sync_error,
        updated_at_ms: cfg.updated_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_encoded_pairing_uri_with_optional_e2ee_passphrase() {
        let uri = build_sync_pairing_uri(
            "https://sync.example.workers.dev/",
            "tok+abc/123=",
            true,
            Some("pass word&1"),
        )
        .expect("pairing uri");
        assert!(uri.starts_with("tokenvane://pair?"));
        let parsed = reqwest::Url::parse(&uri).expect("valid uri");
        let pairs: std::collections::HashMap<String, String> =
            parsed.query_pairs().into_owned().collect();
        assert_eq!(pairs.get("v").map(String::as_str), Some("1"));
        assert_eq!(
            pairs.get("url").map(String::as_str),
            Some("https://sync.example.workers.dev")
        );
        assert_eq!(pairs.get("token").map(String::as_str), Some("tok+abc/123="));
        assert_eq!(pairs.get("e2ee").map(String::as_str), Some("1"));
        assert_eq!(pairs.get("key").map(String::as_str), Some("pass word&1"));

        let no_key_uri = build_sync_pairing_uri(
            "https://sync.example.workers.dev",
            "tok",
            false,
            Some("ignored-when-plaintext"),
        )
        .expect("plaintext uri");
        let parsed_plain = reqwest::Url::parse(&no_key_uri).expect("valid plain uri");
        let plain_pairs: std::collections::HashMap<String, String> =
            parsed_plain.query_pairs().into_owned().collect();
        assert_eq!(plain_pairs.get("e2ee").map(String::as_str), Some("0"));
        assert!(!plain_pairs.contains_key("key"));
    }
}
