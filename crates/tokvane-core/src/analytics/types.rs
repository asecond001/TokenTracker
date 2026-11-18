//! Public analytics query inputs and report DTOs shared with IPC / FFI.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyticsFilter {
    pub start_ms: i64,
    pub end_ms: i64,
    pub provider_id: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    pub tz_offset_minutes: i32,
    #[serde(default)]
    pub all_time: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendGranularity {
    Auto,
    FifteenMinutes,
    Hourly,
    Daily,
    Weekly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StackDimension {
    Provider,
    Model,
    TokenType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrendsQuery {
    pub filter: AnalyticsFilter,
    pub granularity: TrendGranularity,
    pub dimension: StackDimension,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthlyQuery {
    pub year: i32,
    pub month: u32,
    pub provider_id: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    pub tz_offset_minutes: i32,
    /// Optional custom statistics range (inclusive, epoch ms). When both are set
    /// they override the calendar month given by `year` / `month`.
    #[serde(default)]
    pub start_ms: Option<i64>,
    #[serde(default)]
    pub end_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeBucketPoint {
    pub bucket_start_ms: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub cost_micro_usd: i64,
    pub requests: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSummaryRow {
    pub provider_id: String,
    pub enabled: bool,
    pub period_tokens: i64,
    pub period_cost_micro_usd: i64,
    pub today_tokens: i64,
    pub today_cost_micro_usd: i64,
    pub month_tokens: i64,
    pub month_cost_micro_usd: i64,
    pub cache_hit_ratio: f64,
    pub observable_steps: i64,
    pub active_window_label: Option<String>,
    pub active_window_used_percent: Option<f64>,
    pub last_sync_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveSessionSummary {
    pub session_key: String,
    #[serde(default)]
    pub session_title: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    pub project_label: String,
    pub activity_state: String,
    pub started_at_ms: i64,
    pub last_active_at_ms: i64,
    pub request_count: i64,
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub estimated_cost_micro_usd: i64,
    pub latest_output_tps: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveModelSpeed {
    pub provider_id: String,
    pub model_id: String,
    pub output_tps: f64,
    pub activity_state: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageShareItem {
    pub key: String,
    pub label: String,
    pub tokens: i64,
    pub token_share_percent: f64,
    pub cost_micro_usd: i64,
    pub cost_share_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewReport {
    pub total_tokens: i64,
    pub total_cost_micro_usd: i64,
    pub today_tokens: i64,
    pub today_cost_micro_usd: i64,
    #[serde(default)]
    pub today_requests: i64,
    #[serde(default)]
    pub today_cache_hit_ratio: f64,
    #[serde(default)]
    pub today_cache_saved_micro_usd: i64,
    #[serde(default)]
    pub today_tokens_delta_pct: Option<f64>,
    #[serde(default)]
    pub today_cost_delta_pct: Option<f64>,
    #[serde(default)]
    pub today_hourly_pulse: Vec<TimeBucketPoint>,
    pub cache_hit_ratio: f64,
    pub avg_output_tps: f64,
    pub peak_output_tps: f64,
    pub latest_output_tps: Option<f64>,
    #[serde(default)]
    pub live_model_speeds: Vec<LiveModelSpeed>,
    pub total_requests: i64,
    pub observable_steps: i64,
    pub tokens_delta_pct: Option<f64>,
    pub cost_delta_pct: Option<f64>,
    pub cache_delta_pct: Option<f64>,
    pub tps_delta_pct: Option<f64>,
    pub series_granularity: String,
    pub series: Vec<TimeBucketPoint>,
    pub provider_series: Vec<TrendSeries>,
    pub providers: Vec<ProviderSummaryRow>,
    pub provider_shares: Vec<UsageShareItem>,
    pub top_model_shares: Vec<UsageShareItem>,
    pub active_sessions: Vec<ActiveSessionSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendPoint {
    pub bucket_start_ms: i64,
    pub tokens: i64,
    pub cost_micro_usd: i64,
    pub requests: i64,
    #[serde(default)]
    pub cache_hit_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendSeries {
    pub key: String,
    pub label: String,
    pub total_tokens: i64,
    pub total_cost_micro_usd: i64,
    pub points: Vec<TrendPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendComparisonRow {
    pub key: String,
    pub label: String,
    pub current_tokens: i64,
    pub previous_tokens: i64,
    pub delta_pct: Option<f64>,
    pub current_cost_micro_usd: i64,
    pub previous_cost_micro_usd: i64,
    #[serde(default)]
    pub cost_delta_pct: Option<f64>,
    #[serde(default)]
    pub cache_hit_ratio: f64,
    pub requests: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendsReport {
    pub effective_granularity: TrendGranularity,
    pub buckets: Vec<i64>,
    pub series: Vec<TrendSeries>,
    pub comparisons: Vec<TrendComparisonRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonthlyDailyLedgerRow {
    pub date: String,
    pub requests: i64,
    pub observable_steps: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    pub cost_micro_usd: i64,
    pub cache_saved_micro_usd: i64,
    pub top_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeatmapDayCell {
    pub date: String,
    pub total_tokens: i64,
    pub cost_micro_usd: i64,
    pub requests: i64,
    pub top_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonthlyReport {
    pub year: i32,
    pub month: u32,
    pub api_equivalent_cost_micro_usd: i64,
    pub subscription_monthly_fee_micro_usd: i64,
    pub roi_multiplier: Option<f64>,
    pub month_total_tokens: i64,
    pub month_total_requests: i64,
    pub month_cache_saved_micro_usd: i64,
    #[serde(default)]
    pub provider_shares: Vec<UsageShareItem>,
    #[serde(default)]
    pub model_shares: Vec<UsageShareItem>,
    pub daily_ledger: Vec<MonthlyDailyLedgerRow>,
    pub heatmap_365d: Vec<HeatmapDayCell>,
    /// Resolved statistics range (inclusive, epoch ms).
    #[serde(default)]
    pub range_start_ms: i64,
    #[serde(default)]
    pub range_end_ms: i64,
    /// Subscription fees prorated to the statistics range (equals the monthly
    /// fee for a calendar-month query). `roi_multiplier` is computed from this.
    #[serde(default)]
    pub subscription_period_fee_micro_usd: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelStatRow {
    pub model_id: String,
    pub provider_id: String,
    pub requests: i64,
    pub observable_steps: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    pub cache_saved_micro_usd: i64,
    pub estimated_cost_micro_usd: i64,
    pub cost_share_ratio: f64,
    pub avg_output_tps: f64,
    pub effective_pricing: Option<ModelPricingEntry>,
    /// API list-price cost with prompt caching ignored: every prompt token
    /// (uncached input, cache reads and cache writes) billed at the input rate.
    /// `0` when no pricing rule matches.
    #[serde(default)]
    pub list_cost_micro_usd: i64,
    /// Share of the provider's subscription fee (prorated to the range, capped
    /// at now) allocated by API-equivalent cost within that provider.
    /// `None` when the provider has no paid subscription plan configured.
    #[serde(default)]
    pub subscription_cost_micro_usd: Option<i64>,
}

/// Paid subscription fee for one provider, prorated to the model-stats range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSubscriptionAllocation {
    pub provider_id: String,
    pub plan_names: Vec<String>,
    pub monthly_fee_micro_usd: i64,
    pub period_fee_micro_usd: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelStatsReport {
    pub rows: Vec<ModelStatRow>,
    pub total_cost_micro_usd: i64,
    pub total_cache_saved_micro_usd: i64,
    pub catalog: Vec<ModelPricingEntry>,
    #[serde(default)]
    pub subscription_allocations: Vec<ProviderSubscriptionAllocation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSpeedStats {
    pub model_id: String,
    pub provider_id: String,
    pub sample_count: i64,
    pub mean_tps: f64,
    pub p50_tps: f64,
    pub p95_tps: f64,
    pub max_tps: f64,
    pub min_tps: f64,
    pub latest_tps: f64,
    pub avg_duration_ms: f64,
    pub avg_ttft_ms: Option<f64>,
    /// Measurement caliber of `*_tps` (decode window vs end-to-end), derived from the provider.
    #[serde(default)]
    pub speed_basis: SpeedBasis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSpeedStats {
    pub provider_id: String,
    pub sample_count: i64,
    pub mean_tps: f64,
    pub p50_tps: f64,
    pub p95_tps: f64,
    pub max_tps: f64,
    pub min_tps: f64,
    pub avg_duration_ms: f64,
    pub avg_ttft_ms: Option<f64>,
    /// Measurement caliber of `*_tps` (decode window vs end-to-end), derived from the provider.
    #[serde(default)]
    pub speed_basis: SpeedBasis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeedLatencyReport {
    pub providers: Vec<ProviderSpeedStats>,
    pub models: Vec<ModelSpeedStats>,
    #[serde(default)]
    pub from_hourly_rollups: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraySummary {
    pub today_tokens: i64,
    pub today_cost_micro_usd: i64,
    pub today_requests: i64,
    pub latest_output_tps: Option<f64>,
    #[serde(default)]
    pub live_model_speeds: Vec<LiveModelSpeed>,
    pub subscriptions: Vec<SubscriptionPlanStatus>,
    pub provider_shares: Vec<UsageShareItem>,
    pub active_sessions: Vec<ActiveSessionSummary>,
    pub currency: CurrencyCode,
    pub usd_to_cny_rate: String,
    pub active_providers_count: usize,
    pub total_ingested_events: i64,
}
