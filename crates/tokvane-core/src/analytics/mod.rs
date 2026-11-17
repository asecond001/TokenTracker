use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{Datelike, FixedOffset, Local, NaiveDate, TimeZone, Utc};
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{
    compute_ewma_tps, compute_iqr_upper_fence, provider_ids, sanitize_session_title, CurrencyCode,
    ModelPricingEntry, ObservedRateLimitWindow, SubscriptionPlan, MAX_VALID_OUTPUT_TPS,
    MIN_VALID_TPS_DURATION_MS, MIN_VALID_TPS_OUTPUT_TOKENS,
};
use crate::error::{CoreError, CoreResult};
use crate::pricing::resolve_pricing;
use crate::storage::repository::{
    load_observed_rate_limits, load_preferences, load_pricing_catalog,
    load_subscription_catalog_templates, load_subscription_plans, load_window_samples_map, DAY_MS,
    HOUR_MS,
};
use crate::storage::Storage;
use crate::subscription::{
    evaluate_subscription_plan, evaluate_subscription_templates_from_list, SubscriptionPlanStatus,
};

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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelStatsReport {
    pub rows: Vec<ModelStatRow>,
    pub total_cost_micro_usd: i64,
    pub total_cache_saved_micro_usd: i64,
    pub catalog: Vec<ModelPricingEntry>,
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

#[derive(Default)]
struct PeriodTotals {
    requests: i64,
    observable_steps: i64,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
    cost_micro_usd: i64,
    cache_saved_micro_usd: i64,
    tps_count: i64,
    tps_sum: f64,
    max_tps: f64,
}

impl PeriodTotals {
    fn total_tokens(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_write + self.reasoning
    }

    fn cache_hit_ratio(&self) -> f64 {
        let denom = self.input + self.cache_read + self.cache_write;
        if denom <= 0 {
            0.0
        } else {
            self.cache_read as f64 / denom as f64
        }
    }

    fn avg_tps(&self) -> f64 {
        if self.tps_count <= 0 {
            0.0
        } else {
            self.tps_sum / self.tps_count as f64
        }
    }
}

fn normalized_optional_filter(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
}

struct SqlScopeClause {
    sql_and: String,
    sql_where: String,
    params: Vec<Value>,
}

fn build_scope_clause(
    base_param_count: usize,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> SqlScopeClause {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    let mut next_idx = base_param_count + 1;
    if let Some(prov) = provider {
        parts.push(format!("provider_id = ?{next_idx}"));
        params.push(Value::Text(prov.to_string()));
        next_idx += 1;
    }
    if let Some(dev) = device_id {
        parts.push(format!("device_id = ?{next_idx}"));
        params.push(Value::Text(dev.to_string()));
    }
    if parts.is_empty() {
        SqlScopeClause {
            sql_and: String::new(),
            sql_where: String::new(),
            params,
        }
    } else {
        let joined = parts.join(" AND ");
        SqlScopeClause {
            sql_and: format!(" AND {joined}"),
            sql_where: format!(" WHERE {joined}"),
            params,
        }
    }
}

fn bound_range_params(start_ms: i64, end_ms: i64, extra: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(2 + extra.len());
    out.push(Value::Integer(start_ms));
    out.push(Value::Integer(end_ms));
    out.extend_from_slice(extra);
    out
}

fn short_session_key(provider_id: &str, session_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider_id.as_bytes());
    hasher.update(b":");
    hasher.update(session_id.as_bytes());
    let digest = hasher.finalize();
    format!("#{:02x}{:02x}{:02x}", digest[0], digest[1], digest[2])
}

fn sanitize_project_label(raw: &str) -> String {
    let leaf = raw
        .trim()
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim();
    if leaf.is_empty() {
        "workspace".to_string()
    } else {
        leaf.to_string()
    }
}

fn pct_share(part: i64, total: i64) -> f64 {
    if total <= 0 || part <= 0 {
        0.0
    } else {
        (part as f64 / total as f64) * 100.0
    }
}

fn pct_change(current: f64, previous: f64) -> Option<f64> {
    if previous.abs() < f64::EPSILON {
        None
    } else {
        Some(((current - previous) / previous) * 100.0)
    }
}

fn local_offset(tz_offset_minutes: i32) -> FixedOffset {
    use chrono::Offset;
    FixedOffset::east_opt(tz_offset_minutes.clamp(-720, 840) * 60).unwrap_or_else(|| Utc.fix())
}

fn local_day_start_ms(ts_ms: i64, tz_offset_minutes: i32) -> i64 {
    let offset_ms = (tz_offset_minutes.clamp(-720, 840) as i64) * 60_000;
    (ts_ms + offset_ms).div_euclid(DAY_MS) * DAY_MS - offset_ms
}

fn local_week_start_ms(ts_ms: i64, tz_offset_minutes: i32) -> i64 {
    const WEEK_MS: i64 = 7 * DAY_MS;
    let offset_ms = (tz_offset_minutes.clamp(-720, 840) as i64) * 60_000;
    // 1970-01-01 UTC was a Thursday (+3 days after Monday 1969-12-29).
    let shifted = ts_ms + offset_ms + 3 * DAY_MS;
    shifted.div_euclid(WEEK_MS) * WEEK_MS - 3 * DAY_MS - offset_ms
}

fn inclusive_window_span(start_ms: i64, end_ms: i64) -> i64 {
    let raw = end_ms.saturating_sub(start_ms);
    if end_ms.rem_euclid(1_000) == 999 {
        raw.saturating_add(1).max(HOUR_MS)
    } else {
        raw.max(HOUR_MS)
    }
}

pub(crate) fn local_day_bounds_ms(now_ms: i64, tz_offset_minutes: i32) -> (i64, i64) {
    let offset = local_offset(tz_offset_minutes);
    let dt = Utc
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_else(Utc::now)
        .with_timezone(&offset);
    let date = dt.date_naive();
    let start_dt = date
        .and_hms_opt(0, 0, 0)
        .and_then(|ndt| offset.from_local_datetime(&ndt).single())
        .map(|d| d.timestamp_millis())
        .unwrap_or(now_ms - DAY_MS);
    let month_start_date = NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date);
    let month_start_ms = month_start_date
        .and_hms_opt(0, 0, 0)
        .and_then(|ndt| offset.from_local_datetime(&ndt).single())
        .map(|d| d.timestamp_millis())
        .unwrap_or(start_dt);
    (start_dt, month_start_ms)
}

