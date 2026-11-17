//! Trend series, granularity resolution and period comparison.

use super::*;

impl Storage {
    pub fn query_trends(&self, query: &TrendsQuery) -> CoreResult<TrendsReport> {
        self.with_read_conn(|conn| {
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
}

pub(super) fn resolve_trend_granularity(requested: TrendGranularity, span_ms: i64) -> TrendGranularity {
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

pub(super) fn granularity_bucket_ms(granularity: TrendGranularity) -> i64 {
    match granularity {
        TrendGranularity::FifteenMinutes => 900_000,
        TrendGranularity::Auto | TrendGranularity::Hourly => HOUR_MS,
        TrendGranularity::Daily => DAY_MS,
        TrendGranularity::Weekly => 7 * DAY_MS,
    }
}

pub(super) struct RawTrendRow {
    pub(super) bucket_start_ms: i64,
    pub(super) provider_id: String,
    pub(super) model_id: String,
    pub(super) requests: i64,
    pub(super) observable_steps: i64,
    pub(super) input_tokens: i64,
    pub(super) output_tokens: i64,
    pub(super) cache_read_tokens: i64,
    pub(super) cache_write_tokens: i64,
    pub(super) reasoning_tokens: i64,
    pub(super) cost_micro_usd: i64,
    pub(super) cache_saved_micro_usd: i64,
}

impl RawTrendRow {
    pub(super) fn total_tokens(&self) -> i64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_tokens
            + self.cache_write_tokens
            + self.reasoning_tokens
    }
}

pub(super) fn fetch_trend_rows(
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

pub(super) fn fetch_hourly_rows(
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

pub(super) fn fetch_daily_rows(
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

pub(super) fn run_trend_query(
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
