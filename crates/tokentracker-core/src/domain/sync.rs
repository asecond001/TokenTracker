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
}
