use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{Datelike, FixedOffset, NaiveDate, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::domain::{provider_ids, CurrencyCode, ModelPricingEntry, ObservedRateLimitWindow};
use crate::error::{CoreError, CoreResult};
use crate::pricing::resolve_pricing;
use crate::storage::repository::{
    load_observed_rate_limits, load_preferences, load_pricing_catalog, load_subscription_plans,
    load_window_samples_map, DAY_MS, HOUR_MS,
};
use crate::storage::Storage;
use crate::subscription::{evaluate_subscription_plan, SubscriptionPlanStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyticsFilter {
    pub start_ms: i64,
    pub end_ms: i64,
    pub provider_id: Option<String>,
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
pub struct OverviewReport {
    pub total_tokens: i64,
    pub total_cost_micro_usd: i64,
    pub cache_hit_ratio: f64,
    pub avg_output_tps: f64,
    pub peak_output_tps: f64,
    pub latest_output_tps: Option<f64>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendPoint {
    pub bucket_start_ms: i64,
    pub tokens: i64,
    pub cost_micro_usd: i64,
    pub requests: i64,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraySummary {
    pub today_tokens: i64,
    pub today_cost_micro_usd: i64,
    pub today_requests: i64,
    pub latest_output_tps: Option<f64>,
    pub subscriptions: Vec<SubscriptionPlanStatus>,
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

fn normalized_provider_filter(provider_id: &Option<String>) -> Option<&str> {
    provider_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
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

fn local_day_bounds_ms(now_ms: i64, tz_offset_minutes: i32) -> (i64, i64) {
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
) -> CoreResult<PeriodTotals> {
    let (sql, use_provider) = if provider.is_some() {
        (
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
                COALESCE(SUM(tps_sample_count), 0),
                COALESCE(SUM(tps_sum), 0.0),
                COALESCE(MAX(max_output_tps), 0.0)
            FROM usage_hourly_rollups
            WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2 AND provider_id = ?3
            "#,
            true,
        )
    } else {
        (
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
                COALESCE(SUM(tps_sample_count), 0),
                COALESCE(SUM(tps_sum), 0.0),
                COALESCE(MAX(max_output_tps), 0.0)
            FROM usage_hourly_rollups
            WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2
            "#,
            false,
        )
    };

    // Align lower bound down to hour bucket boundary so exact hour-bucketed rollups match event windows.
    let aligned_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<PeriodTotals> {
        Ok(PeriodTotals {
            requests: row.get(0)?,
            observable_steps: row.get(1)?,
            input: row.get(2)?,
            output: row.get(3)?,
            cache_read: row.get(4)?,
            cache_write: row.get(5)?,
            reasoning: row.get(6)?,
            cost_micro_usd: row.get(7)?,
            tps_count: row.get(8)?,
            tps_sum: row.get(9)?,
            max_tps: row.get(10)?,
        })
    };

    if use_provider {
        conn.query_row(
            sql,
            params![aligned_start, end_ms, provider.unwrap_or_default()],
            map_row,
        )
        .map_err(CoreError::from)
    } else {
        conn.query_row(sql, params![aligned_start, end_ms], map_row)
            .map_err(CoreError::from)
    }
}

fn resolve_effective_window(
    conn: &Connection,
    filter: &AnalyticsFilter,
    provider: Option<&str>,
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
    let min_ts: Option<i64> = if let Some(prov) = provider {
        conn.query_row(
            "SELECT MIN(timestamp_ms) FROM usage_events WHERE provider_id = ?1",
            params![prov],
            |r| r.get(0),
        )?
    } else {
        conn.query_row("SELECT MIN(timestamp_ms) FROM usage_events", [], |r| r.get(0))?
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

fn query_latest_tps(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
) -> CoreResult<Option<f64>> {
    let result = if let Some(prov) = provider {
        conn.query_row(
            r#"
            SELECT output_tps FROM usage_events
            WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2
              AND provider_id = ?3
              AND output_tps IS NOT NULL
              AND output_tps > 0.0
              AND output_tps <= 800.0
              AND (duration_ms IS NULL OR duration_ms >= 300)
              AND (output_tokens + reasoning_tokens) >= 8
            ORDER BY timestamp_ms DESC LIMIT 1
            "#,
            params![start_ms, end_ms, prov],
            |r| r.get::<_, f64>(0),
        )
    } else {
        conn.query_row(
            r#"
            SELECT output_tps FROM usage_events
            WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2
              AND output_tps IS NOT NULL
              AND output_tps > 0.0
              AND output_tps <= 800.0
              AND (duration_ms IS NULL OR duration_ms >= 300)
              AND (output_tokens + reasoning_tokens) >= 8
            ORDER BY timestamp_ms DESC LIMIT 1
            "#,
            params![start_ms, end_ms],
            |r| r.get::<_, f64>(0),
        )
    };

    match result {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(CoreError::from(e)),
    }
}

impl Storage {
    pub fn query_overview(&self, filter: &AnalyticsFilter, now_ms: i64) -> CoreResult<OverviewReport> {
        self.with_conn(|conn| {
            let provider = normalized_provider_filter(&filter.provider_id);
            let (eff_start_ms, eff_end_ms) =
                resolve_effective_window(conn, filter, provider, now_ms)?;
            let current = query_period_totals(conn, eff_start_ms, eff_end_ms, provider)?;

            let span = (eff_end_ms - eff_start_ms).max(HOUR_MS);
            let (tokens_delta_pct, cost_delta_pct, cache_delta_pct, tps_delta_pct) =
                if filter.all_time {
                    (None, None, None, None)
                } else {
                    let prev_start = eff_start_ms.saturating_sub(span);
                    let prev_end = eff_start_ms.saturating_sub(1);
                    let previous = query_period_totals(conn, prev_start, prev_end, provider)?;
                    (
                        pct_change(current.total_tokens() as f64, previous.total_tokens() as f64),
                        pct_change(current.cost_micro_usd as f64, previous.cost_micro_usd as f64),
                        pct_change(current.cache_hit_ratio(), previous.cache_hit_ratio()),
                        pct_change(current.avg_tps(), previous.avg_tps()),
                    )
                };

            let latest_output_tps = query_latest_tps(conn, eff_start_ms, eff_end_ms, provider)?;

            // Choose hourly series when range <= 48 hours, otherwise daily series
            let use_hourly = span <= 48 * HOUR_MS;
            let (table_name, bucket_size, series_granularity) = if use_hourly {
                ("usage_hourly_rollups", HOUR_MS, "hourly".to_string())
            } else {
                ("usage_daily_rollups", DAY_MS, "daily".to_string())
            };
            let aligned_start = eff_start_ms.div_euclid(bucket_size) * bucket_size;

            let sql = if provider.is_some() {
                format!(
                    r#"
                    SELECT bucket_start_ms,
                           SUM(input_tokens), SUM(output_tokens),
                           SUM(cache_read_tokens), SUM(cache_write_tokens),
                           SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
                           SUM(request_count)
                    FROM {table_name}
                    WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2 AND provider_id = ?3
                    GROUP BY bucket_start_ms
                    ORDER BY bucket_start_ms ASC
                    "#
                )
            } else {
                format!(
                    r#"
                    SELECT bucket_start_ms,
                           SUM(input_tokens), SUM(output_tokens),
                           SUM(cache_read_tokens), SUM(cache_write_tokens),
                           SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
                           SUM(request_count)
                    FROM {table_name}
                    WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2
                    GROUP BY bucket_start_ms
                    ORDER BY bucket_start_ms ASC
                    "#
                )
            };

            let mut series = Vec::new();
            let mut stmt = conn.prepare(&sql)?;
            let map_point = |row: &rusqlite::Row<'_>| -> rusqlite::Result<TimeBucketPoint> {
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
            };

            if let Some(prov) = provider {
                let rows = stmt.query_map(params![aligned_start, eff_end_ms, prov], map_point)?;
                for row in rows {
                    series.push(row?);
                }
            } else {
                let rows = stmt.query_map(params![aligned_start, eff_end_ms], map_point)?;
                for row in rows {
                    series.push(row?);
                }
            }

            let prov_series_sql = if provider.is_some() {
                format!(
                    r#"
                    SELECT bucket_start_ms, provider_id,
                           SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens),
                           SUM(estimated_cost_micro_usd),
                           SUM(request_count)
                    FROM {table_name}
                    WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2 AND provider_id = ?3
                    GROUP BY bucket_start_ms, provider_id
                    ORDER BY bucket_start_ms ASC
                    "#
                )
            } else {
                format!(
                    r#"
                    SELECT bucket_start_ms, provider_id,
                           SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens),
                           SUM(estimated_cost_micro_usd),
                           SUM(request_count)
                    FROM {table_name}
                    WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2
                    GROUP BY bucket_start_ms, provider_id
                    ORDER BY bucket_start_ms ASC
                    "#
                )
            };

            let mut prov_buckets: BTreeMap<String, BTreeMap<i64, (i64, i64, i64)>> =
                BTreeMap::new();
            let mut prov_stmt = conn.prepare(&prov_series_sql)?;
            let map_prov_row =
                |row: &rusqlite::Row<'_>| -> rusqlite::Result<(i64, String, i64, i64, i64)> {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                };
            if let Some(prov) = provider {
                let rows =
                    prov_stmt.query_map(params![aligned_start, eff_end_ms, prov], map_prov_row)?;
                for row in rows {
                    let (b, pid, tok, cost, req) = row?;
                    prov_buckets.entry(pid).or_default().insert(b, (tok, cost, req));
                }
            } else {
                let rows =
                    prov_stmt.query_map(params![aligned_start, eff_end_ms], map_prov_row)?;
                for row in rows {
                    let (b, pid, tok, cost, req) = row?;
                    prov_buckets.entry(pid).or_default().insert(b, (tok, cost, req));
                }
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
                let period_t = query_period_totals(conn, eff_start_ms, eff_end_ms, Some(&pid))?;
                let today_t = query_period_totals(conn, today_start_ms, now_ms, Some(&pid))?;
                let month_t = query_period_totals(conn, month_start_ms, now_ms, Some(&pid))?;

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

            Ok(OverviewReport {
                total_tokens: current.total_tokens(),
                total_cost_micro_usd: current.cost_micro_usd,
                cache_hit_ratio: current.cache_hit_ratio(),
                avg_output_tps: current.avg_tps(),
                peak_output_tps: current.max_tps,
                latest_output_tps,
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
            })
        })
    }

    pub fn query_trends(&self, query: &TrendsQuery) -> CoreResult<TrendsReport> {
        self.with_conn(|conn| {
            let filter = &query.filter;
            let provider = normalized_provider_filter(&filter.provider_id);
            let (eff_start_ms, eff_end_ms) =
                resolve_effective_window(conn, filter, provider, Utc::now().timestamp_millis())?;
            let span = (eff_end_ms - eff_start_ms).max(HOUR_MS);
            let effective_granularity = resolve_trend_granularity(query.granularity, span);

            let current_rows = fetch_trend_rows(
                conn,
                eff_start_ms,
                eff_end_ms,
                provider,
                effective_granularity,
            )?;
            let previous_rows = if filter.all_time {
                Vec::new()
            } else {
                let prev_start = eff_start_ms.saturating_sub(span);
                let prev_end = eff_start_ms.saturating_sub(1);
                fetch_trend_rows(conn, prev_start, prev_end, provider, effective_granularity)?
            };

            let mut all_buckets = BTreeSet::new();
            // series_map[key][bucket] = (tokens, cost, requests)
            let mut series_map: BTreeMap<String, BTreeMap<i64, (i64, i64, i64)>> = BTreeMap::new();
            let mut prev_totals: HashMap<String, (i64, i64)> = HashMap::new();

            let accumulate = |rows: &[RawTrendRow],
                              buckets: &mut BTreeSet<i64>,
                              map: &mut BTreeMap<String, BTreeMap<i64, (i64, i64, i64)>>,
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
                                .or_insert((0, 0, 0));
                            entry.0 += tokens;
                            entry.1 += row.cost_micro_usd;
                            entry.2 += row.requests;
                        }
                        StackDimension::Model => {
                            let tokens = row.total_tokens();
                            let entry = map
                                .entry(row.model_id.clone())
                                .or_default()
                                .entry(row.bucket_start_ms)
                                .or_insert((0, 0, 0));
                            entry.0 += tokens;
                            entry.1 += row.cost_micro_usd;
                            entry.2 += row.requests;
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
                                    .or_insert((0, 0, 0));
                                entry.0 += tok;
                                entry.1 += proportional_cost;
                                entry.2 += if key == "input" { row.requests } else { 0 };
                            }
                        }
                    }
                }
            };

            accumulate(&current_rows, &mut all_buckets, &mut series_map, true);

            // Generate continuous aligned buckets across the active window when data exists (capped at 2000).
            if !all_buckets.is_empty() {
                let bucket_step = granularity_bucket_ms(effective_granularity);
                let first_b = eff_start_ms.div_euclid(bucket_step) * bucket_step;
                let last_b = eff_end_ms.div_euclid(bucket_step) * bucket_step;
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

            let mut prev_map: BTreeMap<String, BTreeMap<i64, (i64, i64, i64)>> = BTreeMap::new();
            let mut dummy_buckets = BTreeSet::new();
            accumulate(&previous_rows, &mut dummy_buckets, &mut prev_map, false);
            for (key, bucket_map) in prev_map {
                let mut tok_sum = 0i64;
                let mut cost_sum = 0i64;
                for (t, c, _) in bucket_map.values() {
                    tok_sum += t;
                    cost_sum += c;
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

                for &b in &buckets {
                    let (t, c, r) = bucket_map.get(&b).copied().unwrap_or((0, 0, 0));
                    total_tokens += t;
                    total_cost += c;
                    total_reqs += r;
                    points.push(TrendPoint {
                        bucket_start_ms: b,
                        tokens: t,
                        cost_micro_usd: c,
                        requests: r,
                    });
                }

                let label = format_dimension_label(query.dimension, &key);
                let (prev_tok, prev_cost, delta_pct) = if filter.all_time {
                    (0, 0, None)
                } else {
                    let (pt, pc) = prev_totals.get(&key).copied().unwrap_or((0, 0));
                    (pt, pc, pct_change(total_tokens as f64, pt as f64))
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

            let provider = normalized_provider_filter(&query.provider_id);

            // Query hourly rollups for exact local-timezone day grouping.
            let rows = fetch_hourly_rows(conn, start_ms, end_ms, provider)?;

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
                *acc.model_tokens.entry(r.model_id).or_default() += tok;

                month_cost += r.cost_micro_usd;
                month_tokens += tok;
                month_requests += r.requests;
                month_saved += r.cache_saved_micro_usd;
            }

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

            let hm_rows = fetch_hourly_rows(conn, heatmap_start_ms, end_ms, provider)?;
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
                daily_ledger,
                heatmap_365d,
            })
        })
    }

    pub fn query_models_pricing(&self, filter: &AnalyticsFilter) -> CoreResult<ModelStatsReport> {
        self.with_conn(|conn| {
            let provider = normalized_provider_filter(&filter.provider_id);
            let (eff_start_ms, eff_end_ms) =
                resolve_effective_window(conn, filter, provider, Utc::now().timestamp_millis())?;
            let catalog = load_pricing_catalog(conn)?;
            let aligned_start = eff_start_ms.div_euclid(HOUR_MS) * HOUR_MS;

            let sql = if provider.is_some() {
                r#"
                SELECT model_id, provider_id,
                       SUM(request_count), SUM(observable_step_count),
                       SUM(input_tokens), SUM(output_tokens),
                       SUM(cache_read_tokens), SUM(cache_write_tokens),
                       SUM(reasoning_tokens), SUM(cache_saved_micro_usd),
                       SUM(estimated_cost_micro_usd),
                       SUM(tps_sample_count), SUM(tps_sum)
                FROM usage_hourly_rollups
                WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2 AND provider_id = ?3
                GROUP BY model_id, provider_id
                ORDER BY SUM(estimated_cost_micro_usd) DESC, SUM(request_count) DESC
                "#
            } else {
                r#"
                SELECT model_id, provider_id,
                       SUM(request_count), SUM(observable_step_count),
                       SUM(input_tokens), SUM(output_tokens),
                       SUM(cache_read_tokens), SUM(cache_write_tokens),
                       SUM(reasoning_tokens), SUM(cache_saved_micro_usd),
                       SUM(estimated_cost_micro_usd),
                       SUM(tps_sample_count), SUM(tps_sum)
                FROM usage_hourly_rollups
                WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2
                GROUP BY model_id, provider_id
                ORDER BY SUM(estimated_cost_micro_usd) DESC, SUM(request_count) DESC
                "#
            };

            let mut stmt = conn.prepare(sql)?;
            let mut raw_rows = Vec::new();
            let mut total_cost = 0i64;
            let mut total_saved = 0i64;

            let collect_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<ModelStatRow> {
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
            };

            if let Some(prov) = provider {
                let iter = stmt.query_map(params![aligned_start, eff_end_ms, prov], |r| {
                    collect_row(r)
                })?;
                for item in iter {
                    let row = item?;
                    total_cost += row.estimated_cost_micro_usd;
                    total_saved += row.cache_saved_micro_usd;
                    raw_rows.push(row);
                }
            } else {
                let iter = stmt.query_map(params![aligned_start, eff_end_ms], |r| {
                    collect_row(r)
                })?;
                for item in iter {
                    let row = item?;
                    total_cost += row.estimated_cost_micro_usd;
                    total_saved += row.cache_saved_micro_usd;
                    raw_rows.push(row);
                }
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
        self.with_conn(|conn| {
            let plans: Vec<_> = load_subscription_plans(conn)?
                .into_iter()
                .filter(|p| p.is_user_configured)
                .collect();
            let observed = load_observed_rate_limits(conn)?;
            let samples_map = load_window_samples_map(conn, now_ms)?;

            let mut out = Vec::with_capacity(plans.len());
            for plan in plans {
                let samples = samples_map
                    .get(&plan.provider_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                out.push(evaluate_subscription_plan(&plan, samples, &observed, now_ms));
            }
            Ok(out)
        })
    }

    pub fn query_speed_latency(&self, filter: &AnalyticsFilter) -> CoreResult<SpeedLatencyReport> {
        self.with_conn(|conn| {
            let provider = normalized_provider_filter(&filter.provider_id);
            let (eff_start_ms, eff_end_ms) =
                resolve_effective_window(conn, filter, provider, Utc::now().timestamp_millis())?;

            struct RawSpeedEvent {
                model_id: String,
                provider_id: String,
                output_tps: f64,
                duration_ms: Option<i64>,
                ttft_ms: Option<i64>,
            }

            let sql = if provider.is_some() {
                r#"
                SELECT model_id, provider_id, output_tps, duration_ms, ttft_ms
                FROM usage_events
                WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2
                  AND provider_id = ?3
                  AND output_tps IS NOT NULL
                  AND output_tps > 0.0
                  AND output_tps <= 800.0
                  AND (duration_ms IS NULL OR duration_ms >= 300)
                  AND (output_tokens + reasoning_tokens) >= 8
                ORDER BY timestamp_ms ASC
                "#
            } else {
                r#"
                SELECT model_id, provider_id, output_tps, duration_ms, ttft_ms
                FROM usage_events
                WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2
                  AND output_tps IS NOT NULL
                  AND output_tps > 0.0
                  AND output_tps <= 800.0
                  AND (duration_ms IS NULL OR duration_ms >= 300)
                  AND (output_tokens + reasoning_tokens) >= 8
                ORDER BY timestamp_ms ASC
                "#
            };

            let mut stmt = conn.prepare(sql)?;
            let mut events = Vec::new();

            let map_fn = |row: &rusqlite::Row<'_>| -> rusqlite::Result<RawSpeedEvent> {
                Ok(RawSpeedEvent {
                    model_id: row.get(0)?,
                    provider_id: row.get(1)?,
                    output_tps: row.get(2)?,
                    duration_ms: row.get(3)?,
                    ttft_ms: row.get(4)?,
                })
            };

            if let Some(prov) = provider {
                let rows = stmt.query_map(params![eff_start_ms, eff_end_ms, prov], map_fn)?;
                for r in rows {
                    events.push(r?);
                }
            } else {
                let rows = stmt.query_map(params![eff_start_ms, eff_end_ms], map_fn)?;
                for r in rows {
                    events.push(r?);
                }
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
                let mut tps_vals: Vec<f64> = group.iter().map(|e| e.output_tps).collect();
                tps_vals.sort_by(|a, b| a.total_cmp(b));
                let n = tps_vals.len();
                let mean_tps = tps_vals.iter().sum::<f64>() / n as f64;
                let p50_tps = percentile_sorted(&tps_vals, 0.50);
                let p95_tps = percentile_sorted(&tps_vals, 0.95);
                let min_tps = tps_vals.first().copied().unwrap_or(0.0);
                let max_tps = tps_vals.last().copied().unwrap_or(0.0);
                let latest_tps = group.last().map(|e| e.output_tps).unwrap_or(0.0);
                let durations: Vec<i64> = group.iter().filter_map(|e| e.duration_ms).collect();
                let avg_duration_ms = if durations.is_empty() {
                    0.0
                } else {
                    durations.iter().sum::<i64>() as f64 / durations.len() as f64
                };
                let ttfts: Vec<i64> = group.iter().filter_map(|e| e.ttft_ms).collect();
                let avg_ttft_ms = if ttfts.is_empty() {
                    None
                } else {
                    Some(ttfts.iter().sum::<i64>() as f64 / ttfts.len() as f64)
                };
                (
                    n as i64,
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

            Ok(SpeedLatencyReport { providers, models })
        })
    }

    pub fn query_tray_summary(&self, now_ms: i64, tz_offset_minutes: i32) -> CoreResult<TraySummary> {
        self.with_conn(|conn| {
            let (today_start_ms, _) = local_day_bounds_ms(now_ms, tz_offset_minutes);
            let today_totals = query_period_totals(conn, today_start_ms, now_ms, None)?;
            let latest_output_tps = query_latest_tps(conn, today_start_ms, now_ms, None)?;
            let prefs = load_preferences(conn)?;

            let plans = load_subscription_plans(conn)?;
            let observed = load_observed_rate_limits(conn)?;
            let samples_map = load_window_samples_map(conn, now_ms)?;

            let subscriptions = plans
                .into_iter()
                .filter(|p| p.enabled && p.is_user_configured)
                .map(|plan| {
                    let samples = samples_map
                        .get(&plan.provider_id)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    evaluate_subscription_plan(&plan, samples, &observed, now_ms)
                })
                .collect();

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
                subscriptions,
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
        StackDimension::Provider => match key {
            provider_ids::CLAUDE_CODE => "Claude Code".to_string(),
            provider_ids::CODEX => "Codex".to_string(),
            provider_ids::ANTIGRAVITY => "Antigravity".to_string(),
            other => other.to_string(),
        },
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
    granularity: TrendGranularity,
) -> CoreResult<Vec<RawTrendRow>> {
    let resolved = resolve_trend_granularity(granularity, (end_ms - start_ms).max(HOUR_MS));
    match resolved {
        TrendGranularity::Auto => unreachable!("resolve_trend_granularity resolves Auto"),
        TrendGranularity::FifteenMinutes => {
            const BUCKET_15M: i64 = 900_000;
            let sql = if provider.is_some() {
                r#"
                SELECT (timestamp_ms / 900000) * 900000 AS b,
                       provider_id, model_id,
                       COUNT(*), SUM(observable_only),
                       SUM(input_tokens), SUM(output_tokens),
                       SUM(cache_read_tokens), SUM(cache_write_tokens),
                       SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
                       SUM(cache_saved_micro_usd)
                FROM usage_events
                WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2 AND provider_id = ?3
                GROUP BY b, provider_id, model_id
                ORDER BY b ASC
                "#
            } else {
                r#"
                SELECT (timestamp_ms / 900000) * 900000 AS b,
                       provider_id, model_id,
                       COUNT(*), SUM(observable_only),
                       SUM(input_tokens), SUM(output_tokens),
                       SUM(cache_read_tokens), SUM(cache_write_tokens),
                       SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
                       SUM(cache_saved_micro_usd)
                FROM usage_events
                WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2
                GROUP BY b, provider_id, model_id
                ORDER BY b ASC
                "#
            };
            let _ = BUCKET_15M;
            run_trend_query(conn, sql, start_ms, end_ms, provider)
        }
        TrendGranularity::Hourly => fetch_hourly_rows(conn, start_ms, end_ms, provider),
        TrendGranularity::Daily => fetch_daily_rows(conn, start_ms, end_ms, provider),
        TrendGranularity::Weekly => {
            let daily = fetch_daily_rows(conn, start_ms, end_ms, provider)?;
            const WEEK_MS: i64 = 7 * DAY_MS;
            let mut grouped: BTreeMap<(i64, String, String), RawTrendRow> = BTreeMap::new();
            for row in daily {
                let week_bucket = row.bucket_start_ms.div_euclid(WEEK_MS) * WEEK_MS;
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

fn fetch_hourly_rows(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
) -> CoreResult<Vec<RawTrendRow>> {
    let aligned_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let sql = if provider.is_some() {
        r#"
        SELECT bucket_start_ms, provider_id, model_id,
               SUM(request_count), SUM(observable_step_count),
               SUM(input_tokens), SUM(output_tokens),
               SUM(cache_read_tokens), SUM(cache_write_tokens),
               SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
               SUM(cache_saved_micro_usd)
        FROM usage_hourly_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2 AND provider_id = ?3
        GROUP BY bucket_start_ms, provider_id, model_id
        ORDER BY bucket_start_ms ASC
        "#
    } else {
        r#"
        SELECT bucket_start_ms, provider_id, model_id,
               SUM(request_count), SUM(observable_step_count),
               SUM(input_tokens), SUM(output_tokens),
               SUM(cache_read_tokens), SUM(cache_write_tokens),
               SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
               SUM(cache_saved_micro_usd)
        FROM usage_hourly_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2
        GROUP BY bucket_start_ms, provider_id, model_id
        ORDER BY bucket_start_ms ASC
        "#
    };
    run_trend_query(conn, sql, aligned_start, end_ms, provider)
}

fn fetch_daily_rows(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
) -> CoreResult<Vec<RawTrendRow>> {
    let aligned_start = start_ms.div_euclid(DAY_MS) * DAY_MS;
    let sql = if provider.is_some() {
        r#"
        SELECT bucket_start_ms, provider_id, model_id,
               SUM(request_count), SUM(observable_step_count),
               SUM(input_tokens), SUM(output_tokens),
               SUM(cache_read_tokens), SUM(cache_write_tokens),
               SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
               SUM(cache_saved_micro_usd)
        FROM usage_daily_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2 AND provider_id = ?3
        GROUP BY bucket_start_ms, provider_id, model_id
        ORDER BY bucket_start_ms ASC
        "#
    } else {
        r#"
        SELECT bucket_start_ms, provider_id, model_id,
               SUM(request_count), SUM(observable_step_count),
               SUM(input_tokens), SUM(output_tokens),
               SUM(cache_read_tokens), SUM(cache_write_tokens),
               SUM(reasoning_tokens), SUM(estimated_cost_micro_usd),
               SUM(cache_saved_micro_usd)
        FROM usage_daily_rollups
        WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2
        GROUP BY bucket_start_ms, provider_id, model_id
        ORDER BY bucket_start_ms ASC
        "#
    };
    run_trend_query(conn, sql, aligned_start, end_ms, provider)
}

fn run_trend_query(
    conn: &Connection,
    sql: &str,
    start_ms: i64,
    end_ms: i64,
    provider: Option<&str>,
) -> CoreResult<Vec<RawTrendRow>> {
    let mut stmt = conn.prepare(sql)?;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<RawTrendRow> {
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
    };

    let mut out = Vec::new();
    if let Some(prov) = provider {
        let rows = stmt.query_map(params![start_ms, end_ms, prov], map_row)?;
        for r in rows {
            out.push(r?);
        }
    } else {
        let rows = stmt.query_map(params![start_ms, end_ms], map_row)?;
        for r in rows {
            out.push(r?);
        }
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
        }];
        storage
            .ingest_events_with_cursor(&outliers, None, &observed)
            .expect("ingest outliers");

        let all_time_filter = AnalyticsFilter {
            start_ms: base_ts,
            end_ms: base_ts + 2 * HOUR_MS,
            provider_id: None,
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
        assert_eq!(codex_row.active_window_label.as_deref(), Some("5h 官方限额"));
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
        assert!(speed.providers.iter().all(|p| p.max_tps <= 800.0));
        assert!(speed.models.iter().all(|m| m.max_tps <= 800.0));
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
}