fn date_key_from_ms(ts_ms: i64, offset: FixedOffset) -> String {
    Utc.timestamp_millis_opt(ts_ms)
        .single()
        .map(|dt| dt.with_timezone(&offset).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}

fn query_period_totals(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> CoreResult<PeriodTotals> {
    let scope = build_scope_clause(2, provider, device_id);
    let sql = format!(
        r#"
        SELECT
            COALESCE(SUM(request_count), 0),
            COALESCE(SUM(observable_step_count), 0),
            COALESCE(SUM(input_tokens), 0),
            COALESCE(SUM(output_tokens), 0),
            COALESCE(SUM(cache_read_tokens), 0),
            COALESCE(SUM(cache_write_tokens), 0),
            COALESCE(SUM(reasoning_tokens), 0),
            COALESCE(SUM(estimated_cost_micro_usd), 0),
            COALESCE(SUM(cache_saved_micro_usd), 0),
            COALESCE(SUM(tps_sample_count), 0),
            COALESCE(SUM(tps_sum), 0.0),
            COALESCE(MAX(max_output_tps), 0.0)
        FROM usage_hourly_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2{where_and}
        "#,
        where_and = scope.sql_and
    );

    // Align lower bound down to hour bucket boundary so exact hour-bucketed rollups match event windows.
    let aligned_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let bound = bound_range_params(aligned_start, end_ms, &scope.params);
    conn.query_row(&sql, params_from_iter(bound), |row| {
        Ok(PeriodTotals {
            requests: row.get(0)?,
            observable_steps: row.get(1)?,
            input: row.get(2)?,
            output: row.get(3)?,
            cache_read: row.get(4)?,
            cache_write: row.get(5)?,
            reasoning: row.get(6)?,
            cost_micro_usd: row.get(7)?,
            cache_saved_micro_usd: row.get(8)?,
            tps_count: row.get(9)?,
            tps_sum: row.get(10)?,
            max_tps: row.get(11)?,
        })
    })
    .map_err(CoreError::from)
}

fn resolve_effective_window(
    conn: &Connection,
    filter: &AnalyticsFilter,
    provider: Option<&str>,
    device_id: Option<&str>,
    fallback_now_ms: i64,
) -> CoreResult<(i64, i64)> {
    if !filter.all_time {
        let start = filter.start_ms.min(filter.end_ms);
        return Ok((start, filter.end_ms));
    }
    let end_ms = if filter.end_ms > 0 {
        filter.end_ms
    } else {
        fallback_now_ms
    };
    let scope = build_scope_clause(0, provider, device_id);
    let events_sql = format!(
        "SELECT MIN(timestamp_ms) FROM usage_events{where_clause}",
        where_clause = scope.sql_where
    );
    let events_min: Option<i64> =
        conn.query_row(&events_sql, params_from_iter(scope.params.clone()), |r| r.get(0))?;
    let rollups_sql = format!(
        "SELECT MIN(bucket_start_ms) FROM usage_hourly_rollups{where_clause}",
        where_clause = scope.sql_where
    );
    let rollups_min: Option<i64> =
        conn.query_row(&rollups_sql, params_from_iter(scope.params), |r| r.get(0))?;

    let min_ts = match (events_min, rollups_min) {
        (Some(e), Some(r)) => Some(e.min(r)),
        (Some(e), None) => Some(e),
        (None, Some(r)) => Some(r),
        (None, None) => None,
    };
    let start_ms = min_ts
        .unwrap_or_else(|| end_ms.saturating_sub(7 * DAY_MS))
        .min(end_ms);
    Ok((start_ms, end_ms))
}

fn resolve_trend_granularity(requested: TrendGranularity, span_ms: i64) -> TrendGranularity {
    match requested {
        TrendGranularity::Auto => {
            if span_ms <= 2 * DAY_MS {
                TrendGranularity::Hourly
            } else if span_ms <= 90 * DAY_MS {
                TrendGranularity::Daily
            } else {
                TrendGranularity::Weekly
            }
        }
        other => other,
    }
}

fn granularity_bucket_ms(granularity: TrendGranularity) -> i64 {
    match granularity {
        TrendGranularity::FifteenMinutes => 900_000,
        TrendGranularity::Auto | TrendGranularity::Hourly => HOUR_MS,
        TrendGranularity::Daily => DAY_MS,
        TrendGranularity::Weekly => 7 * DAY_MS,
    }
}

fn is_observed_window_fresh(obs: &ObservedRateLimitWindow, is_secondary: bool, now_ms: i64) -> bool {
    match obs.resets_at_ms {
        Some(reset_ms) => reset_ms > now_ms,
        None => {
            let fallback_ms = obs
                .window_minutes
                .map(|m| m.max(1) * 60_000)
                .unwrap_or_else(|| if is_secondary { 7 * DAY_MS } else { 5 * HOUR_MS });
            now_ms.saturating_sub(obs.observed_at_ms) < fallback_ms
        }
    }
}

fn fallback_observed_window_status(
    observed: &[ObservedRateLimitWindow],
    provider_id: &str,
    now_ms: i64,
) -> (Option<String>, Option<f64>) {
    let mut best_primary: Option<(i64, f64)> = None;
    let mut best_secondary: Option<(i64, f64)> = None;

    for obs in observed {
        if !obs.provider_id.eq_ignore_ascii_case(provider_id) {
            continue;
        }
        let name = obs.window_name.to_ascii_lowercase();
        let is_secondary = obs.window_minutes.is_some_and(|m| m >= 1_440)
            || name.contains("secondary")
            || name.contains("7d");
        if !is_observed_window_fresh(obs, is_secondary, now_ms) {
            continue;
        }
        let pct = if obs.used_percent.is_finite() {
            obs.used_percent.clamp(0.0, 100.0)
        } else {
            continue;
        };
        if is_secondary {
            if best_secondary.is_none_or(|(ts, _)| obs.observed_at_ms >= ts) {
                best_secondary = Some((obs.observed_at_ms, pct));
            }
        } else if best_primary.is_none_or(|(ts, _)| obs.observed_at_ms >= ts) {
            best_primary = Some((obs.observed_at_ms, pct));
        }
    }

    if let Some((_, pct)) = best_primary {
        (Some("5h 官方限额".to_string()), Some(pct))
    } else if let Some((_, pct)) = best_secondary {
        (Some("7d 官方限额".to_string()), Some(pct))
    } else {
        (None, None)
    }
}

pub(crate) fn query_latest_tps(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> CoreResult<Option<f64>> {
    Ok(
        query_latest_tps_with_model(conn, start_ms, end_ms, provider, device_id)?
            .map(|(tps, _, _)| tps),
    )
}

pub(crate) fn query_latest_tps_with_model(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> CoreResult<Option<(f64, String, String)>> {
    let scope = build_scope_clause(2, provider, device_id);
    let sql = format!(
        r#"
        SELECT output_tps, model_id, provider_id FROM usage_events
        WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2{where_and}
          AND LOWER(model_id) NOT LIKE '%auto-review%'
          AND LOWER(model_id) NOT LIKE '%auto_review%'
          AND NOT EXISTS (
              SELECT 1 FROM session_titles st
              WHERE st.session_key = usage_events.provider_id || ':' || usage_events.session_id
                AND st.source = 'background'
          )
          AND output_tps IS NOT NULL
          AND output_tps > 0.0
          AND output_tps <= {MAX_VALID_OUTPUT_TPS}
          AND duration_ms IS NOT NULL
          AND duration_ms >= {MIN_VALID_TPS_DURATION_MS}
          AND (output_tokens + reasoning_tokens) >= {MIN_VALID_TPS_OUTPUT_TOKENS}
        ORDER BY timestamp_ms DESC, rowid DESC LIMIT 6
        "#,
        where_and = scope.sql_and
    );
    let bound = bound_range_params(start_ms, end_ms, &scope.params);
    let mut stmt = conn.prepare(&sql)?;
    let recent_rows: Vec<(f64, String, String)> = stmt
        .query_map(params_from_iter(bound), |r| {
            Ok((
                r.get::<_, f64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if let Some((_, latest_model, latest_provider)) = recent_rows.first().cloned() {
        let mut recent_tps: Vec<f64> = recent_rows
            .iter()
            .filter(|(_, m, p)| m == &latest_model && p == &latest_provider)
            .map(|(tps, _, _)| *tps)
            .collect();
        recent_tps.reverse();
        if let Some(smoothed) = compute_ewma_tps(&recent_tps, 0.45) {
            return Ok(Some((smoothed, latest_model, latest_provider)));
        }
    }

    // Pure-rollup devices (e.g. mobile clients) do not hold raw `usage_events`.
    let aligned_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let rollup_sql = format!(
        r#"
        SELECT tps_sum / tps_sample_count AS avg_tps, model_id, provider_id
        FROM usage_hourly_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2{where_and}
          AND LOWER(model_id) NOT LIKE '%auto-review%'
          AND LOWER(model_id) NOT LIKE '%auto_review%'
          AND tps_sample_count > 0
          AND tps_sum > 0.0
        ORDER BY bucket_start_ms DESC, updated_at_ms DESC, rowid DESC
        LIMIT 1
        "#,
        where_and = scope.sql_and
    );
    let rollup_bound = bound_range_params(aligned_start, end_ms, &scope.params);
    match conn.query_row(&rollup_sql, params_from_iter(rollup_bound), |r| {
        Ok((
            r.get::<_, f64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    }) {
        Ok((v, m, p)) if v.is_finite() && v > 0.0 && v <= MAX_VALID_OUTPUT_TPS => {
            Ok(Some((v, m, p)))
        }
        Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(CoreError::from(e)),
    }
}

fn compute_live_model_speeds_and_tps(
    active_sessions: &[ActiveSessionSummary],
    fallback_recent: Option<(f64, String, String)>,
) -> (Option<f64>, Vec<LiveModelSpeed>) {
    let has_active_with_tps = active_sessions.iter().any(|s| {
        s.activity_state == "active"
            && s.latest_output_tps.is_some_and(|v| v.is_finite() && v > 0.0)
    });

    let mut grouped_order: Vec<(String, String)> = Vec::new();
    let mut grouped_vals: HashMap<(String, String), (Vec<f64>, bool)> = HashMap::new();

    for sess in active_sessions {
        let Some(tps) = sess.latest_output_tps.filter(|v| v.is_finite() && *v > 0.0) else {
            continue;
        };
        if has_active_with_tps && sess.activity_state != "active" {
            continue;
        }
        let key = (sess.provider_id.clone(), sess.model_id.clone());
        if !grouped_vals.contains_key(&key) {
            grouped_order.push(key.clone());
        }
        let entry = grouped_vals.entry(key).or_insert_with(|| (Vec::new(), false));
        entry.0.push(tps);
        if sess.activity_state == "active" {
            entry.1 = true;
        }
    }

    if !grouped_order.is_empty() {
        let mut items = Vec::with_capacity(grouped_order.len());
        for (provider_id, model_id) in grouped_order {
            if let Some((vals, is_active)) =
                grouped_vals.remove(&(provider_id.clone(), model_id.clone()))
            {
                let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                items.push(LiveModelSpeed {
                    provider_id,
                    model_id,
                    output_tps: mean,
                    activity_state: if is_active {
                        "active".to_string()
                    } else {
                        "recent".to_string()
                    },
                });
            }
        }
        let primary_tps = items.iter().map(|i| i.output_tps).sum::<f64>() / items.len() as f64;
        return (Some(primary_tps), items);
    }

    if let Some((tps, model_id, provider_id)) = fallback_recent {
        return (
            Some(tps),
            vec![LiveModelSpeed {
                provider_id,
                model_id,
                output_tps: tps,
                activity_state: "recent".to_string(),
            }],
        );
    }

    (None, Vec::new())
}

pub(crate) fn query_provider_shares(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> CoreResult<Vec<UsageShareItem>> {
    let aligned_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let scope = build_scope_clause(2, provider, device_id);
    let sql = format!(
        r#"
        SELECT provider_id,
               SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens) AS total_tok,
               SUM(estimated_cost_micro_usd) AS total_cost
        FROM usage_hourly_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2{where_and}
        GROUP BY provider_id
        HAVING total_tok > 0 OR total_cost > 0
        ORDER BY total_tok DESC, total_cost DESC, provider_id ASC
        "#,
        where_and = scope.sql_and
    );
    let bound = bound_range_params(aligned_start, end_ms, &scope.params);
    let mut stmt = conn.prepare(&sql)?;
    let mut raw = Vec::new();
    let rows = stmt.query_map(params_from_iter(bound), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        raw.push(row?);
    }
    let total_tokens: i64 = raw.iter().map(|(_, t, _)| *t).sum();
    let total_cost: i64 = raw.iter().map(|(_, _, c)| *c).sum();
    Ok(raw
        .into_iter()
        .map(|(pid, tokens, cost_micro_usd)| UsageShareItem {
            label: provider_ids::display_name(&pid).to_string(),
            key: pid,
            tokens,
            token_share_percent: pct_share(tokens, total_tokens),
            cost_micro_usd,
            cost_share_percent: pct_share(cost_micro_usd, total_cost),
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn query_top_model_shares(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
    period_total_tokens: i64,
    period_total_cost: i64,
    limit: usize,
) -> CoreResult<Vec<UsageShareItem>> {
    let aligned_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let scope = build_scope_clause(2, provider, device_id);
    let limit_idx = 3 + scope.params.len();
    let sql = format!(
        r#"
        SELECT model_id,
               SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens) AS total_tok,
               SUM(estimated_cost_micro_usd) AS total_cost
        FROM usage_hourly_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2{where_and}
        GROUP BY model_id
        HAVING total_tok > 0 OR total_cost > 0
        ORDER BY total_tok DESC, total_cost DESC, model_id ASC
        LIMIT ?{limit_idx}
        "#,
        where_and = scope.sql_and
    );
    let mut bound = bound_range_params(aligned_start, end_ms, &scope.params);
    bound.push(Value::Integer(limit as i64));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(bound), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (model_id, tokens, cost_micro_usd) = row?;
        out.push(UsageShareItem {
            label: model_id.clone(),
            key: model_id,
            tokens,
            token_share_percent: pct_share(tokens, period_total_tokens),
            cost_micro_usd,
            cost_share_percent: pct_share(cost_micro_usd, period_total_cost),
        });
    }
    Ok(out)
}

fn is_background_session_marker(
    session_id: &str,
    model_id: &str,
    stored_title: Option<&str>,
    stored_source: Option<&str>,
) -> bool {
    if stored_source == Some("background") {
        return true;
    }
    let sid_lower = session_id.to_ascii_lowercase();
    if sid_lower.contains("/subagents/") || sid_lower.contains("\\subagents\\") {
        return true;
    }
    let model_lower = model_id.to_ascii_lowercase();
    if model_lower.contains("auto-review") || model_lower.contains("auto_review") {
        return true;
    }
    if let Some(title) = stored_title {
        let title_lower = title.trim().to_ascii_lowercase();
        if matches!(
            title_lower.as_str(),
            "guardian review" | "codex-auto-review" | "auto-review" | "auto review"
        ) {
            return true;
        }
    }
    false
}

fn evaluate_subscriptions_with_spend(
    conn: &Connection,
    plans: Vec<SubscriptionPlan>,
    now_ms: i64,
    tz_offset_minutes: i32,
    include_disabled: bool,
) -> CoreResult<Vec<SubscriptionPlanStatus>> {
    let filtered_plans: Vec<SubscriptionPlan> = plans
        .into_iter()
        .filter(|p| p.is_user_configured && (include_disabled || p.enabled))
        .collect();
    let observed = load_observed_rate_limits(conn)?;
    let samples_map = load_window_samples_map(conn, now_ms)?;
    let (today_start_ms, month_start_ms) = local_day_bounds_ms(now_ms, tz_offset_minutes);
    let week_start_ms = now_ms.saturating_sub(7 * DAY_MS);

    let mut spend_cache: HashMap<String, (i64, i64, i64, i64)> = HashMap::new();
    let mut out = Vec::with_capacity(filtered_plans.len());

    for plan in filtered_plans {
        let samples = samples_map
            .get(&plan.provider_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mut status = evaluate_subscription_plan(&plan, samples, &observed, now_ms);

        let spend_key = plan.provider_id.to_ascii_lowercase();
        let (today_cost, week_cost, month_cost, month_tokens) =
            if let Some(&cached) = spend_cache.get(&spend_key) {
                cached
            } else {
                let today_t =
                    query_period_totals(conn, today_start_ms, now_ms, Some(&plan.provider_id), None)?;
                let week_t =
                    query_period_totals(conn, week_start_ms, now_ms, Some(&plan.provider_id), None)?;
                let month_t =
                    query_period_totals(conn, month_start_ms, now_ms, Some(&plan.provider_id), None)?;
                let tuple = (
                    today_t.cost_micro_usd,
                    week_t.cost_micro_usd,
                    month_t.cost_micro_usd,
                    month_t.total_tokens(),
                );
                spend_cache.insert(spend_key, tuple);
                tuple
            };

        status.today_cost_micro_usd = today_cost;
        status.week_cost_micro_usd = week_cost;
        status.month_cost_micro_usd = month_cost;
        status.month_tokens = month_tokens;
        out.push(status);
    }

    Ok(out)
}

fn query_active_sessions(
    conn: &Connection,
    now_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
    limit: usize,
) -> CoreResult<Vec<ActiveSessionSummary>> {
    const ACTIVE_STATE_WINDOW_MS: i64 = 120_000;
    const RECENT_WINDOW_MS: i64 = 900_000;

    let day_start_ms = now_ms.saturating_sub(DAY_MS);
    let recent_cutoff_ms = now_ms.saturating_sub(RECENT_WINDOW_MS);
    let scope = build_scope_clause(2, provider, device_id);
    let cutoff_idx = 3 + scope.params.len();
    let limit_idx = cutoff_idx + 1;

    let sql = format!(
        r#"
        SELECT provider_id,
               session_id,
               MIN(timestamp_ms) AS started_at_ms,
               MAX(timestamp_ms) AS last_active_at_ms,
               COUNT(*) AS request_count,
               SUM(input_tokens),
               SUM(output_tokens),
               SUM(cache_read_tokens),
               SUM(cache_write_tokens),
               SUM(reasoning_tokens),
               SUM(estimated_cost_micro_usd)
        FROM usage_events
        WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2{where_and}
          AND LOWER(model_id) NOT LIKE '%auto-review%'
          AND LOWER(model_id) NOT LIKE '%auto_review%'
        GROUP BY provider_id, session_id
        HAVING MAX(timestamp_ms) >= ?{cutoff_idx}
        ORDER BY last_active_at_ms DESC, request_count DESC
        LIMIT ?{limit_idx}
        "#,
        where_and = scope.sql_and
    );

    let fetch_limit = (limit.max(1) * 4).min(128) as i64;
    let mut bound = bound_range_params(day_start_ms, now_ms, &scope.params);
    bound.push(Value::Integer(recent_cutoff_ms));
    bound.push(Value::Integer(fetch_limit));

    #[allow(clippy::type_complexity)]
    struct SessionGroupRow {
        provider_id: String,
        session_id: String,
        started_at_ms: i64,
        last_active_at_ms: i64,
        request_count: i64,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
        reasoning_tokens: i64,
        estimated_cost_micro_usd: i64,
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(bound), |r| {
        Ok(SessionGroupRow {
            provider_id: r.get(0)?,
            session_id: r.get(1)?,
            started_at_ms: r.get(2)?,
            last_active_at_ms: r.get(3)?,
            request_count: r.get(4)?,
            input_tokens: r.get(5)?,
            output_tokens: r.get(6)?,
            cache_read_tokens: r.get(7)?,
            cache_write_tokens: r.get(8)?,
            reasoning_tokens: r.get(9)?,
            estimated_cost_micro_usd: r.get(10)?,
        })
    })?;

    let mut groups = Vec::new();
    for r in rows {
        groups.push(r?);
    }

    let dev_clause = if device_id.is_some() {
        " AND device_id = ?5"
    } else {
        ""
    };
    let latest_meta_sql = format!(
        r#"
        SELECT model_id, project_label
        FROM usage_events
        WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2
          AND provider_id = ?3 AND session_id = ?4{dev_clause}
        ORDER BY timestamp_ms DESC, rowid DESC
        LIMIT 1
        "#
    );
    let latest_tps_sql = format!(
        r#"
        SELECT output_tps
        FROM usage_events
        WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2
          AND provider_id = ?3 AND session_id = ?4{dev_clause}
          AND output_tps IS NOT NULL
          AND output_tps > 0.0
          AND output_tps <= {MAX_VALID_OUTPUT_TPS}
          AND duration_ms IS NOT NULL
          AND duration_ms >= {MIN_VALID_TPS_DURATION_MS}
          AND (output_tokens + reasoning_tokens) >= {MIN_VALID_TPS_OUTPUT_TOKENS}
        ORDER BY timestamp_ms DESC, rowid DESC
        LIMIT 6
        "#
    );

    let mut meta_stmt = conn.prepare(&latest_meta_sql)?;
    let mut tps_stmt = conn.prepare(&latest_tps_sql)?;
    let mut title_stmt =
        conn.prepare("SELECT title, source FROM session_titles WHERE session_key = ?1 LIMIT 1")?;

    let mut out = Vec::with_capacity(limit);
    for g in groups {
        let mut p = vec![
            Value::Integer(day_start_ms),
            Value::Integer(now_ms),
            Value::Text(g.provider_id.clone()),
            Value::Text(g.session_id.clone()),
        ];
        if let Some(dev) = device_id {
            p.push(Value::Text(dev.to_string()));
        }

        let (model_id, raw_project) = match meta_stmt
            .query_row(params_from_iter(p.clone()), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            }) {
            Ok(pair) => pair,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                ("unknown".to_string(), "workspace".to_string())
            }
            Err(e) => return Err(CoreError::from(e)),
        };

        let canonical_key = format!("{}:{}", g.provider_id, g.session_id);
        let mut stored_entry: Option<(String, String)> = title_stmt
            .query_row(params![canonical_key], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .optional()?;
        if stored_entry.is_none() {
            if let Some((base_sid, _)) = g.session_id.split_once('/') {
                let base_key = format!("{}:{}", g.provider_id, base_sid);
                stored_entry = title_stmt
                    .query_row(params![base_key], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .optional()?;
            }
        }
        let stored_title = stored_entry.as_ref().map(|(t, _)| t.as_str());
        let stored_source = stored_entry.as_ref().map(|(_, s)| s.as_str());
        if is_background_session_marker(&g.session_id, &model_id, stored_title, stored_source) {
            continue;
        }

        let mut session_tps_desc: Vec<f64> = tps_stmt
            .query_map(params_from_iter(p), |r| r.get::<_, f64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        session_tps_desc.reverse();
        let latest_output_tps = compute_ewma_tps(&session_tps_desc, 0.45);

        let activity_state = if now_ms.saturating_sub(g.last_active_at_ms) <= ACTIVE_STATE_WINDOW_MS
        {
            "active".to_string()
        } else {
            "recent".to_string()
        };
        let total_tokens = g.input_tokens
            + g.output_tokens
            + g.cache_read_tokens
            + g.cache_write_tokens
            + g.reasoning_tokens;

        let session_title = stored_entry.map(|(t, _)| t).or_else(|| {
            let trimmed = g.session_id.trim();
            let lower = trimmed.to_ascii_lowercase();
            let looks_synthetic_id = lower.is_empty()
                || lower == "unknown"
                || lower == "default"
                || lower == "wire"
                || lower == "context"
                || lower.starts_with("sess-")
                || lower.starts_with("session-")
                || lower.starts_with("kimi-session")
                || lower.starts_with("conv-")
                || lower.starts_with("rollout-")
                || lower.contains('/')
                || trimmed.chars().all(|c| c.is_ascii_hexdigit() || c == '-' || c == '_');
            if looks_synthetic_id {
                None
            } else {
                sanitize_session_title(trimmed)
            }
        });

        out.push(ActiveSessionSummary {
            session_key: short_session_key(&g.provider_id, &g.session_id),
            session_title,
            provider_id: g.provider_id,
            model_id,
            project_label: sanitize_project_label(&raw_project),
            activity_state,
            started_at_ms: g.started_at_ms,
            last_active_at_ms: g.last_active_at_ms,
            request_count: g.request_count,
            total_tokens,
            input_tokens: g.input_tokens,
            output_tokens: g.output_tokens,
            cache_read_tokens: g.cache_read_tokens,
            estimated_cost_micro_usd: g.estimated_cost_micro_usd,
            latest_output_tps,
        });
        if out.len() >= limit {
            break;
        }
    }

    Ok(out)
}

impl Storage {
    pub fn query_overview(&self, filter: &AnalyticsFilter, now_ms: i64) -> CoreResult<OverviewReport> {
        self.with_conn(|conn| {
            let provider = normalized_optional_filter(&filter.provider_id);
            let device_id = normalized_optional_filter(&filter.device_id);
            let (eff_start_ms, eff_end_ms) =
                resolve_effective_window(conn, filter, provider, device_id, now_ms)?;
            let current = query_period_totals(conn, eff_start_ms, eff_end_ms, provider, device_id)?;

            let span = inclusive_window_span(eff_start_ms, eff_end_ms);
            let (tokens_delta_pct, cost_delta_pct, cache_delta_pct, tps_delta_pct) =
                if filter.all_time {
                    (None, None, None, None)
                } else {
                    let prev_start = eff_start_ms.saturating_sub(span);
                    let prev_end = eff_start_ms.saturating_sub(1);
                    let previous =
                        query_period_totals(conn, prev_start, prev_end, provider, device_id)?;
                    (
                        pct_change(current.total_tokens() as f64, previous.total_tokens() as f64),
                        pct_change(current.cost_micro_usd as f64, previous.cost_micro_usd as f64),
                        pct_change(current.cache_hit_ratio(), previous.cache_hit_ratio()),
                        pct_change(current.avg_tps(), previous.avg_tps()),
                    )
                };

            let fallback_latest =
                query_latest_tps_with_model(conn, eff_start_ms, eff_end_ms, provider, device_id)?;

            // Choose hourly series when range <= 48 hours, otherwise daily series
            let use_hourly = span <= 48 * HOUR_MS;
            let (table_name, bucket_size, series_granularity) = if use_hourly {
                ("usage_hourly_rollups", HOUR_MS, "hourly".to_string())
            } else {
                ("usage_daily_rollups", DAY_MS, "daily".to_string())
            };
            let aligned_start = eff_start_ms.div_euclid(bucket_size) * bucket_size;
            let scope = build_scope_clause(2, provider, device_id);
            let series_bound = bound_range_params(aligned_start, eff_end_ms, &scope.params);

            let sql = format!(
                r#"
                SELECT bucket_start_ms,
                       SUM(input_tokens), SUM(output_tokens),
                       SUM(cache_read_tokens), SUM(cache_write_tokens),
                       SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
                       SUM(request_count)
                FROM {table_name}
                WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2{where_and}
                GROUP BY bucket_start_ms
                ORDER BY bucket_start_ms ASC
                "#,
                where_and = scope.sql_and
            );

            let mut series = Vec::new();
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(series_bound.clone()), |row| {
                Ok(TimeBucketPoint {
                    bucket_start_ms: row.get(0)?,
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    cache_read_tokens: row.get(3)?,
                    cache_write_tokens: row.get(4)?,
                    reasoning_tokens: row.get(5)?,
                    cost_micro_usd: row.get(6)?,
                    requests: row.get(7)?,
                })
            })?;
            for row in rows {
                series.push(row?);
            }

            if !series.is_empty() {
                let aligned_end = eff_end_ms.div_euclid(bucket_size) * bucket_size;
                let first_bucket = series[0].bucket_start_ms;
                let last_bucket = series
                    .last()
                    .map(|p| p.bucket_start_ms)
                    .unwrap_or(first_bucket);
                let (fill_start, fill_end) = if !filter.all_time {
                    (aligned_start.min(first_bucket), aligned_end.max(last_bucket))
                } else {
                    (first_bucket, last_bucket)
                };
                let bucket_span = (fill_end - fill_start).div_euclid(bucket_size);
                if (0..=366).contains(&bucket_span) {
                    let existing: HashMap<i64, TimeBucketPoint> =
                        series.into_iter().map(|p| (p.bucket_start_ms, p)).collect();
                    let mut filled = Vec::with_capacity((bucket_span + 1) as usize);
                    let mut cursor_b = fill_start;
                    while cursor_b <= fill_end {
                        if let Some(pt) = existing.get(&cursor_b) {
                            filled.push(pt.clone());
                        } else {
                            filled.push(TimeBucketPoint {
                                bucket_start_ms: cursor_b,
                                input_tokens: 0,
                                output_tokens: 0,
                                cache_read_tokens: 0,
                                cache_write_tokens: 0,
                                reasoning_tokens: 0,
                                cost_micro_usd: 0,
                                requests: 0,
                            });
                        }
                        cursor_b = cursor_b.saturating_add(bucket_size);
                    }
                    series = filled;
                }
            }

            let prov_series_sql = format!(
                r#"
                SELECT bucket_start_ms, provider_id,
                       SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens),
                       SUM(estimated_cost_micro_usd),
                       SUM(request_count)
                FROM {table_name}
                WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2{where_and}
                GROUP BY bucket_start_ms, provider_id
                ORDER BY bucket_start_ms ASC
                "#,
                where_and = scope.sql_and
            );

            let mut prov_buckets: BTreeMap<String, BTreeMap<i64, (i64, i64, i64)>> =
                BTreeMap::new();
            let mut prov_stmt = conn.prepare(&prov_series_sql)?;
            let prov_rows_iter = prov_stmt.query_map(params_from_iter(series_bound), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            for row in prov_rows_iter {
                let (b, pid, tok, cost, req) = row?;
                prov_buckets.entry(pid).or_default().insert(b, (tok, cost, req));
            }

            let aligned_buckets: Vec<i64> = series.iter().map(|p| p.bucket_start_ms).collect();
            let mut provider_series: Vec<TrendSeries> = prov_buckets
                .into_iter()
                .map(|(pid, bucket_map)| {
                    let mut total_tokens = 0i64;
                    let mut total_cost = 0i64;
                    let points = aligned_buckets
                        .iter()
                        .map(|&b| {
                            let (t, c, r) = bucket_map.get(&b).copied().unwrap_or((0, 0, 0));
                            total_tokens += t;
                            total_cost += c;
                            TrendPoint {
                                bucket_start_ms: b,
                                tokens: t,
                                cost_micro_usd: c,
                                requests: r,
                                cache_hit_ratio: 0.0,
                            }
                        })
                        .collect();
                    TrendSeries {
                        label: format_dimension_label(StackDimension::Provider, &pid),
                        key: pid,
                        total_tokens,
                        total_cost_micro_usd: total_cost,
                        points,
                    }
                })
                .filter(|s| s.total_tokens > 0)
                .collect();
            provider_series.sort_by(|a, b| {
                b.total_tokens
                    .cmp(&a.total_tokens)
                    .then_with(|| a.key.cmp(&b.key))
            });

            // Build per-provider summary rows including today & month spend + active subscription or observed limit status
            let (today_start_ms, month_start_ms) =
                local_day_bounds_ms(now_ms, filter.tz_offset_minutes);
            let today_totals =
                query_period_totals(conn, today_start_ms, now_ms, provider, device_id)?;
            let yesterday_start_ms = today_start_ms.saturating_sub(DAY_MS);
            let yesterday_end_ms = today_start_ms.saturating_sub(1);
            let yesterday_totals =
                query_period_totals(conn, yesterday_start_ms, yesterday_end_ms, provider, device_id)?;
            let today_tokens_delta_pct = pct_change(
                today_totals.total_tokens() as f64,
                yesterday_totals.total_tokens() as f64,
            );
            let today_cost_delta_pct = pct_change(
                today_totals.cost_micro_usd as f64,
                yesterday_totals.cost_micro_usd as f64,
            );

            let today_day_end_ms = today_start_ms.saturating_add(24 * HOUR_MS - 1);
            let today_hourly_rows =
                fetch_hourly_rows(conn, today_start_ms, today_day_end_ms, provider, device_id)?;
            let mut today_hourly_pulse: Vec<TimeBucketPoint> = (0..24)
                .map(|hour_idx| TimeBucketPoint {
                    bucket_start_ms: today_start_ms.saturating_add(hour_idx * HOUR_MS),
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    reasoning_tokens: 0,
                    cost_micro_usd: 0,
                    requests: 0,
                })
                .collect();
            for r in today_hourly_rows {
                if r.bucket_start_ms >= today_start_ms && r.bucket_start_ms <= today_day_end_ms {
                    let idx = ((r.bucket_start_ms - today_start_ms) / HOUR_MS) as usize;
                    if let Some(pt) = today_hourly_pulse.get_mut(idx) {
                        pt.input_tokens += r.input_tokens;
                        pt.output_tokens += r.output_tokens;
                        pt.cache_read_tokens += r.cache_read_tokens;
                        pt.cache_write_tokens += r.cache_write_tokens;
                        pt.reasoning_tokens += r.reasoning_tokens;
                        pt.cost_micro_usd += r.cost_micro_usd;
                        pt.requests += r.requests;
                    }
                }
            }

            let plans = load_subscription_plans(conn)?;
            let observed = load_observed_rate_limits(conn)?;
            let samples_map = load_window_samples_map(conn, now_ms)?;

            let mut prov_rows = Vec::new();
            let mut cfg_stmt = conn.prepare(
                "SELECT provider_id, enabled, last_sync_at_ms, last_error FROM provider_configs ORDER BY provider_id ASC",
            )?;
            let cfg_rows = cfg_stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? != 0,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?;

            for cfg in cfg_rows {
                let (pid, enabled, last_sync_at_ms, last_error) = cfg?;
                if let Some(filter_pid) = provider {
                    if !pid.eq_ignore_ascii_case(filter_pid) {
                        continue;
                    }
                }
                let period_t =
                    query_period_totals(conn, eff_start_ms, eff_end_ms, Some(&pid), device_id)?;
                let today_t =
                    query_period_totals(conn, today_start_ms, now_ms, Some(&pid), device_id)?;
                let month_t =
                    query_period_totals(conn, month_start_ms, now_ms, Some(&pid), device_id)?;

                let active_plan = plans.iter().find(|p| {
                    p.enabled && p.is_user_configured && p.provider_id.eq_ignore_ascii_case(&pid)
                });
                let (active_window_label, active_window_used_percent) =
                    if let Some(plan) = active_plan {
                        let samples = samples_map.get(&pid).map(Vec::as_slice).unwrap_or(&[]);
                        let eval = evaluate_subscription_plan(plan, samples, &observed, now_ms);
                        eval.windows
                            .first()
                            .map(|w| (Some(w.label.clone()), Some(w.used_percent)))
                            .unwrap_or_else(|| fallback_observed_window_status(&observed, &pid, now_ms))
                    } else {
                        fallback_observed_window_status(&observed, &pid, now_ms)
                    };

                prov_rows.push(ProviderSummaryRow {
                    provider_id: pid,
                    enabled,
                    period_tokens: period_t.total_tokens(),
                    period_cost_micro_usd: period_t.cost_micro_usd,
                    today_tokens: today_t.total_tokens(),
                    today_cost_micro_usd: today_t.cost_micro_usd,
                    month_tokens: month_t.total_tokens(),
                    month_cost_micro_usd: month_t.cost_micro_usd,
                    cache_hit_ratio: period_t.cache_hit_ratio(),
                    observable_steps: period_t.observable_steps,
                    active_window_label,
                    active_window_used_percent,
                    last_sync_at_ms,
                    last_error,
                });
            }

            let provider_shares =
                query_provider_shares(conn, today_start_ms, now_ms, provider, device_id)?;
            let top_model_shares = query_top_model_shares(
                conn,
                today_start_ms,
                now_ms,
                provider,
                device_id,
                today_totals.total_tokens(),
                today_totals.cost_micro_usd,
                3,
            )?;
            let active_sessions =
                query_active_sessions(conn, now_ms, provider, device_id, 8)?;
            let (latest_output_tps, live_model_speeds) =
                compute_live_model_speeds_and_tps(&active_sessions, fallback_latest);

            Ok(OverviewReport {
                total_tokens: current.total_tokens(),
                total_cost_micro_usd: current.cost_micro_usd,
                today_tokens: today_totals.total_tokens(),
                today_cost_micro_usd: today_totals.cost_micro_usd,
                today_requests: today_totals.requests,
                today_cache_hit_ratio: today_totals.cache_hit_ratio(),
                today_cache_saved_micro_usd: today_totals.cache_saved_micro_usd,
                today_tokens_delta_pct,
                today_cost_delta_pct,
                today_hourly_pulse,
                cache_hit_ratio: current.cache_hit_ratio(),
                avg_output_tps: current.avg_tps(),
                peak_output_tps: current.max_tps,
                latest_output_tps,
                live_model_speeds,
                total_requests: current.requests,
                observable_steps: current.observable_steps,
                tokens_delta_pct,
                cost_delta_pct,
                cache_delta_pct,
                tps_delta_pct,
                series_granularity,
                series,
                provider_series,
                providers: prov_rows,
                provider_shares,
                top_model_shares,
                active_sessions,
            })
        })
    }

    pub fn query_trends(&self, query: &TrendsQuery) -> CoreResult<TrendsReport> {
        self.with_conn(|conn| {
            let filter = &query.filter;
            let provider = normalized_optional_filter(&filter.provider_id);
            let device_id = normalized_optional_filter(&filter.device_id);
            let (eff_start_ms, eff_end_ms) = resolve_effective_window(
                conn,
                filter,
                provider,
                device_id,
                Utc::now().timestamp_millis(),
            )?;
            let span = inclusive_window_span(eff_start_ms, eff_end_ms);
            let effective_granularity = resolve_trend_granularity(query.granularity, span);

            let current_rows = fetch_trend_rows(
                conn,
                eff_start_ms,
                eff_end_ms,
                provider,
                device_id,
                effective_granularity,
                filter.tz_offset_minutes,
            )?;
            let previous_rows = if filter.all_time {
                Vec::new()
            } else {
                let prev_start = eff_start_ms.saturating_sub(span);
                let prev_end = eff_start_ms.saturating_sub(1);
                fetch_trend_rows(
                    conn,
                    prev_start,
                    prev_end,
                    provider,
                    device_id,
                    effective_granularity,
                    filter.tz_offset_minutes,
                )?
            };

            #[derive(Default, Clone, Copy)]
            struct TrendBucketAccum {
                tokens: i64,
                cost: i64,
                requests: i64,
                input: i64,
                cache_read: i64,
                cache_write: i64,
            }

            let mut all_buckets = BTreeSet::new();
            let mut series_map: BTreeMap<String, BTreeMap<i64, TrendBucketAccum>> =
                BTreeMap::new();
            let mut prev_totals: HashMap<String, (i64, i64)> = HashMap::new();

            let accumulate = |rows: &[RawTrendRow],
                              buckets: &mut BTreeSet<i64>,
                              map: &mut BTreeMap<String, BTreeMap<i64, TrendBucketAccum>>,
                              record_buckets: bool| {
                for row in rows {
                    if record_buckets {
                        buckets.insert(row.bucket_start_ms);
                    }
                    match query.dimension {
                        StackDimension::Provider => {
                            let tokens = row.total_tokens();
                            let entry = map
                                .entry(row.provider_id.clone())
                                .or_default()
                                .entry(row.bucket_start_ms)
                                .or_default();
                            entry.tokens += tokens;
                            entry.cost += row.cost_micro_usd;
                            entry.requests += row.requests;
                            entry.input += row.input_tokens;
                            entry.cache_read += row.cache_read_tokens;
                            entry.cache_write += row.cache_write_tokens;
                        }
                        StackDimension::Model => {
                            let tokens = row.total_tokens();
                            let entry = map
                                .entry(row.model_id.clone())
                                .or_default()
                                .entry(row.bucket_start_ms)
                                .or_default();
                            entry.tokens += tokens;
                            entry.cost += row.cost_micro_usd;
                            entry.requests += row.requests;
                            entry.input += row.input_tokens;
                            entry.cache_read += row.cache_read_tokens;
                            entry.cache_write += row.cache_write_tokens;
                        }
                        StackDimension::TokenType => {
                            let parts = [
                                ("input", row.input_tokens),
                                ("output", row.output_tokens),
                                ("cache_read", row.cache_read_tokens),
                                ("cache_write", row.cache_write_tokens),
                                ("reasoning", row.reasoning_tokens),
                            ];
                            let row_tot = row.total_tokens().max(1);
                            for (key, tok) in parts {
                                let proportional_cost = ((row.cost_micro_usd as i128
                                    * tok as i128)
                                    / row_tot as i128)
                                    as i64;
                                let entry = map
                                    .entry(key.to_string())
                                    .or_default()
                                    .entry(row.bucket_start_ms)
                                    .or_default();
                                entry.tokens += tok;
                                entry.cost += proportional_cost;
                                entry.requests += if key == "input" { row.requests } else { 0 };
                            }
                        }
                    }
                }
            };

            accumulate(&current_rows, &mut all_buckets, &mut series_map, true);

            // Generate continuous aligned buckets across the active window when data exists (capped at 2000).
            if !all_buckets.is_empty() {
                let bucket_step = granularity_bucket_ms(effective_granularity);
                let align_bucket = |ts: i64| -> i64 {
                    match effective_granularity {
                        TrendGranularity::FifteenMinutes => ts.div_euclid(900_000) * 900_000,
                        TrendGranularity::Auto | TrendGranularity::Hourly => {
                            ts.div_euclid(HOUR_MS) * HOUR_MS
                        }
                        TrendGranularity::Daily => {
                            local_day_start_ms(ts, filter.tz_offset_minutes)
                        }
                        TrendGranularity::Weekly => {
                            local_week_start_ms(ts, filter.tz_offset_minutes)
                        }
                    }
                };
                let first_b = align_bucket(eff_start_ms);
                let last_b = align_bucket(eff_end_ms);
                if last_b >= first_b && ((last_b - first_b) / bucket_step) < 2_000 {
                    let mut b = first_b;
                    while b <= last_b {
                        all_buckets.insert(b);
                        b = match b.checked_add(bucket_step) {
                            Some(next) => next,
                            None => break,
                        };
                    }
                }
            }

            let mut prev_map: BTreeMap<String, BTreeMap<i64, TrendBucketAccum>> = BTreeMap::new();
            let mut dummy_buckets = BTreeSet::new();
            accumulate(&previous_rows, &mut dummy_buckets, &mut prev_map, false);
            for (key, bucket_map) in prev_map {
                let mut tok_sum = 0i64;
                let mut cost_sum = 0i64;
                for acc in bucket_map.values() {
                    tok_sum += acc.tokens;
                    cost_sum += acc.cost;
                }
                prev_totals.insert(key, (tok_sum, cost_sum));
            }

            let buckets: Vec<i64> = all_buckets.into_iter().collect();
            let mut series = Vec::new();
            let mut comparisons = Vec::new();

            for (key, bucket_map) in series_map {
                let mut points = Vec::with_capacity(buckets.len());
                let mut total_tokens = 0i64;
                let mut total_cost = 0i64;
                let mut total_reqs = 0i64;
                let mut total_input = 0i64;
                let mut total_cache_read = 0i64;
                let mut total_cache_write = 0i64;

                for &b in &buckets {
                    let acc = bucket_map.get(&b).copied().unwrap_or_default();
                    total_tokens += acc.tokens;
                    total_cost += acc.cost;
                    total_reqs += acc.requests;
                    total_input += acc.input;
                    total_cache_read += acc.cache_read;
                    total_cache_write += acc.cache_write;
                    let denom = acc.input + acc.cache_read + acc.cache_write;
                    let cache_hit_ratio = if denom > 0 {
                        acc.cache_read as f64 / denom as f64
                    } else {
                        0.0
                    };
                    points.push(TrendPoint {
                        bucket_start_ms: b,
                        tokens: acc.tokens,
                        cost_micro_usd: acc.cost,
                        requests: acc.requests,
                        cache_hit_ratio,
                    });
                }

                let total_denom = total_input + total_cache_read + total_cache_write;
                let overall_cache_hit_ratio = if total_denom > 0 {
                    total_cache_read as f64 / total_denom as f64
                } else {
                    0.0
                };

                let label = format_dimension_label(query.dimension, &key);
                let (prev_tok, prev_cost, delta_pct, cost_delta_pct) = if filter.all_time {
                    (0, 0, None, None)
                } else {
                    let (pt, pc) = prev_totals.get(&key).copied().unwrap_or((0, 0));
                    (
                        pt,
                        pc,
                        pct_change(total_tokens as f64, pt as f64),
                        pct_change(total_cost as f64, pc as f64),
                    )
                };

                if total_tokens > 0 || prev_tok > 0 {
                    comparisons.push(TrendComparisonRow {
                        key: key.clone(),
                        label: label.clone(),
                        current_tokens: total_tokens,
                        previous_tokens: prev_tok,
                        delta_pct,
                        current_cost_micro_usd: total_cost,
                        previous_cost_micro_usd: prev_cost,
                        cost_delta_pct,
                        cache_hit_ratio: overall_cache_hit_ratio,
                        requests: total_reqs,
                    });
                }

                if total_tokens > 0 {
                    series.push(TrendSeries {
                        key,
                        label,
                        total_tokens,
                        total_cost_micro_usd: total_cost,
                        points,
                    });
                }
            }

            series.sort_by_key(|a| std::cmp::Reverse(a.total_tokens));
            comparisons.sort_by_key(|a| std::cmp::Reverse(a.current_tokens));

            Ok(TrendsReport {
                effective_granularity,
                buckets,
                series,
                comparisons,
            })
        })
    }

    pub fn query_monthly(&self, query: &MonthlyQuery) -> CoreResult<MonthlyReport> {
        self.with_conn(|conn| {
            let offset = local_offset(query.tz_offset_minutes);
            let month_start_date = NaiveDate::from_ymd_opt(query.year, query.month, 1)
                .ok_or_else(|| CoreError::Validation("Invalid year or month".to_string()))?;
            let (next_year, next_month) = if query.month == 12 {
                (query.year + 1, 1)
            } else {
                (query.year, query.month + 1)
            };
            let next_month_date =
                NaiveDate::from_ymd_opt(next_year, next_month, 1).ok_or_else(|| {
                    CoreError::Validation("Invalid next month date".to_string())
                })?;

            let start_ms = month_start_date
                .and_hms_opt(0, 0, 0)
                .and_then(|ndt| offset.from_local_datetime(&ndt).single())
                .map(|d| d.timestamp_millis())
                .ok_or_else(|| CoreError::TimeOverflow("Month start timestamp".to_string()))?;
            let end_ms = next_month_date
                .and_hms_opt(0, 0, 0)
                .and_then(|ndt| offset.from_local_datetime(&ndt).single())
                .map(|d| d.timestamp_millis() - 1)
                .ok_or_else(|| CoreError::TimeOverflow("Month end timestamp".to_string()))?;

            let provider = normalized_optional_filter(&query.provider_id);
            let device_id = normalized_optional_filter(&query.device_id);

            // Query hourly rollups for exact local-timezone day grouping.
            let rows = fetch_hourly_rows(conn, start_ms, end_ms, provider, device_id)?;

            #[derive(Default)]
            struct DayAccum {
                requests: i64,
                observable_steps: i64,
                input: i64,
                output: i64,
                cache_read: i64,
                cache_write: i64,
                reasoning: i64,
                cost: i64,
                saved: i64,
                model_tokens: HashMap<String, i64>,
            }

            let mut by_day: BTreeMap<String, DayAccum> = BTreeMap::new();
            let mut by_provider: HashMap<String, (i64, i64)> = HashMap::new();
            let mut by_model: HashMap<String, (i64, i64)> = HashMap::new();
            let mut month_cost = 0i64;
            let mut month_tokens = 0i64;
            let mut month_requests = 0i64;
            let mut month_saved = 0i64;

            for r in rows {
                let day_key = date_key_from_ms(r.bucket_start_ms, offset);
                let acc = by_day.entry(day_key).or_default();
                acc.requests += r.requests;
                acc.observable_steps += r.observable_steps;
                acc.input += r.input_tokens;
                acc.output += r.output_tokens;
                acc.cache_read += r.cache_read_tokens;
                acc.cache_write += r.cache_write_tokens;
                acc.reasoning += r.reasoning_tokens;
                acc.cost += r.cost_micro_usd;
                acc.saved += r.cache_saved_micro_usd;
                let tok = r.total_tokens();
                *acc.model_tokens.entry(r.model_id.clone()).or_default() += tok;

                let p_entry = by_provider.entry(r.provider_id).or_default();
                p_entry.0 += tok;
                p_entry.1 += r.cost_micro_usd;

                let m_entry = by_model.entry(r.model_id).or_default();
                m_entry.0 += tok;
                m_entry.1 += r.cost_micro_usd;

                month_cost += r.cost_micro_usd;
                month_tokens += tok;
                month_requests += r.requests;
                month_saved += r.cache_saved_micro_usd;
            }

            let mut provider_shares: Vec<UsageShareItem> = by_provider
                .into_iter()
                .filter(|(_, (tok, cost))| *tok > 0 || *cost > 0)
                .map(|(pid, (tokens, cost_micro_usd))| UsageShareItem {
                    label: provider_ids::display_name(&pid).to_string(),
                    key: pid,
                    tokens,
                    token_share_percent: pct_share(tokens, month_tokens),
                    cost_micro_usd,
                    cost_share_percent: pct_share(cost_micro_usd, month_cost),
                })
                .collect();
            provider_shares.sort_by(|a, b| {
                b.cost_micro_usd
                    .cmp(&a.cost_micro_usd)
                    .then_with(|| b.tokens.cmp(&a.tokens))
                    .then_with(|| a.key.cmp(&b.key))
            });

            let mut model_shares: Vec<UsageShareItem> = by_model
                .into_iter()
                .filter(|(_, (tok, cost))| *tok > 0 || *cost > 0)
                .map(|(model_id, (tokens, cost_micro_usd))| UsageShareItem {
                    label: model_id.clone(),
                    key: model_id,
                    tokens,
                    token_share_percent: pct_share(tokens, month_tokens),
                    cost_micro_usd,
                    cost_share_percent: pct_share(cost_micro_usd, month_cost),
                })
                .collect();
            model_shares.sort_by(|a, b| {
                b.cost_micro_usd
                    .cmp(&a.cost_micro_usd)
                    .then_with(|| b.tokens.cmp(&a.tokens))
                    .then_with(|| a.key.cmp(&b.key))
            });
            model_shares.truncate(6);

            let mut daily_ledger = Vec::new();
            for (date, acc) in by_day {
                let top_model = acc
                    .model_tokens
                    .into_iter()
                    .max_by_key(|(_, v)| *v)
                    .map(|(m, _)| m);
                let total_tokens =
                    acc.input + acc.output + acc.cache_read + acc.cache_write + acc.reasoning;
                daily_ledger.push(MonthlyDailyLedgerRow {
                    date,
                    requests: acc.requests,
                    observable_steps: acc.observable_steps,
                    input_tokens: acc.input,
                    output_tokens: acc.output,
                    cache_read_tokens: acc.cache_read,
                    cache_write_tokens: acc.cache_write,
                    reasoning_tokens: acc.reasoning,
                    total_tokens,
                    cost_micro_usd: acc.cost,
                    cache_saved_micro_usd: acc.saved,
                    top_model,
                });
            }

            // Active user-configured subscription monthly fee total
            let plans = load_subscription_plans(conn)?;
            let subscription_monthly_fee_micro_usd: i64 = plans
                .iter()
                .filter(|p| {
                    p.enabled
                        && p.is_user_configured
                        && p.monthly_fee_micro_usd > 0
                        && provider.is_none_or(|pid| p.provider_id.eq_ignore_ascii_case(pid))
                })
                .map(|p| p.monthly_fee_micro_usd)
                .sum();

            let roi_multiplier = if subscription_monthly_fee_micro_usd > 0 {
                Some(month_cost as f64 / subscription_monthly_fee_micro_usd as f64)
            } else {
                None
            };

            // 365-day heatmap ending at the last local day of the queried month, aligned via hourly rollups.
            let end_date = next_month_date.pred_opt().unwrap_or(month_start_date);
            let heatmap_start_date = end_date - chrono::Duration::days(364);
            let heatmap_start_str = heatmap_start_date.format("%Y-%m-%d").to_string();
            let end_date_str = end_date.format("%Y-%m-%d").to_string();
            let heatmap_start_ms = heatmap_start_date
                .and_hms_opt(0, 0, 0)
                .and_then(|ndt| offset.from_local_datetime(&ndt).single())
                .map(|d| d.timestamp_millis())
                .unwrap_or_else(|| end_ms.saturating_sub(365 * DAY_MS));

            let hm_rows = fetch_hourly_rows(conn, heatmap_start_ms, end_ms, provider, device_id)?;
            let mut hm_by_day: BTreeMap<String, DayAccum> = BTreeMap::new();
            for r in hm_rows {
                let day_key = date_key_from_ms(r.bucket_start_ms, offset);
                if day_key < heatmap_start_str || day_key > end_date_str {
                    continue;
                }
                let acc = hm_by_day.entry(day_key).or_default();
                acc.requests += r.requests;
                acc.observable_steps += r.observable_steps;
                acc.input += r.input_tokens;
                acc.output += r.output_tokens;
                acc.cache_read += r.cache_read_tokens;
                acc.cache_write += r.cache_write_tokens;
                acc.reasoning += r.reasoning_tokens;
                acc.cost += r.cost_micro_usd;
                let tok = r.total_tokens();
                *acc.model_tokens.entry(r.model_id).or_default() += tok.max(r.requests).max(1);
            }
            let heatmap_365d = hm_by_day
                .into_iter()
                .filter_map(|(date, acc)| {
                    let top_model = acc
                        .model_tokens
                        .into_iter()
                        .max_by_key(|(_, v)| *v)
                        .map(|(m, _)| m);
                    let total_tokens = acc.input
                        + acc.output
                        + acc.cache_read
                        + acc.cache_write
                        + acc.reasoning;
                    if total_tokens <= 0 && acc.requests <= 0 && acc.observable_steps <= 0 {
                        return None;
                    }
                    Some(HeatmapDayCell {
                        date,
                        total_tokens,
                        cost_micro_usd: acc.cost,
                        requests: acc.requests.max(acc.observable_steps),
                        top_model,
                    })
                })
                .collect();

            Ok(MonthlyReport {
                year: query.year,
                month: query.month,
                api_equivalent_cost_micro_usd: month_cost,
                subscription_monthly_fee_micro_usd,
                roi_multiplier,
                month_total_tokens: month_tokens,
                month_total_requests: month_requests,
                month_cache_saved_micro_usd: month_saved,
                provider_shares,
                model_shares,
                daily_ledger,
                heatmap_365d,
            })
        })
    }

    pub fn query_models_pricing(&self, filter: &AnalyticsFilter) -> CoreResult<ModelStatsReport> {
        self.with_conn(|conn| {
            let provider = normalized_optional_filter(&filter.provider_id);
            let device_id = normalized_optional_filter(&filter.device_id);
            let (eff_start_ms, eff_end_ms) = resolve_effective_window(
                conn,
                filter,
                provider,
                device_id,
                Utc::now().timestamp_millis(),
            )?;
            let catalog = load_pricing_catalog(conn)?;
            let aligned_start = eff_start_ms.div_euclid(HOUR_MS) * HOUR_MS;
            let scope = build_scope_clause(2, provider, device_id);
            let bound = bound_range_params(aligned_start, eff_end_ms, &scope.params);

            let sql = format!(
                r#"
                SELECT model_id, provider_id,
                       SUM(request_count), SUM(observable_step_count),
                       SUM(input_tokens), SUM(output_tokens),
                       SUM(cache_read_tokens), SUM(cache_write_tokens),
                       SUM(reasoning_tokens), SUM(cache_saved_micro_usd),
                       SUM(estimated_cost_micro_usd),
                       SUM(tps_sample_count), SUM(tps_sum)
                FROM usage_hourly_rollups
                WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2{where_and}
                GROUP BY model_id, provider_id
                ORDER BY SUM(estimated_cost_micro_usd) DESC, SUM(request_count) DESC
                "#,
                where_and = scope.sql_and
            );

            let mut stmt = conn.prepare(&sql)?;
            let mut raw_rows = Vec::new();
            let mut total_cost = 0i64;
            let mut total_saved = 0i64;

            let iter = stmt.query_map(params_from_iter(bound), |row| {
                let model_id: String = row.get(0)?;
                let provider_id: String = row.get(1)?;
                let requests: i64 = row.get(2)?;
                let observable_steps: i64 = row.get(3)?;
                let input_tokens: i64 = row.get(4)?;
                let output_tokens: i64 = row.get(5)?;
                let cache_read_tokens: i64 = row.get(6)?;
                let cache_write_tokens: i64 = row.get(7)?;
                let reasoning_tokens: i64 = row.get(8)?;
                let cache_saved_micro_usd: i64 = row.get(9)?;
                let estimated_cost_micro_usd: i64 = row.get(10)?;
                let tps_count: i64 = row.get(11)?;
                let tps_sum: f64 = row.get(12)?;
                let total_tokens = input_tokens
                    + output_tokens
                    + cache_read_tokens
                    + cache_write_tokens
                    + reasoning_tokens;
                let avg_output_tps = if tps_count > 0 {
                    tps_sum / tps_count as f64
                } else {
                    0.0
                };
                let effective_pricing =
                    resolve_pricing(&catalog, &provider_id, &model_id).cloned();

                Ok(ModelStatRow {
                    model_id,
                    provider_id,
                    requests,
                    observable_steps,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    reasoning_tokens,
                    total_tokens,
                    cache_saved_micro_usd,
                    estimated_cost_micro_usd,
                    cost_share_ratio: 0.0,
                    avg_output_tps,
                    effective_pricing,
                })
            })?;

            for item in iter {
                let row = item?;
                total_cost += row.estimated_cost_micro_usd;
                total_saved += row.cache_saved_micro_usd;
                raw_rows.push(row);
            }

            if total_cost > 0 {
                for row in &mut raw_rows {
                    row.cost_share_ratio =
                        row.estimated_cost_micro_usd as f64 / total_cost as f64;
                }
            }

            Ok(ModelStatsReport {
                rows: raw_rows,
                total_cost_micro_usd: total_cost,
                total_cache_saved_micro_usd: total_saved,
                catalog,
            })
        })
    }

    pub fn query_subscriptions(&self, now_ms: i64) -> CoreResult<Vec<SubscriptionPlanStatus>> {
        let tz_offset_minutes = Local::now().offset().local_minus_utc() / 60;
        self.query_subscriptions_with_tz(now_ms, tz_offset_minutes)
    }

    pub fn query_subscriptions_with_tz(
        &self,
        now_ms: i64,
        tz_offset_minutes: i32,
    ) -> CoreResult<Vec<SubscriptionPlanStatus>> {
        self.with_conn(|conn| {
            let plans = load_subscription_plans(conn)?;
            evaluate_subscriptions_with_spend(conn, plans, now_ms, tz_offset_minutes, true)
        })
    }

    pub fn query_subscription_templates(
        &self,
        now_ms: i64,
    ) -> CoreResult<Vec<crate::domain::SubscriptionPlanTemplate>> {
        self.with_conn(|conn| {
            let observed = load_observed_rate_limits(conn)?;
            let samples_map = load_window_samples_map(conn, now_ms)?;
            let templates = load_subscription_catalog_templates(conn)?;
            Ok(evaluate_subscription_templates_from_list(
                templates,
                &samples_map,
                &observed,
                now_ms,
            ))
        })
    }

    pub fn query_speed_latency(&self, filter: &AnalyticsFilter) -> CoreResult<SpeedLatencyReport> {
        self.with_conn(|conn| {
            let provider = normalized_optional_filter(&filter.provider_id);
            let device_id = normalized_optional_filter(&filter.device_id);
            let (eff_start_ms, eff_end_ms) = resolve_effective_window(
                conn,
                filter,
                provider,
                device_id,
                Utc::now().timestamp_millis(),
            )?;

            struct RawSpeedEvent {
                model_id: String,
                provider_id: String,
                output_tps: f64,
                duration_ms: Option<i64>,
                ttft_ms: Option<i64>,
            }

            let scope = build_scope_clause(2, provider, device_id);
            let bound = bound_range_params(eff_start_ms, eff_end_ms, &scope.params);
            let sql = format!(
                r#"
                SELECT model_id, provider_id, output_tps, duration_ms, ttft_ms
                FROM usage_events
                WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2{where_and}
                  AND output_tps IS NOT NULL
                  AND output_tps > 0.0
                  AND output_tps <= {MAX_VALID_OUTPUT_TPS}
                  AND duration_ms IS NOT NULL
                  AND duration_ms >= {MIN_VALID_TPS_DURATION_MS}
                  AND (output_tokens + reasoning_tokens) >= {MIN_VALID_TPS_OUTPUT_TOKENS}
                ORDER BY timestamp_ms ASC
                "#,
                where_and = scope.sql_and
            );

            let mut stmt = conn.prepare(&sql)?;
            let mut events = Vec::new();
            let rows = stmt.query_map(params_from_iter(bound), |row| {
                Ok(RawSpeedEvent {
                    model_id: row.get(0)?,
                    provider_id: row.get(1)?,
                    output_tps: row.get(2)?,
                    duration_ms: row.get(3)?,
                    ttft_ms: row.get(4)?,
                })
            })?;
            for r in rows {
                events.push(r?);
            }

            if events.is_empty() {
                return query_speed_latency_from_hourly_rollups(
                    conn,
                    eff_start_ms,
                    eff_end_ms,
                    provider,
                    device_id,
                );
            }

            // Aggregate per-provider speed & latency stats
            let mut by_provider: BTreeMap<String, Vec<&RawSpeedEvent>> = BTreeMap::new();
            let mut by_model: BTreeMap<(String, String), Vec<&RawSpeedEvent>> = BTreeMap::new();
            for ev in &events {
                by_provider
                    .entry(ev.provider_id.clone())
                    .or_default()
                    .push(ev);
                by_model
                    .entry((ev.model_id.clone(), ev.provider_id.clone()))
                    .or_default()
                    .push(ev);
            }

            let compute_group_metrics = |group: &[&RawSpeedEvent]| {
                let mut raw_sorted: Vec<f64> = group.iter().map(|e| e.output_tps).collect();
                raw_sorted.sort_by(|a, b| a.total_cmp(b));
                let upper_fence = compute_iqr_upper_fence(&raw_sorted);

                let inliers: Vec<&RawSpeedEvent> = match upper_fence {
                    Some(fence) => {
                        let filtered: Vec<&RawSpeedEvent> = group
                            .iter()
                            .copied()
                            .filter(|e| e.output_tps <= fence)
                            .collect();
                        if filtered.is_empty() {
                            group.to_vec()
                        } else {
                            filtered
                        }
                    }
                    None => group.to_vec(),
                };

                let mut tps_vals: Vec<f64> = inliers.iter().map(|e| e.output_tps).collect();
                tps_vals.sort_by(|a, b| a.total_cmp(b));
                let n = tps_vals.len();

                // Duration-weighted harmonic throughput: sum(tps_i * dur_i) / sum(dur_i) == sum(tokens_i) / sum(dur_i),
                // preventing short-duration bursts from inflating average generation speed.
                let mut weighted_tps_num = 0.0_f64;
                let mut total_weight_ms = 0.0_f64;
                for ev in &inliers {
                    let w = ev.duration_ms.unwrap_or(1_000).clamp(500, 60_000) as f64;
                    weighted_tps_num += ev.output_tps * w;
                    total_weight_ms += w;
                }
                let mean_tps = if total_weight_ms > 0.0 {
                    weighted_tps_num / total_weight_ms
                } else if n > 0 {
                    tps_vals.iter().sum::<f64>() / n as f64
                } else {
                    0.0
                };

                let p50_tps = percentile_sorted(&tps_vals, 0.50);
                let p95_tps = percentile_sorted(&tps_vals, 0.95);
                let min_tps = tps_vals.first().copied().unwrap_or(0.0);
                let max_tps = tps_vals.last().copied().unwrap_or(0.0);

                let chrono_all: Vec<f64> = group.iter().map(|e| e.output_tps).collect();
                let latest_tps = compute_ewma_tps(&chrono_all, 0.45)
                    .or_else(|| inliers.last().map(|e| e.output_tps))
                    .unwrap_or(0.0);

                let durations: Vec<i64> = inliers.iter().filter_map(|e| e.duration_ms).collect();
                let avg_duration_ms = if durations.is_empty() {
                    0.0
                } else {
                    durations.iter().sum::<i64>() as f64 / durations.len() as f64
                };
                let ttfts: Vec<i64> = inliers.iter().filter_map(|e| e.ttft_ms).collect();
                let avg_ttft_ms = if ttfts.is_empty() {
                    None
                } else {
                    Some(ttfts.iter().sum::<i64>() as f64 / ttfts.len() as f64)
                };
                (
                    group.len() as i64,
                    mean_tps,
                    p50_tps,
                    p95_tps,
                    max_tps,
                    min_tps,
                    latest_tps,
                    avg_duration_ms,
                    avg_ttft_ms,
                )
            };

            let mut providers = Vec::with_capacity(by_provider.len());
            for (provider_id, group) in by_provider {
                let (
                    sample_count,
                    mean_tps,
                    p50_tps,
                    p95_tps,
                    max_tps,
                    min_tps,
                    _latest_tps,
                    avg_duration_ms,
                    avg_ttft_ms,
                ) = compute_group_metrics(&group);
                providers.push(ProviderSpeedStats {
                    provider_id,
                    sample_count,
                    mean_tps,
                    p50_tps,
                    p95_tps,
                    max_tps,
                    min_tps,
                    avg_duration_ms,
                    avg_ttft_ms,
                });
            }
            providers.sort_by(|a, b| {
                b.sample_count
                    .cmp(&a.sample_count)
                    .then_with(|| a.provider_id.cmp(&b.provider_id))
            });

            let mut models = Vec::with_capacity(by_model.len());
            for ((model_id, provider_id), group) in by_model {
                let (
                    sample_count,
                    mean_tps,
                    p50_tps,
                    p95_tps,
                    max_tps,
                    min_tps,
                    latest_tps,
                    avg_duration_ms,
                    avg_ttft_ms,
                ) = compute_group_metrics(&group);
                models.push(ModelSpeedStats {
                    model_id,
                    provider_id,
                    sample_count,
                    mean_tps,
                    p50_tps,
                    p95_tps,
                    max_tps,
                    min_tps,
                    latest_tps,
                    avg_duration_ms,
                    avg_ttft_ms,
                });
            }
            models.sort_by(|a, b| {
                b.sample_count
                    .cmp(&a.sample_count)
                    .then_with(|| a.model_id.cmp(&b.model_id))
            });

            Ok(SpeedLatencyReport {
                providers,
                models,
                from_hourly_rollups: false,
            })
        })
    }

    pub fn query_tray_summary(&self, now_ms: i64, tz_offset_minutes: i32) -> CoreResult<TraySummary> {
        self.with_conn(|conn| {
            let (today_start_ms, _) = local_day_bounds_ms(now_ms, tz_offset_minutes);
            let today_totals = query_period_totals(conn, today_start_ms, now_ms, None, None)?;
            let fallback_latest =
                query_latest_tps_with_model(conn, today_start_ms, now_ms, None, None)?;
            let prefs = load_preferences(conn)?;

            let plans = load_subscription_plans(conn)?;
            let subscriptions = evaluate_subscriptions_with_spend(
                conn,
                plans,
                now_ms,
                tz_offset_minutes,
                false,
            )?;

            let provider_shares = query_provider_shares(conn, today_start_ms, now_ms, None, None)?;
            let active_sessions = query_active_sessions(conn, now_ms, None, None, 8)?;
            let (latest_output_tps, live_model_speeds) =
                compute_live_model_speeds_and_tps(&active_sessions, fallback_latest);

            let active_providers_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM provider_configs WHERE enabled = 1",
                [],
                |r| r.get(0),
            )?;
            let total_ingested_events: i64 =
                conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0))?;

            Ok(TraySummary {
                today_tokens: today_totals.total_tokens(),
                today_cost_micro_usd: today_totals.cost_micro_usd,
                today_requests: today_totals.requests,
                latest_output_tps,
                live_model_speeds,
                subscriptions,
                provider_shares,
                active_sessions,
                currency: prefs.currency,
                usd_to_cny_rate: prefs.usd_to_cny_rate,
                active_providers_count: active_providers_count.max(0) as usize,
                total_ingested_events,
            })
        })
    }
}

/// Export a `MonthlyReport` as deterministic RFC-4180 CSV matching displayed totals.
pub fn export_monthly_csv(report: &MonthlyReport) -> String {
    let mut lines = Vec::with_capacity(report.daily_ledger.len() + 2);
    lines.push(
        "date,requests,observable_steps,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,reasoning_tokens,total_tokens,estimated_cost_usd,cache_saved_usd,top_model".to_string(),
    );
    for row in &report.daily_ledger {
        let cost_usd = format!("{:.6}", row.cost_micro_usd as f64 / 1_000_000.0);
        let saved_usd = format!("{:.6}", row.cache_saved_micro_usd as f64 / 1_000_000.0);
        let model = row.top_model.as_deref().unwrap_or("");
        let escaped_model = if model.contains(',') || model.contains('"') {
            format!("\"{}\"", model.replace('"', "\"\""))
        } else {
            model.to_string()
        };
        lines.push(format!(
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            row.date,
            row.requests,
            row.observable_steps,
            row.input_tokens,
            row.output_tokens,
            row.cache_read_tokens,
            row.cache_write_tokens,
            row.reasoning_tokens,
            row.total_tokens,
            cost_usd,
            saved_usd,
            escaped_model
        ));
    }
    lines.join("\n")
}

/// Export a `MonthlyReport` as pretty-printed JSON.
pub fn export_monthly_json(report: &MonthlyReport) -> CoreResult<String> {
    serde_json::to_string_pretty(report).map_err(CoreError::from)
}

fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn format_dimension_label(dim: StackDimension, key: &str) -> String {
    match dim {
        StackDimension::Provider => provider_ids::display_name(key).to_string(),
        StackDimension::Model => key.to_string(),
        StackDimension::TokenType => match key {
            "input" => "Input".to_string(),
            "output" => "Output".to_string(),
            "cache_read" => "Cache Read".to_string(),
            "cache_write" => "Cache Write".to_string(),
            "reasoning" => "Reasoning".to_string(),
            other => other.to_string(),
        },
    }
}

struct RawTrendRow {
    bucket_start_ms: i64,
    provider_id: String,
    model_id: String,
    requests: i64,
    observable_steps: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    reasoning_tokens: i64,
    cost_micro_usd: i64,
    cache_saved_micro_usd: i64,
}

impl RawTrendRow {
    fn total_tokens(&self) -> i64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_tokens
            + self.cache_write_tokens
            + self.reasoning_tokens
    }
}

fn fetch_trend_rows(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
    granularity: TrendGranularity,
    tz_offset_minutes: i32,
) -> CoreResult<Vec<RawTrendRow>> {
    let resolved = resolve_trend_granularity(granularity, (end_ms - start_ms).max(HOUR_MS));
    match resolved {
        TrendGranularity::Auto => unreachable!("resolve_trend_granularity resolves Auto"),
        TrendGranularity::FifteenMinutes => {
            const BUCKET_15M: i64 = 900_000;
            let scope = build_scope_clause(2, provider, device_id);
            let sql = format!(
                r#"
                SELECT (timestamp_ms / 900000) * 900000 AS b,
                       provider_id, model_id,
                       COUNT(*), SUM(observable_only),
                       SUM(input_tokens), SUM(output_tokens),
                       SUM(cache_read_tokens), SUM(cache_write_tokens),
                       SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
                       SUM(cache_saved_micro_usd)
                FROM usage_events
                WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2{where_and}
                GROUP BY b, provider_id, model_id
                ORDER BY b ASC
                "#,
                where_and = scope.sql_and
            );
            let _ = BUCKET_15M;
            let rows = run_trend_query(conn, &sql, start_ms, end_ms, &scope.params)?;
            if rows.is_empty() {
                return fetch_hourly_rows(conn, start_ms, end_ms, provider, device_id);
            }
            Ok(rows)
        }
        TrendGranularity::Hourly => fetch_hourly_rows(conn, start_ms, end_ms, provider, device_id),
        TrendGranularity::Daily => {
            fetch_daily_rows(conn, start_ms, end_ms, provider, device_id, tz_offset_minutes)
        }
        TrendGranularity::Weekly => {
            let daily =
                fetch_daily_rows(conn, start_ms, end_ms, provider, device_id, tz_offset_minutes)?;
            let mut grouped: BTreeMap<(i64, String, String), RawTrendRow> = BTreeMap::new();
            for row in daily {
                let week_bucket = local_week_start_ms(row.bucket_start_ms, tz_offset_minutes);
                let entry = grouped
                    .entry((week_bucket, row.provider_id.clone(), row.model_id.clone()))
                    .or_insert_with(|| RawTrendRow {
                        bucket_start_ms: week_bucket,
                        provider_id: row.provider_id,
                        model_id: row.model_id,
                        requests: 0,
                        observable_steps: 0,
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                        reasoning_tokens: 0,
                        cost_micro_usd: 0,
                        cache_saved_micro_usd: 0,
                    });
                entry.requests += row.requests;
                entry.observable_steps += row.observable_steps;
                entry.input_tokens += row.input_tokens;
                entry.output_tokens += row.output_tokens;
                entry.cache_read_tokens += row.cache_read_tokens;
                entry.cache_write_tokens += row.cache_write_tokens;
                entry.reasoning_tokens += row.reasoning_tokens;
                entry.cost_micro_usd += row.cost_micro_usd;
                entry.cache_saved_micro_usd += row.cache_saved_micro_usd;
            }
            Ok(grouped.into_values().collect())
        }
    }
}

struct RollupSpeedBucket {
    model_id: String,
    provider_id: String,
    request_count: i64,
    total_duration_ms: i64,
    tps_sample_count: i64,
    tps_sum: f64,
    max_output_tps: f64,
}

fn summarize_rollup_speed_group(
    group: &[&RollupSpeedBucket],
) -> (i64, f64, f64, f64, f64, f64, f64, f64) {
    let mut bucket_tps: Vec<f64> = group
        .iter()
        .filter(|b| b.tps_sample_count > 0 && b.tps_sum > 0.0)
        .map(|b| b.tps_sum / b.tps_sample_count as f64)
        .collect();
    bucket_tps.sort_by(|a, b| a.total_cmp(b));

    let sample_count: i64 = group.iter().map(|b| b.tps_sample_count).sum();
    let total_tps_sum: f64 = group.iter().map(|b| b.tps_sum).sum();
    let mean_tps = if sample_count > 0 {
        total_tps_sum / sample_count as f64
    } else {
        0.0
    };
    let p50_tps = percentile_sorted(&bucket_tps, 0.50);
    let p95_tps = percentile_sorted(&bucket_tps, 0.95);
    let min_tps = bucket_tps.first().copied().unwrap_or(0.0);
    let max_bucket_tps = bucket_tps.last().copied().unwrap_or(0.0);
    let max_recorded_tps = group
        .iter()
        .map(|b| b.max_output_tps)
        .fold(0.0_f64, f64::max);
    let max_tps = max_recorded_tps.max(max_bucket_tps);
    let latest_tps = group
        .last()
        .filter(|b| b.tps_sample_count > 0)
        .map(|b| b.tps_sum / b.tps_sample_count as f64)
        .unwrap_or(mean_tps);

    let total_requests: i64 = group.iter().map(|b| b.request_count).sum();
    let total_duration_ms: i64 = group.iter().map(|b| b.total_duration_ms).sum();
    let avg_duration_ms = if total_requests > 0 {
        total_duration_ms as f64 / total_requests as f64
    } else if sample_count > 0 {
        total_duration_ms as f64 / sample_count as f64
    } else {
        0.0
    };

    (
        sample_count,
        mean_tps,
        p50_tps,
        p95_tps,
        max_tps,
        min_tps,
        latest_tps,
        avg_duration_ms,
    )
}

fn query_speed_latency_from_hourly_rollups(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> CoreResult<SpeedLatencyReport> {
    let aligned_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let scope = build_scope_clause(2, provider, device_id);
    let bound = bound_range_params(aligned_start, end_ms, &scope.params);
    let sql = format!(
        r#"
        SELECT model_id, provider_id,
               SUM(request_count), SUM(total_duration_ms),
               SUM(tps_sample_count), SUM(tps_sum), MAX(max_output_tps)
        FROM usage_hourly_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2{where_and}
          AND tps_sample_count > 0
          AND tps_sum > 0.0
        GROUP BY bucket_start_ms, model_id, provider_id
        ORDER BY bucket_start_ms ASC
        "#,
        where_and = scope.sql_and
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut buckets = Vec::new();
    let rows = stmt.query_map(params_from_iter(bound), |row| {
        Ok(RollupSpeedBucket {
            model_id: row.get(0)?,
            provider_id: row.get(1)?,
            request_count: row.get(2)?,
            total_duration_ms: row.get(3)?,
            tps_sample_count: row.get(4)?,
            tps_sum: row.get(5)?,
            max_output_tps: row.get(6)?,
        })
    })?;
    for r in rows {
        buckets.push(r?);
    }

    if buckets.is_empty() {
        return Ok(SpeedLatencyReport {
            providers: Vec::new(),
            models: Vec::new(),
            from_hourly_rollups: false,
        });
    }

    let mut by_provider: BTreeMap<String, Vec<&RollupSpeedBucket>> = BTreeMap::new();
    let mut by_model: BTreeMap<(String, String), Vec<&RollupSpeedBucket>> = BTreeMap::new();
    for bucket in &buckets {
        by_provider
            .entry(bucket.provider_id.clone())
            .or_default()
            .push(bucket);
        by_model
            .entry((bucket.model_id.clone(), bucket.provider_id.clone()))
            .or_default()
            .push(bucket);
    }

    let mut providers = Vec::with_capacity(by_provider.len());
    for (provider_id, group) in by_provider {
        let (
            sample_count,
            mean_tps,
            p50_tps,
            p95_tps,
            max_tps,
            min_tps,
            _latest_tps,
            avg_duration_ms,
        ) = summarize_rollup_speed_group(&group);
        providers.push(ProviderSpeedStats {
            provider_id,
            sample_count,
            mean_tps,
            p50_tps,
            p95_tps,
            max_tps,
            min_tps,
            avg_duration_ms,
            avg_ttft_ms: None,
        });
    }
    providers.sort_by(|a, b| {
        b.sample_count
            .cmp(&a.sample_count)
            .then_with(|| a.provider_id.cmp(&b.provider_id))
    });

    let mut models = Vec::with_capacity(by_model.len());
    for ((model_id, provider_id), group) in by_model {
        let (
            sample_count,
            mean_tps,
            p50_tps,
            p95_tps,
            max_tps,
            min_tps,
            latest_tps,
            avg_duration_ms,
        ) = summarize_rollup_speed_group(&group);
        models.push(ModelSpeedStats {
            model_id,
            provider_id,
            sample_count,
            mean_tps,
            p50_tps,
            p95_tps,
            max_tps,
            min_tps,
            latest_tps,
            avg_duration_ms,
            avg_ttft_ms: None,
        });
    }
    models.sort_by(|a, b| {
        b.sample_count
            .cmp(&a.sample_count)
            .then_with(|| a.model_id.cmp(&b.model_id))
    });

    Ok(SpeedLatencyReport {
        providers,
        models,
        from_hourly_rollups: true,
    })
}

fn fetch_hourly_rows(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> CoreResult<Vec<RawTrendRow>> {
    let aligned_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let scope = build_scope_clause(2, provider, device_id);
    let sql = format!(
        r#"
        SELECT bucket_start_ms, provider_id, model_id,
               SUM(request_count), SUM(observable_step_count),
               SUM(input_tokens), SUM(output_tokens),
               SUM(cache_read_tokens), SUM(cache_write_tokens),
               SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
               SUM(cache_saved_micro_usd)
        FROM usage_hourly_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2{where_and}
        GROUP BY bucket_start_ms, provider_id, model_id
        ORDER BY bucket_start_ms ASC
        "#,
        where_and = scope.sql_and
    );
    run_trend_query(conn, &sql, aligned_start, end_ms, &scope.params)
}

fn fetch_daily_rows(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
    tz_offset_minutes: i32,
) -> CoreResult<Vec<RawTrendRow>> {
    let min_day = local_day_start_ms(start_ms, tz_offset_minutes);
    let max_day = local_day_start_ms(end_ms, tz_offset_minutes);
    let aligned_hour_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let hourly_rows = fetch_hourly_rows(conn, aligned_hour_start, end_ms, provider, device_id)?;
    let mut grouped: BTreeMap<(i64, String, String), RawTrendRow> = BTreeMap::new();
    for row in hourly_rows {
        if row.bucket_start_ms < aligned_hour_start || row.bucket_start_ms > end_ms {
            continue;
        }
        let day_bucket = local_day_start_ms(row.bucket_start_ms, tz_offset_minutes);
        if day_bucket < min_day || day_bucket > max_day {
            continue;
        }
        let entry = grouped
            .entry((day_bucket, row.provider_id.clone(), row.model_id.clone()))
            .or_insert_with(|| RawTrendRow {
                bucket_start_ms: day_bucket,
                provider_id: row.provider_id,
                model_id: row.model_id,
                requests: 0,
                observable_steps: 0,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            });
        entry.requests += row.requests;
        entry.observable_steps += row.observable_steps;
        entry.input_tokens += row.input_tokens;
        entry.output_tokens += row.output_tokens;
        entry.cache_read_tokens += row.cache_read_tokens;
        entry.cache_write_tokens += row.cache_write_tokens;
        entry.reasoning_tokens += row.reasoning_tokens;
        entry.cost_micro_usd += row.cost_micro_usd;
        entry.cache_saved_micro_usd += row.cache_saved_micro_usd;
    }
    Ok(grouped.into_values().collect())
}

fn run_trend_query(
    conn: &Connection,
    sql: &str,
    start_ms: i64,
    end_ms: i64,
    extra_params: &[Value],
) -> CoreResult<Vec<RawTrendRow>> {
    let mut stmt = conn.prepare(sql)?;
    let bound = bound_range_params(start_ms, end_ms, extra_params);
    let rows = stmt.query_map(params_from_iter(bound), |row| {
        Ok(RawTrendRow {
            bucket_start_ms: row.get(0)?,
            provider_id: row.get(1)?,
            model_id: row.get(2)?,
            requests: row.get(3)?,
            observable_steps: row.get(4)?,
            input_tokens: row.get(5)?,
            output_tokens: row.get(6)?,
            cache_read_tokens: row.get(7)?,
            cache_write_tokens: row.get(8)?,
            reasoning_tokens: row.get(9)?,
            cost_micro_usd: row.get(10)?,
            cache_saved_micro_usd: row.get(11)?,
        })
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{compute_event_id, NormalizedUsageEvent};

    fn seed_sample_events(storage: &Storage) -> i64 {
        let dev = storage.local_device().expect("device").device_id;
        // 2026-11-10T10:00:00Z = 1_794_304_800_000
        let base_ts = Utc
            .with_ymd_and_hms(2026, 11, 10, 10, 0, 0)
            .single()
            .expect("valid date")
            .timestamp_millis();

        let events = vec![
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "s1", "m1", base_ts),
                device_id: dev.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "s1".to_string(),
                project_label: "app".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: base_ts,
                input_tokens: 100_000,
                output_tokens: 20_000,
                cache_read_tokens: 400_000,
                cache_write_tokens: 50_000,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(4_000),
                ttft_ms: Some(300),
                output_tps: Some(80.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            NormalizedUsageEvent {
                event_id: compute_event_id("codex", "s2", "m2", base_ts + 3_600_000),
                device_id: dev,
                provider_id: "codex".to_string(),
                session_id: "s2".to_string(),
                project_label: "cli".to_string(),
                model_id: "gpt-5-codex".to_string(),
                timestamp_ms: base_ts + 3_600_000,
                input_tokens: 50_000,
                output_tokens: 10_000,
                cache_read_tokens: 50_000,
                cache_write_tokens: 0,
                reasoning_tokens: 5_000,
                observable_only: false,
                duration_ms: Some(3_000),
                ttft_ms: None,
                output_tps: Some(50.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
        ];

        storage
            .ingest_events_with_cursor(&events, None, &[])
            .expect("ingest");
        base_ts
    }

    #[test]
    fn overview_monthly_models_and_speed_queries_align() {
        let storage = Storage::open_in_memory().expect("db");
        let base_ts = seed_sample_events(&storage);

        let filter = AnalyticsFilter {
            start_ms: base_ts - HOUR_MS,
            end_ms: base_ts + 4 * HOUR_MS,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
            all_time: false,
        };
        let ov = storage
            .query_overview(&filter, base_ts + 2 * HOUR_MS)
            .expect("overview");
        assert_eq!(ov.total_requests, 2);
        assert_eq!(ov.total_tokens, 685_000);
        assert!(ov.total_cost_micro_usd > 0);
        assert_eq!(ov.provider_series.len(), 2);
        assert_eq!(
            ov.provider_series[0].points.len(),
            ov.series.len()
        );

        let monthly = storage
            .query_monthly(&MonthlyQuery {
                year: 2026,
                month: 11,
                provider_id: None,
                device_id: None,
                tz_offset_minutes: 480,
            })
            .expect("monthly");
        assert_eq!(monthly.month_total_tokens, 685_000);
        assert_eq!(monthly.api_equivalent_cost_micro_usd, ov.total_cost_micro_usd);
        // Zero user-configured plans => zero fee & None ROI
        assert_eq!(monthly.subscription_monthly_fee_micro_usd, 0);
        assert!(monthly.roi_multiplier.is_none());
        let csv = export_monthly_csv(&monthly);
        assert!(csv.contains("2026-11-10"));

        let models = storage.query_models_pricing(&filter).expect("models");
        assert_eq!(models.rows.len(), 2);
        assert_eq!(models.total_cost_micro_usd, ov.total_cost_micro_usd);

        let speed = storage.query_speed_latency(&filter).expect("speed");
        assert_eq!(speed.providers.len(), 2);
        assert_eq!(speed.models.len(), 2);
    }

    #[test]
    fn all_time_trends_auto_observed_fallback_and_speed_outliers() {
        let storage = Storage::open_in_memory().expect("db");
        let base_ts = seed_sample_events(&storage);
        let dev = storage.local_device().expect("device").device_id;

        // Add an unphysical TPS outlier (> 800) and a short-duration step (< 300ms) + a valid observed limit
        let outliers = vec![
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "s-out", "m-fast", base_ts + 120_000),
                device_id: dev.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "s-out".to_string(),
                project_label: "app".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: base_ts + 120_000,
                input_tokens: 1_000,
                output_tokens: 500,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(150),
                ttft_ms: None,
                output_tps: Some(3_333.3),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            NormalizedUsageEvent {
                event_id: compute_event_id("codex", "s-old", "m-old", base_ts - 5 * DAY_MS),
                device_id: dev,
                provider_id: "codex".to_string(),
                session_id: "s-old".to_string(),
                project_label: "cli".to_string(),
                model_id: "gpt-5-codex".to_string(),
                timestamp_ms: base_ts - 5 * DAY_MS,
                input_tokens: 10_000,
                output_tokens: 5_000,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(2_000),
                ttft_ms: Some(200),
                output_tps: Some(60.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
        ];
        let observed = vec![ObservedRateLimitWindow {
            provider_id: "codex".to_string(),
            window_name: "primary".to_string(),
            used_percent: 42.5,
            window_minutes: Some(300),
            resets_at_ms: Some(base_ts + 4 * HOUR_MS),
            observed_at_ms: base_ts,
            source: Some("local_log".to_string()),
            plan_hint: Some("plus".to_string()),
        }];
        storage
            .ingest_events_with_cursor(&outliers, None, &observed)
            .expect("ingest outliers");

        let all_time_filter = AnalyticsFilter {
            start_ms: base_ts,
            end_ms: base_ts + 2 * HOUR_MS,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
            all_time: true,
        };
        let ov = storage
            .query_overview(&all_time_filter, base_ts + HOUR_MS)
            .expect("all_time overview");
        assert!(ov.tokens_delta_pct.is_none());
        assert!(ov.cost_delta_pct.is_none());
        assert!(ov.cache_delta_pct.is_none());
        assert!(ov.tps_delta_pct.is_none());
        let codex_row = ov
            .providers
            .iter()
            .find(|p| p.provider_id == "codex")
            .expect("codex row");
        assert_eq!(codex_row.active_window_label.as_deref(), Some("5h 滚动窗口"));
        assert_eq!(codex_row.active_window_used_percent, Some(42.5));

        let trends = storage
            .query_trends(&TrendsQuery {
                filter: all_time_filter.clone(),
                granularity: TrendGranularity::Auto,
                dimension: StackDimension::Provider,
            })
            .expect("all_time trends");
        assert_eq!(trends.effective_granularity, TrendGranularity::Daily);
        assert!(trends.buckets.len() >= 5);
        assert!(trends.comparisons.iter().all(|c| c.delta_pct.is_none() && c.previous_tokens == 0));

        let speed = storage
            .query_speed_latency(&all_time_filter)
            .expect("speed filtered");
        assert!(speed.providers.iter().all(|p| p.max_tps <= 450.0));
        assert!(speed.models.iter().all(|m| m.max_tps <= 450.0));
    }

    #[test]
    fn heatmap_365d_aligns_with_daily_ledger_and_includes_observable_only() {
        let storage = Storage::open_in_memory().expect("db");
        let dev = storage.local_device().expect("device").device_id;

        // 2026-11-15T23:30:00Z:
        //   - UTC+8 (+480): 2026-11-16 07:30 (+8) -> must appear in 2026-11-16
        //   - UTC-5 (-300): 2026-11-15 18:30 (-5) -> must appear in 2026-11-15
        let cross_boundary_ts = Utc
            .with_ymd_and_hms(2026, 11, 15, 23, 30, 0)
            .single()
            .expect("ts")
            .timestamp_millis();

        // Observable-only Antigravity event on 2026-11-12T10:00:00Z with 0 tokens
        let obs_ts = Utc
            .with_ymd_and_hms(2026, 11, 12, 10, 0, 0)
            .single()
            .expect("ts")
            .timestamp_millis();

        let events = vec![
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "s-tz", "m1", cross_boundary_ts),
                device_id: dev.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "s-tz".to_string(),
                project_label: "app".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: cross_boundary_ts,
                input_tokens: 12_000,
                output_tokens: 3_000,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_000),
                ttft_ms: None,
                output_tps: Some(30.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            NormalizedUsageEvent {
                event_id: compute_event_id("antigravity", "s-obs", "m2", obs_ts),
                device_id: dev,
                provider_id: "antigravity".to_string(),
                session_id: "s-obs".to_string(),
                project_label: "agt".to_string(),
                model_id: "antigravity-default".to_string(),
                timestamp_ms: obs_ts,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: true,
                duration_ms: None,
                ttft_ms: None,
                output_tps: None,
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
        ];
        storage
            .ingest_events_with_cursor(&events, None, &[])
            .expect("ingest");

        // Verify UTC+8
        let m_plus8 = storage
            .query_monthly(&MonthlyQuery {
                year: 2026,
                month: 11,
                provider_id: None,
                device_id: None,
                tz_offset_minutes: 480,
            })
            .expect("monthly +8");
        assert_eq!(m_plus8.daily_ledger.len(), 2);
        assert_eq!(m_plus8.heatmap_365d.len(), 2);
        for ledger in &m_plus8.daily_ledger {
            let cell = m_plus8
                .heatmap_365d
                .iter()
                .find(|c| c.date == ledger.date)
                .expect("matching heatmap date");
            assert_eq!(cell.total_tokens, ledger.total_tokens);
            assert_eq!(cell.requests, ledger.requests);
        }
        assert!(m_plus8.heatmap_365d.iter().any(|c| c.date == "2026-11-16"));
        let obs_cell = m_plus8
            .heatmap_365d
            .iter()
            .find(|c| c.date == "2026-11-12")
            .expect("observable-only day present in heatmap");
        assert_eq!(obs_cell.total_tokens, 0);
        assert_eq!(obs_cell.requests, 1);

        // Verify UTC-5
        let m_minus5 = storage
            .query_monthly(&MonthlyQuery {
                year: 2026,
                month: 11,
                provider_id: None,
                device_id: None,
                tz_offset_minutes: -300,
            })
            .expect("monthly -5");
        assert!(m_minus5
            .heatmap_365d
            .iter()
            .any(|c| c.date == "2026-11-15" && c.total_tokens == 15_000));
    }

    #[test]
    fn overview_and_trends_exclude_zero_token_series() {
        let storage = Storage::open_in_memory().expect("db");
        let dev = storage.local_device().expect("device").device_id;
        let ts = Utc
            .with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
            .single()
            .expect("ts")
            .timestamp_millis();

        // Ingest claude_code with real tokens (0 reasoning, 0 cache_write) + antigravity observable_only (0 tokens)
        let events = vec![
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "s1", "m1", ts),
                device_id: dev.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "s1".to_string(),
                project_label: "app".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: ts,
                input_tokens: 1_000,
                output_tokens: 200,
                cache_read_tokens: 500,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_000),
                ttft_ms: None,
                output_tps: Some(20.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            NormalizedUsageEvent {
                event_id: compute_event_id("antigravity", "s2", "m2", ts + 1_000),
                device_id: dev,
                provider_id: "antigravity".to_string(),
                session_id: "s2".to_string(),
                project_label: "agt".to_string(),
                model_id: "antigravity-default".to_string(),
                timestamp_ms: ts + 1_000,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: true,
                duration_ms: None,
                ttft_ms: None,
                output_tps: None,
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
        ];
        storage
            .ingest_events_with_cursor(&events, None, &[])
            .expect("ingest");

        let filter = AnalyticsFilter {
            start_ms: ts - HOUR_MS,
            end_ms: ts + HOUR_MS,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
            all_time: false,
        };
        let ov = storage.query_overview(&filter, ts + HOUR_MS).expect("overview");
        assert_eq!(ov.provider_series.len(), 1);
        assert_eq!(ov.provider_series[0].key, "claude_code");

        let prov_trends = storage
            .query_trends(&TrendsQuery {
                filter: filter.clone(),
                granularity: TrendGranularity::Hourly,
                dimension: StackDimension::Provider,
            })
            .expect("provider trends");
        assert_eq!(prov_trends.series.len(), 1);
        assert_eq!(prov_trends.series[0].key, "claude_code");

        let tok_trends = storage
            .query_trends(&TrendsQuery {
                filter,
                granularity: TrendGranularity::Hourly,
                dimension: StackDimension::TokenType,
            })
            .expect("token type trends");
        assert!(tok_trends.series.iter().all(|s| s.total_tokens > 0));
        assert!(tok_trends.series.iter().all(|s| s.key != "reasoning"));
    }

    #[test]
    fn active_sessions_and_shares_classify_active_vs_recent_and_sanitize_labels() {
        let storage = Storage::open_in_memory().expect("db");
        let dev = storage.local_device().expect("device").device_id;
        let now_ms = Utc
            .with_ymd_and_hms(2026, 11, 16, 12, 0, 0)
            .single()
            .expect("now")
            .timestamp_millis();

        let events = vec![
            // Active (45s ago) + earlier step 2h ago in same session
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "sess-alpha", "m1", now_ms - 7_200_000),
                device_id: dev.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "sess-alpha".to_string(),
                project_label: "C:\\Users\\alice\\repos\\my-secret-app".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: now_ms - 7_200_000,
                input_tokens: 1_000,
                output_tokens: 200,
                cache_read_tokens: 400,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_500),
                ttft_ms: None,
                output_tps: Some(45.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "sess-alpha", "m2", now_ms - 45_000),
                device_id: dev.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "sess-alpha".to_string(),
                project_label: "C:\\Users\\alice\\repos\\my-secret-app\\".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: now_ms - 45_000,
                input_tokens: 2_000,
                output_tokens: 300,
                cache_read_tokens: 600,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_000),
                ttft_ms: None,
                output_tps: Some(82.5),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            // Recent (10 min ago)
            NormalizedUsageEvent {
                event_id: compute_event_id("kimi_code", "sess-beta", "m1", now_ms - 600_000),
                device_id: dev.clone(),
                provider_id: "kimi_code".to_string(),
                session_id: "sess-beta".to_string(),
                project_label: "/home/dev/worktree/backend-api".to_string(),
                model_id: "kimi-k2-0905-preview".to_string(),
                timestamp_ms: now_ms - 600_000,
                input_tokens: 500,
                output_tokens: 150,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(2_000),
                ttft_ms: None,
                output_tps: Some(55.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            // Too old (20 min ago) -> excluded from active_sessions
            NormalizedUsageEvent {
                event_id: compute_event_id("codex", "sess-gamma", "m1", now_ms - 1_200_000),
                device_id: dev,
                provider_id: "codex".to_string(),
                session_id: "sess-gamma".to_string(),
                project_label: "old-project".to_string(),
                model_id: "gpt-5-codex".to_string(),
                timestamp_ms: now_ms - 1_200_000,
                input_tokens: 300,
                output_tokens: 100,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_000),
                ttft_ms: None,
                output_tps: Some(60.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
        ];
        storage
            .ingest_events_with_cursor(&events, None, &[])
            .expect("ingest");

        let ov = storage
            .query_overview(
                &AnalyticsFilter {
                    start_ms: now_ms - 6 * HOUR_MS,
                    end_ms: now_ms,
                    provider_id: None,
                    device_id: None,
                    tz_offset_minutes: 480,
                    all_time: false,
                },
                now_ms,
            )
            .expect("overview");

        assert_eq!(ov.active_sessions.len(), 2);
        let s0 = &ov.active_sessions[0];
        assert_eq!(s0.provider_id, "claude_code");
        assert_eq!(s0.activity_state, "active");
        assert_eq!(s0.project_label, "my-secret-app");
        assert!(s0.session_key.starts_with('#') && s0.session_key.len() == 7);
        assert_ne!(s0.session_key, "sess-alpha");
        assert_eq!(s0.request_count, 2);
        assert_eq!(s0.total_tokens, 4_500);
        assert_eq!(s0.latest_output_tps, Some(82.5));

        let s1 = &ov.active_sessions[1];
        assert_eq!(s1.provider_id, "kimi_code");
        assert_eq!(s1.activity_state, "recent");
        assert_eq!(s1.project_label, "backend-api");

        assert_eq!(ov.provider_shares.len(), 3);
        let pct_sum: f64 = ov
            .provider_shares
            .iter()
            .map(|p| p.token_share_percent)
            .sum();
        assert!((pct_sum - 100.0).abs() < 1e-6);
        assert_eq!(ov.top_model_shares.len(), 3);

        let tray = storage.query_tray_summary(now_ms, 480).expect("tray");
        assert_eq!(tray.active_sessions.len(), 2);
        assert_eq!(tray.provider_shares.len(), 3);
    }

    #[test]
    fn multi_device_filter_isolates_overview_trends_monthly_and_models() {
        use crate::domain::{SyncEnvelopeV1, SyncHourlyDelta};

        let storage = Storage::open_in_memory().expect("db");
        let base_ts = seed_sample_events(&storage);
        let bucket = base_ts.div_euclid(HOUR_MS) * HOUR_MS;
        let remote_envelope = SyncEnvelopeV1 {
            schema_version: 1,
            device_id: "remote-mbp".to_string(),
            device_display_name: "MacBook Pro".to_string(),
            generated_at_ms: base_ts + 5_000,
            since_watermark_ms: 0,
            hourly_deltas: vec![SyncHourlyDelta {
                provider_id: "deepseek_harness".to_string(),
                model_id: "deepseek-reasoner".to_string(),
                bucket_start_ms: bucket,
                request_count: 3,
                observable_step_count: 0,
                input_tokens: 20_000,
                output_tokens: 10_000,
                cache_read_tokens: 5_000,
                cache_write_tokens: 0,
                reasoning_tokens: 15_000,
                estimated_cost_micro_usd: 420_000,
                cache_saved_micro_usd: 10_000,
                total_duration_ms: 9_000,
                tps_sample_count: 3,
                tps_sum: 150.0,
                max_output_tps: 60.0,
                updated_at_ms: base_ts + 5_000,
            }],
            subscription_snapshots: Vec::new(),
        };
        storage
            .apply_remote_sync_envelope_v1(&remote_envelope, base_ts + 6_000)
            .expect("apply remote");

        // All devices = local (685k) + remote (50k) = 735k
        let all_filter = AnalyticsFilter {
            start_ms: base_ts - HOUR_MS,
            end_ms: base_ts + 4 * HOUR_MS,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
            all_time: false,
        };
        let all_ov = storage
            .query_overview(&all_filter, base_ts + 2 * HOUR_MS)
            .expect("all overview");
        assert_eq!(all_ov.total_tokens, 735_000);

        // Single remote device = 50k tokens
        let remote_filter = AnalyticsFilter {
            device_id: Some("remote-mbp".to_string()),
            ..all_filter.clone()
        };
        let remote_ov = storage
            .query_overview(&remote_filter, base_ts + 2 * HOUR_MS)
            .expect("remote overview");
        assert_eq!(remote_ov.total_tokens, 50_000);
        assert_eq!(remote_ov.total_cost_micro_usd, 420_000);
        assert_eq!(remote_ov.active_sessions.len(), 0);

        let remote_trends = storage
            .query_trends(&TrendsQuery {
                filter: remote_filter.clone(),
                granularity: TrendGranularity::Hourly,
                dimension: StackDimension::Provider,
            })
            .expect("remote trends");
        assert_eq!(remote_trends.series.len(), 1);
        assert_eq!(remote_trends.series[0].key, "deepseek_harness");

        let remote_monthly = storage
            .query_monthly(&MonthlyQuery {
                year: 2026,
                month: 11,
                provider_id: None,
                device_id: Some("remote-mbp".to_string()),
                tz_offset_minutes: 480,
            })
            .expect("remote monthly");
        assert_eq!(remote_monthly.month_total_tokens, 50_000);

        let remote_models = storage
            .query_models_pricing(&remote_filter)
            .expect("remote models");
        assert_eq!(remote_models.rows.len(), 1);
        assert_eq!(remote_models.rows[0].model_id, "deepseek-reasoner");

        // Pure-rollup remote device must also support speed latency, latest TPS, and 15m trend fallback.
        let remote_speed = storage
            .query_speed_latency(&remote_filter)
            .expect("remote speed latency");
        assert!(remote_speed.from_hourly_rollups);
        assert_eq!(remote_speed.providers.len(), 1);
        assert_eq!(remote_speed.providers[0].sample_count, 3);
        assert!((remote_speed.providers[0].mean_tps - 50.0).abs() < 1e-6);
        assert!((remote_speed.providers[0].max_tps - 60.0).abs() < 1e-6);
        assert_eq!(remote_speed.providers[0].avg_ttft_ms, None);
        assert_eq!(remote_speed.models.len(), 1);
        assert_eq!(remote_speed.models[0].model_id, "deepseek-reasoner");

        assert!(remote_ov.latest_output_tps.is_some());
        assert!((remote_ov.latest_output_tps.expect("latest tps") - 50.0).abs() < 1e-6);

        let remote_15m_trends = storage
            .query_trends(&TrendsQuery {
                filter: remote_filter,
                granularity: TrendGranularity::FifteenMinutes,
                dimension: StackDimension::Provider,
            })
            .expect("remote 15m fallback trends");
        assert_eq!(remote_15m_trends.series.len(), 1);
        assert_eq!(remote_15m_trends.series[0].key, "deepseek_harness");

        // Empty query should report from_hourly_rollups = false cleanly.
        let empty_speed = storage
            .query_speed_latency(&AnalyticsFilter {
                device_id: Some("non-existent-device".to_string()),
                ..all_filter
            })
            .expect("empty speed latency");
        assert!(!empty_speed.from_hourly_rollups);
        assert!(empty_speed.providers.is_empty());
        assert!(empty_speed.models.is_empty());
    }

    #[test]
    fn overview_today_shares_remain_fixed_across_time_ranges() {
        let storage = Storage::open_in_memory().expect("in-memory");
        let dev_id = storage.local_device().expect("local device").device_id;
        let now_ms = 1_794_800_000_000i64; // 2026-11-15 in UTC+8
        let (today_start_ms, _) = local_day_bounds_ms(now_ms, 480);
        let today_ts = today_start_ms + HOUR_MS;
        let yesterday_ts = today_start_ms - 4 * HOUR_MS;

        let events = vec![
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "s-today", "m-today", today_ts),
                device_id: dev_id.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "s-today".to_string(),
                project_label: "proj".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: today_ts,
                input_tokens: 1_000,
                output_tokens: 500,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_000),
                ttft_ms: None,
                output_tps: Some(50.0),
                estimated_cost_micro_usd: 15_000,
                cache_saved_micro_usd: 0,
            },
            NormalizedUsageEvent {
                event_id: compute_event_id("codex", "s-yesterday", "m-yday", yesterday_ts),
                device_id: dev_id,
                provider_id: "codex".to_string(),
                session_id: "s-yesterday".to_string(),
                project_label: "proj".to_string(),
                model_id: "gpt-5.3-codex".to_string(),
                timestamp_ms: yesterday_ts,
                input_tokens: 9_000,
                output_tokens: 1_000,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(2_000),
                ttft_ms: None,
                output_tps: Some(50.0),
                estimated_cost_micro_usd: 90_000,
                cache_saved_micro_usd: 0,
            },
        ];
        storage
            .ingest_events_with_cursor(&events, None, &[])
            .expect("ingest");

        let today_filter = AnalyticsFilter {
            start_ms: today_start_ms,
            end_ms: now_ms,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
            all_time: false,
        };
        let range_30d_filter = AnalyticsFilter {
            start_ms: now_ms - 30 * 24 * HOUR_MS,
            end_ms: now_ms,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
            all_time: false,
        };

        let ov_today = storage.query_overview(&today_filter, now_ms).expect("today");
        let ov_30d = storage.query_overview(&range_30d_filter, now_ms).expect("30d");

        // Trend totals change between today vs 30d
        assert_eq!(ov_today.total_tokens, 1_500);
        assert_eq!(ov_30d.total_tokens, 11_500);

        // Today live pulse share cards stay pinned to today regardless of filter range
        assert_eq!(ov_today.provider_shares.len(), 1);
        assert_eq!(ov_today.provider_shares[0].key, "claude_code");
        assert_eq!(ov_today.provider_shares[0].tokens, 1_500);
        assert_eq!(ov_30d.provider_shares, ov_today.provider_shares);
        assert_eq!(ov_30d.top_model_shares, ov_today.top_model_shares);
        assert_eq!(ov_30d.top_model_shares.len(), 1);
        assert_eq!(ov_30d.top_model_shares[0].key, "claude-sonnet-4-5");
    }

    #[test]
    fn overview_series_zero_fills_gaps_and_active_sessions_resolve_titles() {
        let storage = Storage::open_in_memory().expect("in-memory");
        let dev_id = storage.local_device().expect("device").device_id;
        let now_ms = 1_794_830_400_000i64; // aligned hour boundary in Nov 2026
        let h0 = now_ms - 3 * HOUR_MS;
        let h3 = now_ms - 10_000;

        let events = vec![
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "sess-uuid-1", "m1", h0),
                device_id: dev_id.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "sess-uuid-1".to_string(),
                project_label: "TokenVane".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: h0,
                input_tokens: 1_000,
                output_tokens: 120,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(2_000),
                ttft_ms: None,
                output_tps: Some(60.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "sess-uuid-1", "m2", h3),
                device_id: dev_id,
                provider_id: "claude_code".to_string(),
                session_id: "sess-uuid-1".to_string(),
                project_label: "TokenVane".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: h3,
                input_tokens: 2_000,
                output_tokens: 200,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(2_500),
                ttft_ms: None,
                output_tps: Some(80.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
        ];
        let titles = vec![crate::domain::SessionTitleRecord {
            session_key: "claude_code:sess-uuid-1".to_string(),
            provider_id: "claude_code".to_string(),
            title: "Refactor Chart Stacking".to_string(),
            source: "explicit".to_string(),
            updated_at_ms: h3,
        }];
        storage
            .ingest_events_with_cursor_and_titles(&events, None, &[], &titles)
            .expect("ingest");

        let filter = AnalyticsFilter {
            start_ms: h0,
            end_ms: now_ms,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
            all_time: false,
        };
        let ov = storage.query_overview(&filter, now_ms).expect("overview");
        // Continuous hourly buckets from h0 through now_ms: 4 buckets with zeros in middle gaps
        assert_eq!(ov.series.len(), 4);
        assert_eq!(ov.series[1].input_tokens, 0);
        assert_eq!(ov.series[1].output_tokens, 0);
        assert_eq!(ov.provider_series[0].points.len(), ov.series.len());
        assert_eq!(ov.active_sessions.len(), 1);
        assert_eq!(
            ov.active_sessions[0].session_title.as_deref(),
            Some("Refactor Chart Stacking")
        );
    }

    #[test]
    fn active_sessions_and_live_tps_exclude_background_guardian_review_sessions() {
        let storage = Storage::open_in_memory().expect("in-memory");
        let dev_id = storage.local_device().expect("device").device_id;
        let now_ms = 1_794_830_400_000i64;

        let events = vec![
            NormalizedUsageEvent {
                event_id: compute_event_id("codex", "sess-user-real", "step-1", now_ms - 15_000),
                device_id: dev_id.clone(),
                provider_id: "codex".to_string(),
                session_id: "sess-user-real".to_string(),
                project_label: "TokenVane".to_string(),
                model_id: "gpt-5.4".to_string(),
                timestamp_ms: now_ms - 15_000,
                input_tokens: 2_000,
                output_tokens: 300,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(3_000),
                ttft_ms: None,
                output_tps: Some(100.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            // Background Guardian auto-review event (newer timestamp, high token rate)
            NormalizedUsageEvent {
                event_id: compute_event_id("codex", "sess-guardian-bg", "step-bg", now_ms - 2_000),
                device_id: dev_id,
                provider_id: "codex".to_string(),
                session_id: "sess-guardian-bg".to_string(),
                project_label: "TokenVane".to_string(),
                model_id: "codex-auto-review".to_string(),
                timestamp_ms: now_ms - 2_000,
                input_tokens: 1_500,
                output_tokens: 250,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_000),
                ttft_ms: None,
                output_tps: Some(250.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
        ];
        let titles = vec![
            crate::domain::SessionTitleRecord {
                session_key: "codex:sess-user-real".to_string(),
                provider_id: "codex".to_string(),
                title: "Implement v0.2.6 features".to_string(),
                source: "user_prompt".to_string(),
                updated_at_ms: now_ms - 15_000,
            },
            crate::domain::SessionTitleRecord {
                session_key: "codex:sess-guardian-bg".to_string(),
                provider_id: "codex".to_string(),
                title: "Guardian review".to_string(),
                source: "background".to_string(),
                updated_at_ms: now_ms - 2_000,
            },
        ];
        storage
            .ingest_events_with_cursor_and_titles(&events, None, &[], &titles)
            .expect("ingest");

        let ov = storage
            .query_overview(
                &AnalyticsFilter {
                    start_ms: now_ms - HOUR_MS,
                    end_ms: now_ms,
                    provider_id: None,
                    device_id: None,
                    tz_offset_minutes: 480,
                    all_time: false,
                },
                now_ms,
            )
            .expect("overview");

        assert_eq!(ov.active_sessions.len(), 1);
        assert_eq!(
            ov.active_sessions[0].session_title.as_deref(),
            Some("Implement v0.2.6 features")
        );
        assert_eq!(ov.active_sessions[0].model_id, "gpt-5.4");
        // Live TPS fallback & active speed must reflect user session (100 tok/s), NOT 250 tok/s auto-review.
        assert_eq!(ov.latest_output_tps, Some(100.0));
        assert!(!ov.live_model_speeds.iter().any(|s| s.model_id == "codex-auto-review"));
    }

    #[test]
    fn trends_daily_buckets_and_comparisons_align_to_local_timezone() {
        let storage = Storage::open_in_memory().expect("in-memory");
        let dev_id = storage.local_device().expect("device").device_id;
        let offset_plus8 = local_offset(480);

        // Window: 2026-11-11 00:00:00.000 +08:00 through 2026-11-17 23:59:59.999 +08:00 (7 calendar days)
        let start_ms = NaiveDate::from_ymd_opt(2026, 11, 11)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .and_then(|ndt| offset_plus8.from_local_datetime(&ndt).single())
            .unwrap()
            .timestamp_millis();
        let end_ms = NaiveDate::from_ymd_opt(2026, 11, 17)
            .unwrap()
            .and_hms_milli_opt(23, 59, 59, 999)
            .and_then(|ndt| offset_plus8.from_local_datetime(&ndt).single())
            .unwrap()
            .timestamp_millis();

        // Event 1: 2026-11-11 01:00 +08:00 (= 2026-11-10 17:00 UTC) -> MUST land in 2026-11-11 +08:00
        let ts_nov11_early = start_ms + HOUR_MS;
        // Event 2: 2026-11-17 03:36 +08:00 -> MUST land in 2026-11-17 +08:00
        let ts_nov17 = start_ms + 6 * DAY_MS + 3 * HOUR_MS + 36 * 60_000;
        // Event 3: 2026-11-10 23:30 +08:00 -> MUST belong ONLY to previous 7-day period (2026-11-04..2026-11-10)
        let ts_nov10_late = start_ms - 30 * 60_000;

        let events = vec![
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "s-11", "m1", ts_nov11_early),
                device_id: dev_id.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "s-11".to_string(),
                project_label: "proj".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: ts_nov11_early,
                input_tokens: 1_000,
                output_tokens: 500,
                cache_read_tokens: 3_000,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_000),
                ttft_ms: None,
                output_tps: Some(50.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "s-17", "m2", ts_nov17),
                device_id: dev_id.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "s-17".to_string(),
                project_label: "proj".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: ts_nov17,
                input_tokens: 2_000,
                output_tokens: 1_000,
                cache_read_tokens: 2_000,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_500),
                ttft_ms: None,
                output_tps: Some(60.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "s-10", "m3", ts_nov10_late),
                device_id: dev_id,
                provider_id: "claude_code".to_string(),
                session_id: "s-10".to_string(),
                project_label: "proj".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: ts_nov10_late,
                input_tokens: 1_500,
                output_tokens: 500,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_000),
                ttft_ms: None,
                output_tps: Some(40.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
        ];
        storage
            .ingest_events_with_cursor(&events, None, &[])
            .expect("ingest");

        let filter = AnalyticsFilter {
            start_ms,
            end_ms,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
            all_time: false,
        };

        let trends = storage
            .query_trends(&TrendsQuery {
                filter: filter.clone(),
                granularity: TrendGranularity::Auto,
                dimension: StackDimension::Provider,
            })
            .expect("trends");

        assert_eq!(trends.effective_granularity, TrendGranularity::Daily);
        assert_eq!(trends.buckets.len(), 7);
        assert_eq!(trends.buckets[0], start_ms);
        assert_eq!(trends.buckets[6], start_ms + 6 * DAY_MS);

        let claude = &trends.series[0];
        assert_eq!(claude.points[0].tokens, 4_500);
        // 3000 / (1000 + 3000) = 0.75
        assert!((claude.points[0].cache_hit_ratio - 0.75).abs() < 1e-6);
        assert_eq!(claude.points[6].tokens, 5_000);
        // 2000 / (2000 + 2000) = 0.50
        assert!((claude.points[6].cache_hit_ratio - 0.50).abs() < 1e-6);

        let cmp = &trends.comparisons[0];
        assert_eq!(cmp.current_tokens, 9_500);
        assert_eq!(cmp.previous_tokens, 2_000);
        // Overall cache hit ratio: 5000 / (3000 + 5000) = 0.625
        assert!((cmp.cache_hit_ratio - 0.625).abs() < 1e-6);
        assert!(cmp.cost_delta_pct.is_some());

        let ov = storage.query_overview(&filter, ts_nov17).expect("overview");
        assert_eq!(ov.today_hourly_pulse.len(), 24);
        assert_eq!(ov.today_requests, 1);
        assert_eq!(ov.today_tokens, 5_000);
        assert!((ov.today_cache_hit_ratio - 0.50).abs() < 1e-6);
        assert_eq!(ov.today_hourly_pulse[3].input_tokens, 2_000);

        let monthly = storage
            .query_monthly(&MonthlyQuery {
                year: 2026,
                month: 11,
                provider_id: None,
                device_id: None,
                tz_offset_minutes: 480,
            })
            .expect("monthly");
        assert_eq!(monthly.provider_shares.len(), 1);
        assert_eq!(monthly.provider_shares[0].key, "claude_code");
        assert_eq!(monthly.model_shares.len(), 1);
        assert_eq!(monthly.model_shares[0].key, "claude-sonnet-4-5");
    }
}
