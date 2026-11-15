use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Canonical Phase 1 provider identifiers.
pub mod provider_ids {
    pub const CLAUDE_CODE: &str = "claude_code";
    pub const CODEX: &str = "codex";
    pub const ANTIGRAVITY: &str = "antigravity";

    pub const ALL: [&str; 3] = [CLAUDE_CODE, CODEX, ANTIGRAVITY];
}

/// Compliant data acquisition modes permitted by AGENTS.md Section 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionMode {
    LocalLog,
    Otlp,
    OfficialApi,
}

impl CollectionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalLog => "local_log",
            Self::Otlp => "otlp",
            Self::OfficialApi => "official_api",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "otlp" => Self::Otlp,
            "official_api" => Self::OfficialApi,
            _ => Self::LocalLog,
        }
    }
}

/// Display currency supported in Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CurrencyCode {
    Usd,
    Cny,
}

impl CurrencyCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Cny => "CNY",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("CNY") {
            Self::Cny
        } else {
            Self::Usd
        }
    }
}

/// Local or remote registered device record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub created_at_ms: i64,
}

/// Provider collector configuration persisted in SQLite.
/// Note: API keys are NEVER stored here; only `has_credential` boolean flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider_id: String,
    pub enabled: bool,
    pub collection_mode: CollectionMode,
    pub custom_log_paths: Vec<String>,
    pub has_credential: bool,
    pub last_sync_at_ms: Option<i64>,
    pub last_error: Option<String>,
    pub updated_at_ms: i64,
}

/// Byte-offset resume cursor for incremental log tailing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestionCursor {
    pub source_path: String,
    pub provider_id: String,
    pub file_identity: String,
    pub byte_offset: u64,
    pub last_line_hash: String,
    pub updated_at_ms: i64,
}

/// Raw parsed event emitted by a compliant collector before/after pricing enrichment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedUsageEvent {
    pub event_id: String,
    pub device_id: String,
    pub provider_id: String,
    pub session_id: String,
    pub project_label: String,
    pub model_id: String,
    pub timestamp_ms: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    /// True when the public log records an observable agent step without public token counters.
    pub observable_only: bool,
    pub duration_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub output_tps: Option<f64>,
    pub estimated_cost_micro_usd: i64,
    pub cache_saved_micro_usd: i64,
}

impl NormalizedUsageEvent {
    pub fn total_tokens(&self) -> i64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_tokens
            + self.cache_write_tokens
            + self.reasoning_tokens
    }

    pub fn effective_output_tokens(&self) -> i64 {
        self.output_tokens + self.reasoning_tokens
    }

    pub fn context_tokens(&self) -> i64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

/// Deterministic SHA-256 event identifier guaranteeing idempotent re-ingestion.
pub fn compute_event_id(
    provider_id: &str,
    source_session_id: &str,
    step_or_msg_id: &str,
    timestamp_ms: i64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider_id.as_bytes());
    hasher.update(b"|");
    hasher.update(source_session_id.as_bytes());
    hasher.update(b"|");
    hasher.update(step_or_msg_id.as_bytes());
    hasher.update(b"|");
    hasher.update(timestamp_ms.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

/// Extract a desensitized leaf folder label without retaining sensitive absolute paths.
pub fn sanitize_project_label(raw_path: Option<&str>) -> String {
    let Some(raw) = raw_path else {
        return "workspace".to_string();
    };
    let trimmed = raw.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return "workspace".to_string();
    }
    let leaf = trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("workspace")
        .trim();
    if leaf.is_empty() {
        "workspace".to_string()
    } else {
        leaf.chars().take(48).collect()
    }
}

/// Pre-aggregated hourly rollup bucket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HourlyRollup {
    pub device_id: String,
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

/// Pre-aggregated daily rollup bucket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyRollup {
    pub device_id: String,
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

/// Model pricing catalog entry priced in micro-USD per 1,000,000 tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPricingEntry {
    pub model_pattern: String,
    pub provider_id: String,
    pub input_per_m_micro_usd: i64,
    pub output_per_m_micro_usd: i64,
    pub cache_read_per_m_micro_usd: i64,
    pub cache_write_per_m_micro_usd: i64,
    pub reasoning_per_m_micro_usd: i64,
    pub is_user_override: bool,
    pub effective_from: String,
}

