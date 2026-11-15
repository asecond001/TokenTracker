use serde::{Deserialize, Serialize};

use crate::domain::{
    provider_ids, ObservedRateLimitWindow, QuotaUnit, SubscriptionPlan, SubscriptionWindowRule,
};

const HOUR_MS: i64 = 3_600_000;

/// Single event contribution sample used for rolling window evaluation.
#[derive(Debug, Clone, Copy)]
pub struct WindowUsageSample {
    pub timestamp_ms: i64,
    pub cost_micro_usd: i64,
    pub total_tokens: i64,
}

/// Evaluated live status for one subscription window (e.g. 5h rolling or 7d weekly).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionWindowStatus {
    pub window_id: String,
    pub label: String,
    pub duration_hours: i64,
    pub rolling: bool,
    pub unit: QuotaUnit,
    pub used_value: i64,
    pub limit_value: i64,
    pub used_percent: f64,
    pub warning_percent: u8,
    pub critical_percent: u8,
    pub burn_rate_per_hour: f64,
    pub projected_exhaustion_hours: Option<f64>,
    pub resets_at_ms: Option<i64>,
    pub resets_in_seconds: i64,
    /// `"official_log_snapshot"` or `"local_estimation"`.
    pub source: String,
    /// `"normal"` | `"warning"` | `"critical"` | `"exhausted"`.
    pub alert_level: String,
}

/// Evaluated live status for a subscription plan across all its windows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionPlanStatus {
    pub plan: SubscriptionPlan,
    pub windows: Vec<SubscriptionWindowStatus>,
}

