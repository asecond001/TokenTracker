//! Generation speed and latency statistics.

use super::*;

impl Storage {
    pub fn query_speed_latency(&self, filter: &AnalyticsFilter) -> CoreResult<SpeedLatencyReport> {
        self.with_read_conn(|conn| {
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
}

pub(super) struct RollupSpeedBucket {
    pub(super) model_id: String,
    pub(super) provider_id: String,
    pub(super) request_count: i64,
    pub(super) total_duration_ms: i64,
    pub(super) tps_sample_count: i64,
    pub(super) tps_sum: f64,
    pub(super) max_output_tps: f64,
}

pub(super) fn summarize_rollup_speed_group(
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

pub(super) fn query_speed_latency_from_hourly_rollups(
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
