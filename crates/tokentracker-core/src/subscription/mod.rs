use serde::{Deserialize, Serialize};

pub use crate::domain::SubscriptionPlanTemplate;
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

fn standard_two_windows(limit_5h_micro_usd: i64, limit_7d_micro_usd: i64) -> Vec<SubscriptionWindowRule> {
    vec![
        SubscriptionWindowRule {
            window_id: "5h".to_string(),
            label: "5h 滚动窗口".to_string(),
            duration_hours: 5,
            rolling: true,
            unit: QuotaUnit::MicroUsd,
            limit_value: limit_5h_micro_usd,
            warning_percent: 80,
            critical_percent: 95,
        },
        SubscriptionWindowRule {
            window_id: "7d".to_string(),
            label: "7d 周额度".to_string(),
            duration_hours: 168,
            rolling: true,
            unit: QuotaUnit::MicroUsd,
            limit_value: limit_7d_micro_usd,
            warning_percent: 80,
            critical_percent: 95,
        },
    ]
}

fn template_entry(
    template_id: &str,
    provider_id: &str,
    plan_name: &str,
    monthly_fee_micro_usd: i64,
    limit_5h_micro_usd: i64,
    limit_7d_micro_usd: i64,
    description: &str,
) -> SubscriptionPlanTemplate {
    SubscriptionPlanTemplate {
        template_id: template_id.to_string(),
        provider_id: provider_id.to_string(),
        plan_name: plan_name.to_string(),
        monthly_fee_micro_usd,
        windows: standard_two_windows(limit_5h_micro_usd, limit_7d_micro_usd),
        description: description.to_string(),
    }
}

