//! Monthly ledger, 365-day heatmap and CSV / JSON export.

use super::*;

impl Storage {
    pub fn query_monthly(&self, query: &MonthlyQuery) -> CoreResult<MonthlyReport> {
        self.with_read_conn(|conn| {
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

            let month_start_ms = month_start_date
                .and_hms_opt(0, 0, 0)
                .and_then(|ndt| offset.from_local_datetime(&ndt).single())
                .map(|d| d.timestamp_millis())
                .ok_or_else(|| CoreError::TimeOverflow("Month start timestamp".to_string()))?;
            let month_end_ms = next_month_date
                .and_hms_opt(0, 0, 0)
                .and_then(|ndt| offset.from_local_datetime(&ndt).single())
                .map(|d| d.timestamp_millis() - 1)
                .ok_or_else(|| CoreError::TimeOverflow("Month end timestamp".to_string()))?;

            let custom_range = match (query.start_ms, query.end_ms) {
                (Some(s), Some(e)) if e >= s => Some((s, e)),
                (Some(_), Some(_)) => {
                    return Err(CoreError::Validation(
                        "Statistics range end precedes start".to_string(),
                    ))
                }
                _ => None,
            };
            let (start_ms, end_ms) = custom_range.unwrap_or((month_start_ms, month_end_ms));

            let provider = normalized_optional_filter(&query.provider_id);
            let device_id = normalized_optional_filter(&query.device_id);

            // Query hourly rollups for exact local-timezone day grouping.
            let rows = fetch_hourly_rows(conn, start_ms, end_ms, provider, device_id)?;
            // Open-ended ranges (e.g. "all history" starting at epoch 0) prorate
            // subscription fees from the first observed bucket instead.
            let first_bucket_ms = rows.iter().map(|r| r.bucket_start_ms).min();

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

            let subscription_period_fee_micro_usd = if custom_range.is_some() {
                let fee_start_ms = if start_ms <= 0 {
                    first_bucket_ms.unwrap_or(start_ms)
                } else {
                    start_ms
                };
                prorate_monthly_fee(subscription_monthly_fee_micro_usd, fee_start_ms, end_ms)
            } else {
                subscription_monthly_fee_micro_usd
            };

            let roi_multiplier = if subscription_period_fee_micro_usd > 0 {
                Some(month_cost as f64 / subscription_period_fee_micro_usd as f64)
            } else {
                None
            };

            // 365-day heatmap ending at the last local day of the statistics range, aligned via hourly rollups.
            let end_date = if custom_range.is_some() {
                local_date_from_ms(end_ms, offset).unwrap_or(month_start_date)
            } else {
                next_month_date.pred_opt().unwrap_or(month_start_date)
            };
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
                range_start_ms: start_ms,
                range_end_ms: end_ms,
                subscription_period_fee_micro_usd,
            })
        })
    }
}

/// Average Gregorian month length used to prorate monthly subscription fees.
const AVG_MONTH_MS: f64 = 30.436_875 * DAY_MS as f64;

/// Prorates a monthly fee to the inclusive `[start_ms, end_ms]` range.
pub(crate) fn prorate_monthly_fee(monthly_fee_micro_usd: i64, start_ms: i64, end_ms: i64) -> i64 {
    if monthly_fee_micro_usd <= 0 || end_ms < start_ms {
        return 0;
    }
    let span_ms = (end_ms - start_ms + 1) as f64;
    (monthly_fee_micro_usd as f64 * span_ms / AVG_MONTH_MS).round() as i64
}

fn local_date_from_ms(ms: i64, offset: FixedOffset) -> Option<NaiveDate> {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.with_timezone(&offset).date_naive())
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