/// Budget unit used when evaluating a local subscription rolling window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaUnit {
    MicroUsd,
    Tokens,
}

/// Single window rule inside a subscription plan (e.g. 5h rolling or 7d weekly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionWindowRule {
    pub window_id: String,
    pub label: String,
    pub duration_hours: i64,
    pub rolling: bool,
    pub unit: QuotaUnit,
    pub limit_value: i64,
    pub warning_percent: u8,
    pub critical_percent: u8,
}

/// User-configurable subscription plan definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionPlan {
    pub plan_id: String,
    pub provider_id: String,
    pub plan_name: String,
    pub enabled: bool,
    pub monthly_fee_micro_usd: i64,
    pub windows: Vec<SubscriptionWindowRule>,
    #[serde(default)]
    pub is_user_configured: bool,
    pub updated_at_ms: i64,
}

/// Reference template used only to prefill a user-configured subscription form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionPlanTemplate {
    pub template_id: String,
    pub provider_id: String,
    pub plan_name: String,
    pub monthly_fee_micro_usd: i64,
    pub windows: Vec<SubscriptionWindowRule>,
    pub description: String,
}

/// Official rate limit window snapshot parsed from provider local logs (e.g. Codex).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedRateLimitWindow {
    pub provider_id: String,
    pub window_name: String,
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at_ms: Option<i64>,
    pub observed_at_ms: i64,
}

fn default_bg_mode() -> String {
    "none".to_string()
}

fn default_bg_preset_id() -> String {
    "aurora_slate".to_string()
}

fn default_bg_opacity() -> f64 {
    0.18
}

fn default_bg_blur_px() -> i64 {
    16
}

fn default_true() -> bool {
    true
}

/// Persisted desktop/workspace user preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppPreferences {
    pub currency: CurrencyCode,
    pub usd_to_cny_rate: String,
    pub theme: String,
    pub minimize_to_tray: bool,
    pub launch_at_startup: bool,
    pub otlp_enabled: bool,
    pub otlp_port: u16,
    pub device_display_name: String,
    #[serde(default = "default_bg_mode")]
    pub bg_mode: String,
    #[serde(default)]
    pub bg_image_path: Option<String>,
    #[serde(default = "default_bg_preset_id")]
    pub bg_preset_id: String,
    #[serde(default = "default_bg_opacity")]
    pub bg_opacity: f64,
    #[serde(default = "default_bg_blur_px")]
    pub bg_blur_px: i64,
    #[serde(default = "default_true")]
    pub auto_sync_pricing: bool,
    #[serde(default = "default_true")]
    pub exchange_rate_auto: bool,
    #[serde(default)]
    pub last_rate_sync_at_ms: Option<i64>,
    #[serde(default)]
    pub last_pricing_sync_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            currency: CurrencyCode::Usd,
            usd_to_cny_rate: "7.25".to_string(),
            theme: "dark".to_string(),
            minimize_to_tray: true,
            launch_at_startup: false,
            otlp_enabled: false,
            otlp_port: 4318,
            device_display_name: "win-workstation".to_string(),
            bg_mode: default_bg_mode(),
            bg_image_path: None,
            bg_preset_id: default_bg_preset_id(),
            bg_opacity: default_bg_opacity(),
            bg_blur_px: default_bg_blur_px(),
            auto_sync_pricing: true,
            exchange_rate_auto: true,
            last_rate_sync_at_ms: None,
            last_pricing_sync_at_ms: None,
            updated_at_ms: 0,
        }
    }
}

/// Result summary returned after synchronizing remote USD->CNY rate and public model pricing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSyncResult {
    pub rate_updated: bool,
    pub usd_to_cny_rate: String,
    pub pricing_updated: bool,
    pub synced_models_count: usize,
    pub recomputed_events_count: usize,
    pub synced_at_ms: i64,
    pub rate_source: Option<String>,
    pub pricing_source: Option<String>,
    pub warning: Option<String>,
}