/// Optional reference subscription templates offered in the UI drawer for quick prefill.
/// Never auto-seeded into SQLite on startup (`PLAN.md` §3.2 zero-assumption principle).
pub fn subscription_plan_templates() -> Vec<SubscriptionPlanTemplate> {
    vec![
        template_entry(
            "claude_pro",
            provider_ids::CLAUDE_CODE,
            "Claude Pro ($20/mo)",
            20_000_000,
            15_000_000,
            75_000_000,
            "Anthropic Claude Pro 个人订阅（含 5h 滚动与 7d 周参考窗口）",
        ),
        template_entry(
            "claude_max_5x",
            provider_ids::CLAUDE_CODE,
            "Claude Max 5x ($100/mo)",
            100_000_000,
            75_000_000,
            375_000_000,
            "Anthropic Claude Max 5x 高配额订阅（5 倍 Pro 参考额度）",
        ),
        template_entry(
            "claude_max_20x",
            provider_ids::CLAUDE_CODE,
            "Claude Max 20x ($200/mo)",
            200_000_000,
            200_000_000,
            1_200_000_000,
            "Anthropic Claude Max 20x 重度编码订阅（20 倍 Pro 参考额度）",
        ),
        template_entry(
            "codex_plus",
            provider_ids::CODEX,
            "ChatGPT Plus / Codex ($20/mo)",
            20_000_000,
            18_000_000,
            90_000_000,
            "OpenAI ChatGPT Plus（可自动对齐 Codex 日志官方 5h / 7d 限额）",
        ),
        template_entry(
            "codex_pro",
            provider_ids::CODEX,
            "ChatGPT Pro / Codex ($200/mo)",
            200_000_000,
            150_000_000,
            900_000_000,
            "OpenAI ChatGPT Pro（可自动对齐 Codex 日志官方 5h / 7d 限额）",
        ),
        template_entry(
            "google_ai_pro",
            provider_ids::ANTIGRAVITY,
            "Google AI Pro ($19.99/mo)",
            19_990_000,
            20_000_000,
            100_000_000,
            "Google AI Pro / Antigravity 个人订阅（19.99 USD/月参考窗口）",
        ),
        template_entry(
            "google_ai_ultra",
            provider_ids::ANTIGRAVITY,
            "Google AI Ultra ($249.99/mo)",
            249_990_000,
            250_000_000,
            1_250_000_000,
            "Google AI Ultra / Antigravity 旗舰订阅（249.99 USD/月高额度窗口）",
        ),
        template_entry(
            "minimax_coding_plan",
            provider_ids::MINIMAX_CODE,
            "MiniMax Coding Plan ($15/mo)",
            15_000_000,
            12_000_000,
            60_000_000,
            "MiniMax Coding Plan 编码订阅参考窗口（支持官方 Coding Plan 额度接口对齐）",
        ),
        SubscriptionPlanTemplate {
            template_id: "minimax_token_plan".to_string(),
            provider_id: provider_ids::MINIMAX_CODE.to_string(),
            plan_name: "MiniMax Token Plan ($49/mo)".to_string(),
            monthly_fee_micro_usd: 49_000_000,
            windows: vec![
                SubscriptionWindowRule {
                    window_id: "5h".to_string(),
                    label: "5h Token 窗口".to_string(),
                    duration_hours: 5,
                    rolling: true,
                    unit: QuotaUnit::Tokens,
                    limit_value: 20_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
                SubscriptionWindowRule {
                    window_id: "30d".to_string(),
                    label: "月度 Token 额度".to_string(),
                    duration_hours: 720,
                    rolling: false,
                    unit: QuotaUnit::Tokens,
                    limit_value: 120_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            description: "MiniMax Token Plan 按 Token 计量订阅参考模板（可按需修改额度）".to_string(),
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

    fn plan_from_template(template_id: &str) -> SubscriptionPlan {
        let tpl = subscription_plan_templates()
            .into_iter()
            .find(|t| t.template_id == template_id)
            .expect("template exists");
        SubscriptionPlan {
            plan_id: tpl.template_id,
            provider_id: tpl.provider_id,
            plan_name: tpl.plan_name,
            enabled: true,
            monthly_fee_micro_usd: tpl.monthly_fee_micro_usd,
            windows: tpl.windows,
            is_user_configured: true,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn templates_cover_all_reference_plans() {
        let templates = subscription_plan_templates();
        assert_eq!(templates.len(), 9);
        let ids: Vec<&str> = templates.iter().map(|t| t.template_id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "claude_pro",
                "claude_max_5x",
                "claude_max_20x",
                "codex_plus",
                "codex_pro",
                "google_ai_pro",
                "google_ai_ultra",
                "minimax_coding_plan",
                "minimax_token_plan",
            ]
        );
        assert_eq!(templates[5].monthly_fee_micro_usd, 19_990_000);
        assert_eq!(templates[6].monthly_fee_micro_usd, 249_990_000);
        assert_eq!(templates[7].provider_id, provider_ids::MINIMAX_CODE);
        assert_eq!(templates[8].windows[0].unit, QuotaUnit::Tokens);
    }

    #[test]
    fn rolling_window_calculates_usage_countdown_and_projection() {
        let pro = plan_from_template("claude_pro");
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

        let status = evaluate_subscription_plan(&pro, &samples, &[], now_ms);
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
        let codex = plan_from_template("codex_plus");
        let now_ms = 1_760_000_000_000i64;
        let obs = vec![ObservedRateLimitWindow {
            provider_id: provider_ids::CODEX.to_string(),
            window_name: "primary".to_string(),
            used_percent: 42.5,
            window_minutes: Some(300),
            resets_at_ms: Some(now_ms + 3_600_000),
            observed_at_ms: now_ms - 30_000,
        }];

        let status = evaluate_subscription_plan(&codex, &[], &obs, now_ms);
        let w5 = &status.windows[0];
        assert_eq!(w5.source, "official_log_snapshot");
        assert!((w5.used_percent - 42.5).abs() < 0.01);
        assert_eq!(w5.resets_in_seconds, 3600);
    }
}
