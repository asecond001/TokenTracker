use serde::{Deserialize, Serialize};

/// Desensitized hourly delta row safe for Phase 2 Cloudflare Worker synchronization.
/// Strictly excludes session IDs, prompts, file paths, project paths, and credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncHourlyDelta {
    pub provider_id: String,
    pub model_id: String,
    pub bucket_start_ms: i64,
    pub request_count: i64,
    pub observable_step_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub estimated_cost_micro_usd: i64,
    pub cache_saved_micro_usd: i64,
    pub total_duration_ms: i64,
    pub tps_sample_count: i64,
    pub tps_sum: f64,
    pub max_output_tps: f64,
    pub updated_at_ms: i64,
}

/// Desensitized subscription window snapshot for Phase 2 multi-device sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncSubscriptionSnapshot {
    pub plan_id: String,
    pub provider_id: String,
    pub plan_name: String,
    pub window_id: String,
    pub window_label: String,
    pub used_percent: f64,
    pub resets_at_ms: Option<i64>,
    pub source: String,
    #[serde(default)]
    pub monthly_fee_micro_usd: i64,
}

/// Desensitized pay-as-you-go account balance & spend snapshot for multi-device sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPaygSnapshot {
    pub account_id: String,
    pub provider_id: String,
    pub account_name: String,
    pub enabled: bool,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub total_balance_micro: Option<i64>,
    #[serde(default)]
    pub granted_balance_micro: Option<i64>,
    #[serde(default)]
    pub topped_up_balance_micro: Option<i64>,
    #[serde(default)]
    pub remaining_manual_balance_micro_usd: Option<i64>,
    #[serde(default)]
    pub monthly_budget_micro_usd: Option<i64>,
    #[serde(default)]
    pub today_cost_micro_usd: i64,
    #[serde(default)]
    pub month_cost_micro_usd: i64,
    #[serde(default)]
    pub fetched_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

/// SyncEnvelope v1 contract defined upfront in Phase 1 so Phase 2 Cloudflare Worker
/// integration requires zero schema migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncEnvelopeV1 {
    pub schema_version: u32,
    pub device_id: String,
    pub device_display_name: String,
    pub generated_at_ms: i64,
    pub since_watermark_ms: i64,
    pub hourly_deltas: Vec<SyncHourlyDelta>,
    pub subscription_snapshots: Vec<SyncSubscriptionSnapshot>,
    #[serde(default)]
    pub payg_snapshots: Vec<SyncPaygSnapshot>,
}

/// Persistent cloud synchronization configuration and watermarks (`sync_state` row `id = 1`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConfigState {
    pub enabled: bool,
    pub worker_url: Option<String>,
    pub e2ee_enabled: bool,
    pub e2ee_kdf_salt_hex: Option<String>,
    pub last_push_watermark_ms: i64,
    pub last_pull_watermark_ms: i64,
    pub last_sync_at_ms: Option<i64>,
    pub last_sync_error: Option<String>,
    pub updated_at_ms: i64,
}

/// Remote device subscription quota status synced via `SyncEnvelopeV1`.
/// Isolated from local `subscription_plans` so remote snapshots stay read-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSubscriptionSnapshot {
    pub device_id: String,
    pub device_display_name: String,
    pub plan_id: String,
    pub window_id: String,
    pub provider_id: String,
    pub plan_name: String,
    pub window_label: String,
    pub used_percent: f64,
    pub resets_at_ms: Option<i64>,
    pub source: String,
    #[serde(default)]
    pub monthly_fee_micro_usd: i64,
    pub updated_at_ms: i64,
}

/// Remote device pay-as-you-go account balance snapshot synced via `SyncEnvelopeV1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePaygSnapshot {
    pub device_id: String,
    pub device_display_name: String,
    pub account_id: String,
    pub provider_id: String,
    pub account_name: String,
    pub enabled: bool,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub total_balance_micro: Option<i64>,
    #[serde(default)]
    pub granted_balance_micro: Option<i64>,
    #[serde(default)]
    pub topped_up_balance_micro: Option<i64>,
    #[serde(default)]
    pub remaining_manual_balance_micro_usd: Option<i64>,
    #[serde(default)]
    pub monthly_budget_micro_usd: Option<i64>,
    #[serde(default)]
    pub today_cost_micro_usd: i64,
    #[serde(default)]
    pub month_cost_micro_usd: i64,
    #[serde(default)]
    pub fetched_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

/// Optional E2EE wrapper for `SyncEnvelopeV1` using PBKDF2-HMAC-SHA256 + AES-256-GCM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedSyncEnvelopeV1 {
    pub schema_version: u32,
    pub device_id: String,
    pub seq_ms: i64,
    pub kdf_salt_hex: String,
    pub nonce_hex: String,
    pub ciphertext_b64: String,
}

/// Outcome summary after applying a pulled `SyncEnvelopeV1` locally.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSyncApplyResult {
    pub device_id: String,
    pub hourly_buckets_applied: usize,
    pub subscription_snapshots_applied: usize,
    #[serde(default)]
    pub payg_snapshots_applied: usize,
}
