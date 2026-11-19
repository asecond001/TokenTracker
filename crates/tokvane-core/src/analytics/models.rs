//! Per-model token, cost and efficiency breakdown.

use super::monthly::prorate_monthly_fee;
use super::*;

impl Storage {
    pub fn query_models_pricing(&self, filter: &AnalyticsFilter) -> CoreResult<ModelStatsReport> {
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
                let list_cost_micro_usd = effective_pricing
                    .as_ref()
                    .map(|rule| {
                        list_price_cost(
                            rule,
                            input_tokens,
                            output_tokens,
                            cache_read_tokens,
                            cache_write_tokens,
                            reasoning_tokens,
                        )
                    })
                    .unwrap_or(0);

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
                    list_cost_micro_usd,
                    subscription_cost_micro_usd: None,
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

            // Subscription amortization: include enabled user-configured local plans and
            // deduped paid remote synced snapshots, prorated to the selected range up to now.
            let fee_end_ms = eff_end_ms.min(Utc::now().timestamp_millis());
            let local_dev = crate::storage::repository::query_local_device(conn)?;
            let include_local_plans = device_id.is_none_or(|d| d == local_dev.device_id);
            let plans = if include_local_plans {
                load_subscription_plans(conn)?
            } else {
                Vec::new()
            };
            let remote_plans = load_remote_paid_plans(
                conn,
                &local_dev.device_id,
                provider,
                device_id,
            )?;
            let subscription_allocations = build_subscription_allocations(
                &plans,
                &remote_plans,
                &raw_rows,
                eff_start_ms,
                fee_end_ms,
            );
            allocate_subscription_costs(&mut raw_rows, &subscription_allocations);

            Ok(ModelStatsReport {
                rows: raw_rows,
                total_cost_micro_usd: total_cost,
                total_cache_saved_micro_usd: total_saved,
                catalog,
                subscription_allocations,
            })
        })
    }
}

/// Cost at API list price with prompt caching ignored: all prompt tokens at the input rate.
pub(crate) fn list_price_cost(
    rule: &ModelPricingEntry,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    reasoning_tokens: i64,
) -> i64 {
    let prompt = input_tokens.max(0) as i128
        + cache_read_tokens.max(0) as i128
        + cache_write_tokens.max(0) as i128;
    let micro = prompt * rule.input_per_m_micro_usd.max(0) as i128
        + output_tokens.max(0) as i128 * rule.output_per_m_micro_usd.max(0) as i128
        + reasoning_tokens.max(0) as i128 * rule.reasoning_per_m_micro_usd.max(0) as i128;
    // Round half away from zero to whole micro-USD, matching `compute_event_cost`.
    let rounded = (micro + 500_000) / 1_000_000;
    i64::try_from(rounded).unwrap_or(i64::MAX)
}

struct RemotePaidPlan {
    provider_id: String,
    plan_id: String,
    plan_name: String,
    monthly_fee_micro_usd: i64,
}

fn load_remote_paid_plans(
    conn: &Connection,
    local_device_id: &str,
    provider: Option<&str>,
    device_id: Option<&str>,
) -> CoreResult<Vec<RemotePaidPlan>> {
    let mut sql = String::from(
        "SELECT provider_id, plan_id, MAX(plan_name), MAX(monthly_fee_micro_usd)
         FROM remote_subscription_snapshots
         WHERE monthly_fee_micro_usd > 0 AND device_id != ?1",
    );
    let mut params: Vec<Value> = vec![Value::Text(local_device_id.to_string())];
    let mut next_param = 2usize;
    if let Some(prov) = provider {
        sql.push_str(&format!(" AND LOWER(provider_id) = LOWER(?{next_param})"));
        params.push(Value::Text(prov.to_string()));
        next_param += 1;
    }
    if let Some(dev) = device_id {
        sql.push_str(&format!(" AND device_id = ?{next_param}"));
        params.push(Value::Text(dev.to_string()));
    }
    sql.push_str(" GROUP BY device_id, provider_id, plan_id ORDER BY MAX(updated_at_ms) DESC, plan_id ASC");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params), |r| {
        Ok(RemotePaidPlan {
            provider_id: r.get(0)?,
            plan_id: r.get(1)?,
            plan_name: r.get(2)?,
            monthly_fee_micro_usd: r.get(3)?,
        })
    })?;
    let mut out = Vec::new();
    for item in rows {
        out.push(item?);
    }
    Ok(out)
}

