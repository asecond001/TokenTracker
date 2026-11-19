//! Today overview, live sessions, model speed pulse and tray summary.

use super::*;

impl Storage {
    pub fn query_overview(&self, filter: &AnalyticsFilter, now_ms: i64) -> CoreResult<OverviewReport> {
        self.with_read_conn(|conn| {
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
            let mut seen_providers: HashSet<String> = HashSet::new();
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

            let mut provider_entries: Vec<(String, bool, Option<i64>, Option<String>)> = Vec::new();
            for cfg in cfg_rows {
                let item = cfg?;
                seen_providers.insert(item.0.to_ascii_lowercase());
                provider_entries.push(item);
            }

            // Include any custom or dynamically synced providers present in rollups.
            let mut extra_stmt =
                conn.prepare("SELECT DISTINCT provider_id FROM usage_hourly_rollups ORDER BY provider_id ASC")?;
            let extra_rows = extra_stmt.query_map([], |r| r.get::<_, String>(0))?;
            for extra in extra_rows {
                let pid = extra?;
                if seen_providers.insert(pid.to_ascii_lowercase()) {
                    provider_entries.push((pid, true, None, None));
                }
            }

            for (pid, enabled, last_sync_at_ms, last_error) in provider_entries {
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
                let (mut active_window_label, mut active_window_used_percent) =
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
                if active_window_label.is_none() {
                    let (remote_lbl, remote_pct) =
                        fallback_remote_window_status(conn, &pid, device_id)?;
                    active_window_label = remote_lbl;
                    active_window_used_percent = remote_pct;
                }

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
            let has_raw_events: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM usage_events LIMIT 1)",
                [],
                |r| r.get(0),
            )?;
            let (latest_output_tps, live_model_speeds) = if active_sessions.is_empty() && !has_raw_events {
                let rollup_live =
                    query_recent_rollup_live_speeds(conn, now_ms, provider, device_id)?;
                if !rollup_live.is_empty() {
                    let primary = rollup_live.iter().map(|s| s.output_tps).sum::<f64>()
                        / rollup_live.len() as f64;
                    (Some(primary), rollup_live)
                } else {
                    (fallback_latest.map(|(tps, _, _)| tps), Vec::new())
                }
            } else {
                compute_live_model_speeds_and_tps(&active_sessions, fallback_latest)
            };

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

    pub fn query_tray_summary(&self, now_ms: i64, tz_offset_minutes: i32) -> CoreResult<TraySummary> {
        self.with_read_conn(|conn| {
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
            let total_ingested_events = self.total_event_count(conn)?;

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

/// Returns `(latest_output_tps, live_model_speeds)`.
///
/// `live_model_speeds` only ever contains sessions that are *currently active*
/// (last request within `ACTIVE_STATE_WINDOW_MS`), so the live readout drops
/// to empty as soon as generation stops instead of freezing on a stale value.
/// `latest_output_tps` keeps the most recent observed speed as a historical
/// reference (active mean when available, otherwise the latest sample).
pub(super) fn compute_live_model_speeds_and_tps(
    active_sessions: &[ActiveSessionSummary],
    fallback_recent: Option<(f64, String, String)>,
) -> (Option<f64>, Vec<LiveModelSpeed>) {
    let mut grouped_order: Vec<(String, String)> = Vec::new();
    let mut grouped_vals: HashMap<(String, String), Vec<f64>> = HashMap::new();

    for sess in active_sessions {
        if sess.activity_state != "active" {
            continue;
        }
        let Some(tps) = sess.latest_output_tps.filter(|v| v.is_finite() && *v > 0.0) else {
            continue;
        };
        let key = (sess.provider_id.clone(), sess.model_id.clone());
        if !grouped_vals.contains_key(&key) {
            grouped_order.push(key.clone());
        }
        grouped_vals.entry(key).or_default().push(tps);
    }

    let mut items = Vec::with_capacity(grouped_order.len());
    for key in grouped_order {
        if let Some(vals) = grouped_vals.remove(&key) {
            let mean = vals.iter().sum::<f64>() / vals.len() as f64;
            items.push(LiveModelSpeed {
                provider_id: key.0,
                model_id: key.1,
                output_tps: mean,
                activity_state: "active".to_string(),
            });
        }
    }

    if !items.is_empty() {
        let primary_tps = items.iter().map(|i| i.output_tps).sum::<f64>() / items.len() as f64;
        return (Some(primary_tps), items);
    }

    (fallback_recent.map(|(tps, _, _)| tps), Vec::new())
}

fn fallback_remote_window_status(
    conn: &Connection,
    provider_id: &str,
    device_id: Option<&str>,
) -> CoreResult<(Option<String>, Option<f64>)> {
    let mut sql = String::from(
        "SELECT window_label, used_percent FROM remote_subscription_snapshots WHERE LOWER(provider_id) = LOWER(?1)",
    );
    let mut params_vec: Vec<Value> = vec![Value::Text(provider_id.to_string())];
    if let Some(dev) = device_id {
        sql.push_str(" AND device_id = ?2");
        params_vec.push(Value::Text(dev.to_string()));
    }
    sql.push_str(" ORDER BY used_percent DESC, updated_at_ms DESC LIMIT 1");
    match conn.query_row(&sql, params_from_iter(params_vec), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
    }) {
        Ok((lbl, pct)) => Ok((Some(lbl), Some(pct))),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None)),
        Err(e) => Err(CoreError::from(e)),
    }
}

/// Recent live generation speeds for pure-rollup mobile databases (active if updated within 3 minutes).
pub(crate) fn query_recent_rollup_live_speeds(
    conn: &Connection,
    now_ms: i64,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> CoreResult<Vec<LiveModelSpeed>> {
    const MOBILE_ROLLUP_ACTIVE_WINDOW_MS: i64 = 180_000;
    let since_ms = now_ms.saturating_sub(MOBILE_ROLLUP_ACTIVE_WINDOW_MS);
    let scope = build_scope_clause(2, provider, device_id);
    let bound = bound_range_params(since_ms, now_ms, &scope.params);
    let sql = format!(
        r#"
        SELECT provider_id,
               model_id,
               SUM(tps_sum) / SUM(tps_sample_count) AS mean_tps
        FROM usage_hourly_rollups
        WHERE updated_at_ms >= ?1 AND updated_at_ms <= ?2{where_and}
          AND tps_sample_count > 0
          AND tps_sum > 0.0
          AND LOWER(model_id) NOT LIKE '%auto-review%'
          AND LOWER(model_id) NOT LIKE '%auto_review%'
        GROUP BY provider_id, model_id
        ORDER BY MAX(updated_at_ms) DESC, SUM(tps_sample_count) DESC
        LIMIT 6
        "#,
        where_and = scope.sql_and
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(bound), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, f64>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (provider_id, model_id, tps) = r?;
        if tps.is_finite() && tps > 0.0 && tps <= MAX_VALID_OUTPUT_TPS {
            out.push(LiveModelSpeed {
                provider_id,
                model_id,
                output_tps: tps,
                activity_state: "active".to_string(),
            });
        }
    }
    Ok(out)
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
pub(super) fn query_top_model_shares(
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

pub(super) fn is_background_session_marker(
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

pub(super) fn query_active_sessions(
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