/// Built-in default subscription plans (`PLAN.md` §5.1, §5.2, §6.2.5).
pub fn default_subscription_plans() -> Vec<SubscriptionPlan> {
    vec![
        SubscriptionPlan {
            plan_id: "claude_pro".to_string(),
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            plan_name: "Claude Pro ($20/mo)".to_string(),
            enabled: true,
            monthly_fee_micro_usd: 20_000_000,
            windows: vec![
                SubscriptionWindowRule {
                    window_id: "5h".to_string(),
                    label: "5h 滚动窗口".to_string(),
                    duration_hours: 5,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 15_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
                SubscriptionWindowRule {
                    window_id: "7d".to_string(),
                    label: "7d 周额度".to_string(),
                    duration_hours: 168,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 75_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            updated_at_ms: 0,
        },
        SubscriptionPlan {
            plan_id: "claude_max_5x".to_string(),
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            plan_name: "Claude Max 5x ($100/mo)".to_string(),
            enabled: false,
            monthly_fee_micro_usd: 100_000_000,
            windows: vec![
                SubscriptionWindowRule {
                    window_id: "5h".to_string(),
                    label: "5h 滚动窗口".to_string(),
                    duration_hours: 5,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 75_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
                SubscriptionWindowRule {
                    window_id: "7d".to_string(),
                    label: "7d 周额度".to_string(),
                    duration_hours: 168,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 375_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            updated_at_ms: 0,
        },
        SubscriptionPlan {
            plan_id: "claude_max_20x".to_string(),
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            plan_name: "Claude Max 20x ($200/mo)".to_string(),
            enabled: false,
            monthly_fee_micro_usd: 200_000_000,
            windows: vec![
                SubscriptionWindowRule {
                    window_id: "5h".to_string(),
                    label: "5h 滚动窗口".to_string(),
                    duration_hours: 5,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 200_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
                SubscriptionWindowRule {
                    window_id: "7d".to_string(),
                    label: "7d 周额度".to_string(),
                    duration_hours: 168,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 1_200_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            updated_at_ms: 0,
        },
        SubscriptionPlan {
            plan_id: "codex_plus".to_string(),
            provider_id: provider_ids::CODEX.to_string(),
            plan_name: "ChatGPT Plus / Codex ($20/mo)".to_string(),
            enabled: true,
            monthly_fee_micro_usd: 20_000_000,
            windows: vec![
                SubscriptionWindowRule {
                    window_id: "5h".to_string(),
                    label: "5h 滚动窗口".to_string(),
                    duration_hours: 5,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 18_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
                SubscriptionWindowRule {
                    window_id: "7d".to_string(),
                    label: "7d 周额度".to_string(),
                    duration_hours: 168,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 90_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            updated_at_ms: 0,
        },
        SubscriptionPlan {
            plan_id: "codex_pro".to_string(),
            provider_id: provider_ids::CODEX.to_string(),
            plan_name: "ChatGPT Pro / Codex ($200/mo)".to_string(),
            enabled: false,
            monthly_fee_micro_usd: 200_000_000,
            windows: vec![
                SubscriptionWindowRule {
                    window_id: "5h".to_string(),
                    label: "5h 滚动窗口".to_string(),
                    duration_hours: 5,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 150_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
                SubscriptionWindowRule {
                    window_id: "7d".to_string(),
                    label: "7d 周额度".to_string(),
                    duration_hours: 168,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 900_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            updated_at_ms: 0,
        },
        SubscriptionPlan {
            plan_id: "antigravity_ai_pro".to_string(),
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            plan_name: "Google AI Pro / Antigravity ($19.99/mo)".to_string(),
            enabled: true,
            monthly_fee_micro_usd: 19_990_000,
            windows: vec![
                SubscriptionWindowRule {
                    window_id: "5h".to_string(),
                    label: "5h 滚动窗口".to_string(),
                    duration_hours: 5,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 20_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
                SubscriptionWindowRule {
                    window_id: "7d".to_string(),
                    label: "7d 周额度".to_string(),
                    duration_hours: 168,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 100_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            updated_at_ms: 0,
        },
    ]
}

fn match_observed_snapshot<'a>(
    provider_id: &str,
    rule: &SubscriptionWindowRule,
    observed: &'a [ObservedRateLimitWindow],
    now_ms: i64,
) -> Option<&'a ObservedRateLimitWindow> {
    let duration_minutes = rule.duration_hours * 60;
    observed.iter().find(|obs| {
        if !obs.provider_id.eq_ignore_ascii_case(provider_id) {
            return false;
        }
        // Only apply snapshots whose reset deadline has not expired yet,
        // or that were observed within the window duration when reset time is omitted.
        let fresh = match obs.resets_at_ms {
            Some(reset_ms) => reset_ms > now_ms,
            None => now_ms.saturating_sub(obs.observed_at_ms) < rule.duration_hours * HOUR_MS,
        };
        if !fresh {
            return false;
        }

        if let Some(mins) = obs.window_minutes {
            if mins == duration_minutes {
                return true;
            }
        }

        let name = obs.window_name.to_ascii_lowercase();
        if rule.window_id == "5h" && (name.contains("primary") || name.contains("5h")) {
            return true;
        }
        if rule.window_id == "7d" && (name.contains("secondary") || name.contains("7d")) {
            return true;
        }
        false
    })
}

/// Evaluate all windows of a subscription plan given historical samples for that provider.
pub fn evaluate_subscription_plan(
    plan: &SubscriptionPlan,
    samples: &[WindowUsageSample],
    observed_snapshots: &[ObservedRateLimitWindow],
    now_ms: i64,
) -> SubscriptionPlanStatus {
    let windows = plan
        .windows
        .iter()
        .map(|rule| evaluate_window(&plan.provider_id, rule, samples, observed_snapshots, now_ms))
        .collect();

    SubscriptionPlanStatus {
        plan: plan.clone(),
        windows,
    }
}

/// Evaluate a single subscription quota window.
pub fn evaluate_window(
    provider_id: &str,
    rule: &SubscriptionWindowRule,
    samples: &[WindowUsageSample],
    observed_snapshots: &[ObservedRateLimitWindow],
    now_ms: i64,
) -> SubscriptionWindowStatus {
    let duration_ms = rule.duration_hours.max(1) * HOUR_MS;
    let window_start_ms = if rule.rolling {
        now_ms.saturating_sub(duration_ms)
    } else {
        now_ms.div_euclid(duration_ms) * duration_ms
    };

    let recent_start_ms = now_ms.saturating_sub(HOUR_MS);

    let mut used_value: i64 = 0;
    let mut recent_1h_value: i64 = 0;
    let mut oldest_sample_ts: Option<i64> = None;

    for sample in samples {
        if sample.timestamp_ms < window_start_ms || sample.timestamp_ms > now_ms {
            continue;
        }
        let val = match rule.unit {
            QuotaUnit::MicroUsd => sample.cost_micro_usd,
            QuotaUnit::Tokens => sample.total_tokens,
        };
        if val <= 0 {
            continue;
        }
        used_value = used_value.saturating_add(val);
        if sample.timestamp_ms >= recent_start_ms {
            recent_1h_value = recent_1h_value.saturating_add(val);
        }
        oldest_sample_ts = Some(match oldest_sample_ts {
            Some(existing) => existing.min(sample.timestamp_ms),
            None => sample.timestamp_ms,
        });
    }

    let safe_limit = rule.limit_value.max(1);
    let mut used_percent = (used_value as f64 / safe_limit as f64) * 100.0;
    let mut resets_at_ms = if rule.rolling {
        oldest_sample_ts.map(|ts| ts.saturating_add(duration_ms))
    } else {
        Some(window_start_ms.saturating_add(duration_ms))
    };
    let mut source = "local_estimation".to_string();

    if let Some(obs) = match_observed_snapshot(provider_id, rule, observed_snapshots, now_ms) {
        used_percent = obs.used_percent.max(0.0);
        if let Some(obs_reset) = obs.resets_at_ms {
            resets_at_ms = Some(obs_reset);
        }
        used_value = ((used_percent / 100.0) * safe_limit as f64).round() as i64;
        source = "official_log_snapshot".to_string();
    }

    let resets_in_seconds = resets_at_ms
        .map(|reset_ts| ((reset_ts - now_ms) / 1000).max(0))
        .unwrap_or(0);

    let burn_rate_per_hour = recent_1h_value as f64;
    let projected_exhaustion_hours = if used_percent >= 100.0 {
        Some(0.0)
    } else if burn_rate_per_hour > 0.0 {
        let remaining_value = if source == "official_log_snapshot" {
            ((100.0 - used_percent) / 100.0) * safe_limit as f64
        } else {
            (safe_limit - used_value).max(0) as f64
        };
        Some(remaining_value / burn_rate_per_hour)
    } else {
        None
    };

    let alert_level = if used_percent >= 100.0 {
        "exhausted"
    } else if used_percent >= f64::from(rule.critical_percent) {
        "critical"
    } else if used_percent >= f64::from(rule.warning_percent) {
        "warning"
    } else {
        "normal"
    }
    .to_string();

    SubscriptionWindowStatus {
        window_id: rule.window_id.clone(),
        label: rule.label.clone(),
        duration_hours: rule.duration_hours,
        rolling: rule.rolling,
        unit: rule.unit,
        used_value,
        limit_value: rule.limit_value,
        used_percent,
        warning_percent: rule.warning_percent,
        critical_percent: rule.critical_percent,
        burn_rate_per_hour,
        projected_exhaustion_hours,
        resets_at_ms,
        resets_in_seconds,
        source,
        alert_level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_window_calculates_usage_countdown_and_projection() {
        let plans = default_subscription_plans();
        let pro = plans
            .iter()
            .find(|p| p.plan_id == "claude_pro")
            .expect("claude_pro exists");
        let now_ms = 1_760_000_000_000i64;
        // 3 hours ago spent $6, 20 mins ago spent $6 -> total $12 / $15 = 80% warning!
        // Recent 1h burn rate = $6/hr, remaining = $3 -> 0.5h exhaustion!
        // Oldest sample 3h ago rolls off in 2h (7200s).
        let samples = vec![
            WindowUsageSample {
                timestamp_ms: now_ms - 3 * HOUR_MS,
                cost_micro_usd: 6_000_000,
                total_tokens: 1_000_000,
            },
            WindowUsageSample {
                timestamp_ms: now_ms - 1_200_000,
                cost_micro_usd: 6_000_000,
                total_tokens: 1_000_000,
            },
            // Outside 5h window: should be ignored by 5h window, counted by 7d window
            WindowUsageSample {
                timestamp_ms: now_ms - 10 * HOUR_MS,
                cost_micro_usd: 10_000_000,
                total_tokens: 2_000_000,
            },
        ];

        let status = evaluate_subscription_plan(pro, &samples, &[], now_ms);
        let w5 = &status.windows[0];
        assert!((w5.used_percent - 80.0).abs() < 0.01);
        assert_eq!(w5.alert_level, "warning");
        assert_eq!(w5.resets_in_seconds, 2 * 3600);
        let proj = w5.projected_exhaustion_hours.expect("projected");
        assert!((proj - 0.5).abs() < 0.01);

        let w7 = &status.windows[1];
        assert_eq!(w7.used_value, 22_000_000);
    }

    #[test]
    fn fresh_official_snapshot_overrides_local_percent_for_codex() {
        let plans = default_subscription_plans();
        let codex = plans
            .iter()
            .find(|p| p.plan_id == "codex_plus")
            .expect("codex_plus exists");
        let now_ms = 1_760_000_000_000i64;
        let obs = vec![ObservedRateLimitWindow {
            provider_id: provider_ids::CODEX.to_string(),
            window_name: "primary".to_string(),
            used_percent: 42.5,
            window_minutes: Some(300),
            resets_at_ms: Some(now_ms + 3_600_000),
            observed_at_ms: now_ms - 30_000,
        }];

        let status = evaluate_subscription_plan(codex, &[], &obs, now_ms);
        let w5 = &status.windows[0];
        assert_eq!(w5.source, "official_log_snapshot");
        assert!((w5.used_percent - 42.5).abs() < 0.01);
        assert_eq!(w5.resets_in_seconds, 3600);
    }
}