fn build_subscription_allocations(
    plans: &[SubscriptionPlan],
    remote_plans: &[RemotePaidPlan],
    rows: &[ModelStatRow],
    start_ms: i64,
    end_ms: i64,
) -> Vec<ProviderSubscriptionAllocation> {
    let providers: BTreeSet<&str> = rows.iter().map(|r| r.provider_id.as_str()).collect();
    let mut out = Vec::new();
    for provider_id in providers {
        let mut covered_plan_ids = HashSet::new();
        let mut plan_names = Vec::new();
        let mut monthly_fee_micro_usd: i64 = 0;

        for p in plans.iter().filter(|p| {
            p.enabled
                && p.is_user_configured
                && p.monthly_fee_micro_usd > 0
                && p.provider_id.eq_ignore_ascii_case(provider_id)
        }) {
            covered_plan_ids.insert(p.plan_id.to_ascii_lowercase());
            plan_names.push(p.plan_name.clone());
            monthly_fee_micro_usd = monthly_fee_micro_usd.saturating_add(p.monthly_fee_micro_usd);
        }

        for rp in remote_plans.iter().filter(|rp| {
            rp.monthly_fee_micro_usd > 0
                && rp.provider_id.eq_ignore_ascii_case(provider_id)
                && !covered_plan_ids.contains(&rp.plan_id.to_ascii_lowercase())
        }) {
            plan_names.push(rp.plan_name.clone());
            monthly_fee_micro_usd = monthly_fee_micro_usd.saturating_add(rp.monthly_fee_micro_usd);
        }

        if monthly_fee_micro_usd <= 0 {
            continue;
        }
        out.push(ProviderSubscriptionAllocation {
            provider_id: provider_id.to_string(),
            plan_names,
            monthly_fee_micro_usd,
            period_fee_micro_usd: prorate_monthly_fee(monthly_fee_micro_usd, start_ms, end_ms),
        });
    }
    out
}

/// Splits each provider's period fee across its models by API-equivalent cost
/// (token share when the provider has no priced usage). Rounding remainders go
/// to the largest row so the shares always sum to the period fee.
fn allocate_subscription_costs(
    rows: &mut [ModelStatRow],
    allocations: &[ProviderSubscriptionAllocation],
) {
    for alloc in allocations {
        let idx: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.provider_id == alloc.provider_id)
            .map(|(i, _)| i)
            .collect();
        let cost_total: i128 = idx.iter().map(|&i| rows[i].estimated_cost_micro_usd.max(0) as i128).sum();
        let token_total: i128 = idx.iter().map(|&i| rows[i].total_tokens.max(0) as i128).sum();
        let (weight, total): (fn(&ModelStatRow) -> i128, i128) = if cost_total > 0 {
            (|r| r.estimated_cost_micro_usd.max(0) as i128, cost_total)
        } else {
            (|r| r.total_tokens.max(0) as i128, token_total)
        };
        if total <= 0 {
            continue;
        }
        let fee = alloc.period_fee_micro_usd as i128;
        let mut assigned: i128 = 0;
        let mut largest: Option<usize> = None;
        for &i in &idx {
            let share = fee * weight(&rows[i]) / total;
            assigned += share;
            rows[i].subscription_cost_micro_usd = Some(i64::try_from(share).unwrap_or(i64::MAX));
            if largest.is_none_or(|l| weight(&rows[i]) > weight(&rows[l])) {
                largest = Some(i);
            }
        }
        if let Some(l) = largest {
            let remainder = fee - assigned;
            if let Some(v) = rows[l].subscription_cost_micro_usd.as_mut() {
                *v = v.saturating_add(i64::try_from(remainder).unwrap_or(0));
            }
        }
    }
}
