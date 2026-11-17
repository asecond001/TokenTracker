//! Subscription window evaluation against observed and local spend.

use super::*;

impl Storage {
    pub fn query_subscriptions(&self, now_ms: i64) -> CoreResult<Vec<SubscriptionPlanStatus>> {
        let tz_offset_minutes = Local::now().offset().local_minus_utc() / 60;
        self.query_subscriptions_with_tz(now_ms, tz_offset_minutes)
    }

    pub fn query_subscriptions_with_tz(
        &self,
        now_ms: i64,
        tz_offset_minutes: i32,
    ) -> CoreResult<Vec<SubscriptionPlanStatus>> {
        self.with_read_conn(|conn| {
            let plans = load_subscription_plans(conn)?;
            evaluate_subscriptions_with_spend(conn, plans, now_ms, tz_offset_minutes, true)
        })
    }

    pub fn query_subscription_templates(
        &self,
        now_ms: i64,
    ) -> CoreResult<Vec<crate::domain::SubscriptionPlanTemplate>> {
        self.with_read_conn(|conn| {
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
}

pub(super) fn is_observed_window_fresh(obs: &ObservedRateLimitWindow, is_secondary: bool, now_ms: i64) -> bool {
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

pub(super) fn fallback_observed_window_status(
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

pub(super) fn evaluate_subscriptions_with_spend(
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
