use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use crate::domain::{SubscriptionPlanTemplate, SubscriptionWindowStatus};
use crate::domain::{
    provider_ids, ObservedRateLimitWindow, QuotaUnit, SubscriptionPlan, SubscriptionWindowRule,
};
use crate::error::{CoreError, CoreResult};

const HOUR_MS: i64 = 3_600_000;
const MAX_MONTHLY_FEE_MICRO_USD: i64 = 10_000_000_000; // $10,000 / mo sanity cap

/// Single event contribution sample used for rolling window evaluation.
#[derive(Debug, Clone, Copy)]
pub struct WindowUsageSample {
    pub timestamp_ms: i64,
    pub cost_micro_usd: i64,
    pub total_tokens: i64,
}

/// Evaluated live status for a subscription plan across authoritative quota windows + factual spend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionPlanStatus {
    pub plan: SubscriptionPlan,
    pub windows: Vec<SubscriptionWindowStatus>,
    #[serde(default)]
    pub today_cost_micro_usd: i64,
    #[serde(default)]
    pub week_cost_micro_usd: i64,
    #[serde(default)]
    pub month_cost_micro_usd: i64,
    #[serde(default)]
    pub month_tokens: i64,
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

fn standard_three_windows(
    limit_5h_micro_usd: i64,
    limit_7d_micro_usd: i64,
    limit_30d_micro_usd: i64,
) -> Vec<SubscriptionWindowRule> {
    let mut wins = standard_two_windows(limit_5h_micro_usd, limit_7d_micro_usd);
    wins.push(SubscriptionWindowRule {
        window_id: "30d".to_string(),
        label: "30d 月度限额".to_string(),
        duration_hours: 720,
        rolling: false,
        unit: QuotaUnit::MicroUsd,
        limit_value: limit_30d_micro_usd,
        warning_percent: 80,
        critical_percent: 95,
    });
    wins
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
        live_windows: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn template_entry_with_monthly(
    template_id: &str,
    provider_id: &str,
    plan_name: &str,
    monthly_fee_micro_usd: i64,
    limit_5h_micro_usd: i64,
    limit_7d_micro_usd: i64,
    limit_30d_micro_usd: i64,
    description: &str,
) -> SubscriptionPlanTemplate {
    SubscriptionPlanTemplate {
        template_id: template_id.to_string(),
        provider_id: provider_id.to_string(),
        plan_name: plan_name.to_string(),
        monthly_fee_micro_usd,
        windows: standard_three_windows(
            limit_5h_micro_usd,
            limit_7d_micro_usd,
            limit_30d_micro_usd,
        ),
        description: description.to_string(),
        live_windows: Vec::new(),
    }
}

/// Maintained subscription plan catalog offered in the UI for quick prefill
/// and real-time multi-tier quota comparison.
pub fn subscription_plan_templates() -> Vec<SubscriptionPlanTemplate> {
    vec![
        // Anthropic Claude Code
        template_entry_with_monthly(
            "claude_pro",
            provider_ids::CLAUDE_CODE,
            "Claude Pro",
            20_000_000,
            15_000_000,
            75_000_000,
            200_000_000,
            "Anthropic Claude Pro 个人订阅（含 5h 滚动、7d 周与 30d 月参考窗口）",
        ),
        template_entry_with_monthly(
            "claude_team",
            provider_ids::CLAUDE_CODE,
            "Claude Team",
            30_000_000,
            25_000_000,
            125_000_000,
            350_000_000,
            "Anthropic Claude Team 团队席位订阅参考窗口",
        ),
        template_entry_with_monthly(
            "claude_max_5x",
            provider_ids::CLAUDE_CODE,
            "Claude Max 5x",
            100_000_000,
            75_000_000,
            375_000_000,
            1_000_000_000,
            "Anthropic Claude Max 5x 高配额订阅（5 倍 Pro 额度）",
        ),
        template_entry_with_monthly(
            "claude_max_20x",
            provider_ids::CLAUDE_CODE,
            "Claude Max 20x",
            200_000_000,
            200_000_000,
            1_200_000_000,
            3_600_000_000,
            "Anthropic Claude Max 20x 重度编码订阅（20 倍 Pro 额度）",
        ),
        // OpenAI ChatGPT / Codex
        template_entry(
            "codex_plus",
            provider_ids::CODEX,
            "ChatGPT Plus / Codex",
            20_000_000,
            18_000_000,
            90_000_000,
            "OpenAI ChatGPT Plus（自动对齐 Codex 日志官方 5h / 7d 限额）",
        ),
        template_entry(
            "codex_team",
            provider_ids::CODEX,
            "ChatGPT Team / Codex",
            25_000_000,
            30_000_000,
            150_000_000,
            "OpenAI ChatGPT Team 团队席位（自动对齐 Codex 日志官方 5h / 7d 限额）",
        ),
        template_entry(
            "codex_pro",
            provider_ids::CODEX,
            "ChatGPT Pro / Codex",
            200_000_000,
            150_000_000,
            900_000_000,
            "OpenAI ChatGPT Pro 旗舰开发订阅（自动对齐 Codex 日志官方 5h / 7d 限额）",
        ),
        // Google AI / Antigravity
        SubscriptionPlanTemplate {
            template_id: "google_ai_free".to_string(),
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            plan_name: "Google AI Free".to_string(),
            monthly_fee_micro_usd: 0,
            windows: vec![SubscriptionWindowRule {
                window_id: "7d".to_string(),
                label: "7d 周额度".to_string(),
                duration_hours: 168,
                rolling: true,
                unit: QuotaUnit::MicroUsd,
                limit_value: 15_000_000,
                warning_percent: 80,
                critical_percent: 95,
            }],
            description: "Google AI Free / Antigravity 个人免费周额度窗口".to_string(),
            live_windows: Vec::new(),
        },
        template_entry(
            "google_ai_plus",
            provider_ids::ANTIGRAVITY,
            "Google AI Plus",
            4_990_000,
            6_000_000,
            30_000_000,
            "Google AI Plus / Antigravity 基础订阅参考窗口",
        ),
        template_entry(
            "google_ai_pro",
            provider_ids::ANTIGRAVITY,
            "Google AI Pro",
            19_990_000,
            20_000_000,
            100_000_000,
            "Google AI Pro / Antigravity 标准开发订阅（5h 滚动 + 7d 周额度）",
        ),
        template_entry(
            "google_ai_ultra_5x",
            provider_ids::ANTIGRAVITY,
            "Google AI Ultra 5x",
            99_990_000,
            100_000_000,
            500_000_000,
            "Google AI Ultra 5x / Antigravity 进阶开发订阅（5 倍 Pro 额度）",
        ),
        template_entry(
            "google_ai_ultra",
            provider_ids::ANTIGRAVITY,
            "Google AI Ultra",
            199_990_000,
            250_000_000,
            1_250_000_000,
            "Google AI Ultra / Antigravity 旗舰订阅（20 倍 Pro 额度）",
        ),
        template_entry(
            "gemini_enterprise",
            provider_ids::ANTIGRAVITY,
            "Gemini Enterprise",
            30_000_000,
            40_000_000,
            200_000_000,
            "Gemini Enterprise / Code Assist 企业席位参考窗口",
        ),
        // Kimi Code (Moonshot AI) — Plus/Moderato, Pro/Allegretto, Max/Allegro, Ultra/Vivace with 5h + 7d + 30d monthly limits
        template_entry_with_monthly(
            "kimi_code_plus",
            provider_ids::KIMI_CODE,
            "Kimi Plus (Moderato)",
            19_000_000,
            8_000_000,
            40_000_000,
            120_000_000,
            "Kimi Plus / Moderato 个人订阅（含 5h 滚动、7d 周与 30d 月度限额）",
        ),
        template_entry_with_monthly(
            "kimi_code_allegretto",
            provider_ids::KIMI_CODE,
            "Kimi Pro (Allegretto)",
            39_000_000,
            25_000_000,
            125_000_000,
            400_000_000,
            "Kimi Pro / Allegretto 进阶编码会员（含 5h 滚动、7d 周与 30d 月度限额）",
        ),
        template_entry_with_monthly(
            "kimi_code_max",
            provider_ids::KIMI_CODE,
            "Kimi Max (Allegro)",
            99_000_000,
            75_000_000,
            375_000_000,
            1_200_000_000,
            "Kimi Max / Allegro 高并发编码会员（含 5h 滚动、7d 周与 30d 月度限额）",
        ),
        template_entry_with_monthly(
            "kimi_code_ultra",
            provider_ids::KIMI_CODE,
            "Kimi Ultra (Vivace)",
            199_000_000,
            180_000_000,
            900_000_000,
            2_800_000_000,
            "Kimi Ultra / Vivace 旗舰编码会员（含 5h 滚动、7d 周与 30d 月度限额）",
        ),
        // MiniMax Code
        template_entry_with_monthly(
            "minimax_coding_plus",
            provider_ids::MINIMAX_CODE,
            "MiniMax Coding Plus",
            7_000_000,
            8_000_000,
            40_000_000,
            120_000_000,
            "MiniMax Coding Plus 基础编码套餐参考窗口",
        ),
        template_entry_with_monthly(
            "minimax_coding_plan",
            provider_ids::MINIMAX_CODE,
            "MiniMax Coding Pro",
            15_000_000,
            18_000_000,
            90_000_000,
            280_000_000,
            "MiniMax Coding Pro 编码订阅参考窗口（支持官方 Coding Plan 额度接口自动同步）",
        ),
        SubscriptionPlanTemplate {
            template_id: "minimax_token_plan".to_string(),
            provider_id: provider_ids::MINIMAX_CODE.to_string(),
            plan_name: "MiniMax Coding Max (Token Plan)".to_string(),
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
                    label: "30d 月度限额".to_string(),
                    duration_hours: 720,
                    rolling: false,
                    unit: QuotaUnit::Tokens,
                    limit_value: 120_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            description: "MiniMax Token Plan 按 Token 计量订阅参考模板".to_string(),
            live_windows: Vec::new(),
        },
        // QCode
        SubscriptionPlanTemplate {
            template_id: "qcode_daily_plan".to_string(),
            provider_id: provider_ids::QCODE.to_string(),
            plan_name: "QCode 标准日额度套餐".to_string(),
            monthly_fee_micro_usd: 15_000_000,
            windows: vec![
                SubscriptionWindowRule {
                    window_id: "24h".to_string(),
                    label: "24h 日额度".to_string(),
                    duration_hours: 24,
                    rolling: false,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 10_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
                SubscriptionWindowRule {
                    window_id: "30d".to_string(),
                    label: "30d 月度限额".to_string(),
                    duration_hours: 720,
                    rolling: false,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 200_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            description: "QCode 标准每日与月度额度参考模板".to_string(),
            live_windows: Vec::new(),
        },
        SubscriptionPlanTemplate {
            template_id: "qcode_pro_plan".to_string(),
            provider_id: provider_ids::QCODE.to_string(),
            plan_name: "QCode Pro 日额度套餐".to_string(),
            monthly_fee_micro_usd: 45_000_000,
            windows: vec![
                SubscriptionWindowRule {
                    window_id: "24h".to_string(),
                    label: "24h 日额度".to_string(),
                    duration_hours: 24,
                    rolling: false,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 35_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
                SubscriptionWindowRule {
                    window_id: "30d".to_string(),
                    label: "30d 月度限额".to_string(),
                    duration_hours: 720,
                    rolling: false,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 700_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            description: "QCode Pro 高配额每日与月度额度参考模板".to_string(),
            live_windows: Vec::new(),
        },
        // Workbuddy
        SubscriptionPlanTemplate {
            template_id: "workbuddy_pro".to_string(),
            provider_id: provider_ids::WORKBUDDY.to_string(),
            plan_name: "Workbuddy Pro".to_string(),
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
                    window_id: "30d".to_string(),
                    label: "30d 月度限额".to_string(),
                    duration_hours: 720,
                    rolling: false,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 150_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ],
            description: "Workbuddy Pro 编码助手订阅参考模板".to_string(),
            live_windows: Vec::new(),
        },
    ]
}

/// Format an authoritative window label from an observed bucket identifier and optional window minutes.
pub fn format_authoritative_window_label(window_name: &str, window_minutes: Option<i64>) -> String {
    let trimmed = window_name.trim();
    if trimmed.contains(" · ") {
        return trimmed.to_string();
    }
    let lower = trimmed.to_ascii_lowercase();
    let dur_label = match window_minutes {
        Some(m) if (240..=360).contains(&m) => "5h 限额".to_string(),
        Some(m) if (1_200..=1_680).contains(&m) => "24h 日限额".to_string(),
        Some(m) if (9_600..=10_560).contains(&m) => "7d 周限额".to_string(),
        Some(m) if m >= 40_000 => "30d 月限额".to_string(),
        Some(m) => format!("{}h 限额", (m / 60).max(1)),
        None if lower.contains("5h") || lower.contains("primary") => "5h 限额".to_string(),
        None if lower.contains("week") || lower.contains("7d") || lower.contains("secondary") => {
            "7d 周限额".to_string()
        }
        None if lower.contains("daily") || lower.contains("24h") || lower.contains("day") => {
            "24h 日限额".to_string()
        }
        None if lower.contains("30d") || lower.contains("month") => "30d 月限额".to_string(),
        None => "官方限额".to_string(),
    };

    if lower.starts_with("gemini-") || lower.contains("gemini") {
        format!("Gemini 模型池 · {dur_label}")
    } else if lower.starts_with("3p-")
        || lower.contains("claude")
        || lower.contains("gpt")
        || lower.contains("third")
    {
        format!("Claude / GPT 模型池 · {dur_label}")
    } else {
        dur_label
    }
}

pub(crate) fn is_pool_qualified_window_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    lower.starts_with("gemini-")
        || lower.starts_with("3p-")
        || name.contains(" · ")
        || lower.contains("::")
}

fn generic_bucket_matches(window_id: &str, observed_name: &str) -> bool {
    let name = observed_name.to_ascii_lowercase();
    let wid = window_id.to_ascii_lowercase();
    if wid == "5h" && (name.contains("primary") || name.contains("5h")) {
        return true;
    }
    if (wid == "7d" || wid == "weekly")
        && (name.contains("secondary") || name.contains("7d") || name.contains("week"))
    {
        return true;
    }
    if (wid == "24h" || wid == "1d" || wid == "daily")
        && (name.contains("24h") || name.contains("daily") || name.contains("day"))
    {
        return true;
    }
    if (wid == "30d" || wid == "monthly") && (name.contains("30d") || name.contains("month")) {
        return true;
    }
    false
}

fn observed_snapshot_is_fresh(
    obs: &ObservedRateLimitWindow,
    fallback_hours: i64,
    now_ms: i64,
) -> bool {
    match obs.resets_at_ms {
        Some(reset_ms) => reset_ms > now_ms,
        None => now_ms.saturating_sub(obs.observed_at_ms) < fallback_hours.max(1) * HOUR_MS,
    }
}

fn match_observed_snapshot<'a>(
    provider_id: &str,
    rule: &SubscriptionWindowRule,
    observed: &'a [ObservedRateLimitWindow],
    now_ms: i64,
) -> Option<&'a ObservedRateLimitWindow> {
    let duration_minutes = rule.duration_hours * 60;
    observed
        .iter()
        .filter(|obs| {
            if !obs.provider_id.eq_ignore_ascii_case(provider_id) {
                return false;
            }
            if obs.source.as_deref() == Some("local_log_hint") || obs.window_name == "tier_hint" {
                return false;
            }
            if !observed_snapshot_is_fresh(obs, rule.duration_hours, now_ms) {
                return false;
            }

            if obs.window_name.eq_ignore_ascii_case(&rule.window_id) {
                return true;
            }

            if is_pool_qualified_window_name(&obs.window_name)
                || is_pool_qualified_window_name(&rule.window_id)
            {
                return false;
            }

            if let Some(mins) = obs.window_minutes {
                if mins == duration_minutes {
                    return true;
                }
            }

            generic_bucket_matches(&rule.window_id, &obs.window_name)
        })
        .max_by(|a, b| {
            a.observed_at_ms.cmp(&b.observed_at_ms).then_with(|| {
                a.used_percent
                    .partial_cmp(&b.used_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        })
}

fn append_unmatched_authoritative_windows(
    provider_id: &str,
    windows: &mut Vec<SubscriptionWindowStatus>,
    samples: &[WindowUsageSample],
    observed_snapshots: &[ObservedRateLimitWindow],
    now_ms: i64,
) {
    let mut best_by_name: std::collections::BTreeMap<String, &ObservedRateLimitWindow> =
        std::collections::BTreeMap::new();
    for obs in observed_snapshots {
        if !obs.provider_id.eq_ignore_ascii_case(provider_id) {
            continue;
        }
        if obs.source.as_deref() == Some("local_log_hint") || obs.window_name == "tier_hint" {
            continue;
        }
        let fallback_hours = obs
            .window_minutes
            .map(|m| (m / 60).max(1))
            .unwrap_or_else(|| {
                let lower = obs.window_name.to_ascii_lowercase();
                if lower.contains("week") || lower.contains("7d") {
                    168
                } else {
                    5
                }
            });
        if !observed_snapshot_is_fresh(obs, fallback_hours, now_ms) {
            continue;
        }
        let key = obs.window_name.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        let pool_qualified = is_pool_qualified_window_name(&obs.window_name);
        let already_matched = windows.iter().any(|w| {
            w.window_id.eq_ignore_ascii_case(&obs.window_name)
                || (!pool_qualified
                    && !is_pool_qualified_window_name(&w.window_id)
                    && (obs.window_minutes == Some(w.duration_hours * 60)
                        || generic_bucket_matches(&w.window_id, &obs.window_name)))
        });
        if already_matched {
            continue;
        }
        match best_by_name.get(&key) {
            Some(existing) if existing.observed_at_ms >= obs.observed_at_ms => {}
            _ => {
                best_by_name.insert(key, obs);
            }
        }
    }

    for obs in best_by_name.into_values() {
        let lower = obs.window_name.to_ascii_lowercase();
        let duration_hours = obs
            .window_minutes
            .map(|m| (m / 60).max(1))
            .unwrap_or_else(|| {
                if lower.contains("week") || lower.contains("7d") {
                    168
                } else if lower.contains("daily") || lower.contains("24h") {
                    24
                } else if lower.contains("month") || lower.contains("30d") {
                    720
                } else {
                    5
                }
            });
        let rule = SubscriptionWindowRule {
            window_id: obs.window_name.clone(),
            label: format_authoritative_window_label(&obs.window_name, obs.window_minutes),
            duration_hours,
            rolling: duration_hours <= 168,
            unit: QuotaUnit::Tokens,
            limit_value: 100,
            warning_percent: 80,
            critical_percent: 95,
        };
        windows.push(evaluate_window(
            provider_id,
            &rule,
            samples,
            std::slice::from_ref(obs),
            now_ms,
        ));
    }

    if windows
        .iter()
        .any(|w| is_pool_qualified_window_name(&w.window_id))
    {
        windows.sort_by(|a, b| {
            let a_3p = a.window_id.to_ascii_lowercase().starts_with("3p-");
            let b_3p = b.window_id.to_ascii_lowercase().starts_with("3p-");
            a_3p.cmp(&b_3p)
                .then_with(|| a.duration_hours.cmp(&b.duration_hours))
                .then_with(|| a.window_id.cmp(&b.window_id))
        });
    }
}

/// Evaluate all reference templates against current live authoritative snapshots.
pub fn evaluate_subscription_templates(
    samples_map: &HashMap<String, Vec<WindowUsageSample>>,
    observed_snapshots: &[ObservedRateLimitWindow],
    now_ms: i64,
) -> Vec<SubscriptionPlanTemplate> {
    evaluate_subscription_templates_from_list(
        subscription_plan_templates(),
        samples_map,
        observed_snapshots,
        now_ms,
    )
}

/// Evaluate a supplied template catalog list against authoritative observed rate-limit snapshots.
/// Unverified local-estimated windows are intentionally omitted from `live_windows` to prevent
/// misleading fabricated percentages on providers without an official quota endpoint.
pub fn evaluate_subscription_templates_from_list(
    templates: Vec<SubscriptionPlanTemplate>,
    samples_map: &HashMap<String, Vec<WindowUsageSample>>,
    observed_snapshots: &[ObservedRateLimitWindow],
    now_ms: i64,
) -> Vec<SubscriptionPlanTemplate> {
    templates
        .into_iter()
        .map(|mut tpl| {
            let samples = samples_map
                .get(&tpl.provider_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut live_windows: Vec<SubscriptionWindowStatus> = tpl
                .windows
                .iter()
                .map(|rule| {
                    evaluate_window(&tpl.provider_id, rule, samples, observed_snapshots, now_ms)
                })
                .filter(|w| w.source != "local_estimation")
                .collect();
            append_unmatched_authoritative_windows(
                &tpl.provider_id,
                &mut live_windows,
                samples,
                observed_snapshots,
                now_ms,
            );
            tpl.live_windows = live_windows;
            tpl
        })
        .collect()
}

/// Evaluate a subscription plan. Only windows backed by fresh authoritative snapshots
/// (`official_api`, `official_cli`, or `official_log_snapshot`) are emitted; plans without
/// an official quota source report an empty `windows` vector and track factual spend + ROI instead.
pub fn evaluate_subscription_plan(
    plan: &SubscriptionPlan,
    samples: &[WindowUsageSample],
    observed_snapshots: &[ObservedRateLimitWindow],
    now_ms: i64,
) -> SubscriptionPlanStatus {
    let mut windows: Vec<SubscriptionWindowStatus> = plan
        .windows
        .iter()
        .map(|rule| evaluate_window(&plan.provider_id, rule, samples, observed_snapshots, now_ms))
        .filter(|w| w.source != "local_estimation")
        .collect();
    append_unmatched_authoritative_windows(
        &plan.provider_id,
        &mut windows,
        samples,
        observed_snapshots,
        now_ms,
    );

    SubscriptionPlanStatus {
        plan: plan.clone(),
        windows,
        today_cost_micro_usd: 0,
        week_cost_micro_usd: 0,
        month_cost_micro_usd: 0,
        month_tokens: 0,
    }
}

#[derive(Debug, Deserialize)]
struct RemoteCatalogPlanRaw {
    template_id: String,
    provider_id: String,
    plan_name: String,
    monthly_fee_micro_usd: i64,
    #[serde(default)]
    windows: Vec<SubscriptionWindowRule>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteCatalogDocumentRaw {
    #[serde(default)]
    plans: Vec<RemoteCatalogPlanRaw>,
}

/// Parse either an `AI Subscription Catalog` full document (`plans.json` / `subscription-plans.json`)
/// or a TokenTracker templates array (`tokentracker-templates.json`), filtering legacy entries and
/// normalizing plan metadata cleanly.
pub fn parse_subscription_catalog_json(raw_json: &str) -> CoreResult<Vec<SubscriptionPlanTemplate>> {
    let trimmed = raw_json.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Validation(
            "Empty subscription catalog JSON payload".to_string(),
        ));
    }

    let raw_plans: Vec<RemoteCatalogPlanRaw> = if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).map_err(|e| {
            CoreError::Validation(format!("Invalid subscription templates array JSON: {e}"))
        })?
    } else {
        let doc: RemoteCatalogDocumentRaw = serde_json::from_str(trimmed).map_err(|e| {
            CoreError::Validation(format!("Invalid subscription catalog document JSON: {e}"))
        })?;
        doc.plans
    };

    let mut seen_ids = HashSet::new();
    let mut out = Vec::with_capacity(raw_plans.len());

    for raw in raw_plans {
        if raw
            .status
            .as_deref()
            .is_some_and(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "legacy" | "deprecated"))
        {
            continue;
        }
        let template_id = raw.template_id.trim().to_string();
        let provider_id = raw.provider_id.trim().to_ascii_lowercase();
        let mut plan_name = raw.plan_name.trim().to_string();
        let mut description = raw.description.trim().to_string();
        if template_id.is_empty() || provider_id.is_empty() || plan_name.is_empty() {
            continue;
        }
        if raw.monthly_fee_micro_usd < 0 || raw.monthly_fee_micro_usd > MAX_MONTHLY_FEE_MICRO_USD {
            continue;
        }
        if !seen_ids.insert(template_id.clone()) {
            continue;
        }

        // Preserve v0.2.5 canonical naming for Google AI Ultra even against v0.2.4-derived upstream tags.
        if template_id == "google_ai_ultra" && plan_name == "Google AI Ultra 20x" {
            plan_name = "Google AI Ultra".to_string();
            description = description.replace("Google AI Ultra 20x", "Google AI Ultra");
        }

        let windows: Vec<SubscriptionWindowRule> = raw
            .windows
            .into_iter()
            .filter(|w| {
                !w.window_id.trim().is_empty()
                    && !w.label.trim().is_empty()
                    && w.duration_hours > 0
                    && w.limit_value >= 0
            })
            .map(|mut w| {
                w.window_id = w.window_id.trim().to_string();
                w.label = w.label.trim().to_string();
                w
            })
            .collect();

        out.push(SubscriptionPlanTemplate {
            template_id,
            provider_id,
            plan_name,
            monthly_fee_micro_usd: raw.monthly_fee_micro_usd,
            windows,
            description,
            live_windows: Vec::new(),
        });
    }

    if out.len() < 10 {
        return Err(CoreError::Validation(format!(
            "Subscription catalog sanity check failed: expected at least 10 active plans, parsed {}",
            out.len()
        )));
    }

    Ok(out)
}

/// Compute a deterministic SHA-256 hash over an ordered list of subscription templates.
pub fn compute_subscription_catalog_hash(templates: &[SubscriptionPlanTemplate]) -> String {
    let mut hasher = Sha256::new();
    for tpl in templates {
        hasher.update(tpl.template_id.as_bytes());
        hasher.update(b"\x1f");
        hasher.update(tpl.provider_id.as_bytes());
        hasher.update(b"\x1f");
        hasher.update(tpl.plan_name.as_bytes());
        hasher.update(b"\x1f");
        hasher.update(tpl.monthly_fee_micro_usd.to_le_bytes());
        hasher.update(b"\x1f");
        hasher.update(tpl.description.as_bytes());
        for w in &tpl.windows {
            hasher.update(b"\x1e");
            hasher.update(w.window_id.as_bytes());
            hasher.update(w.duration_hours.to_le_bytes());
            hasher.update([u8::from(w.rolling)]);
            hasher.update(w.limit_value.to_le_bytes());
        }
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
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
        source = match obs.source.as_deref() {
            Some("official_api") => "official_api".to_string(),
            Some("official_cli") => "official_cli".to_string(),
            _ => "official_log_snapshot".to_string(),
        };
    }

    let resets_in_seconds = resets_at_ms
        .map(|reset_ts| ((reset_ts - now_ms) / 1000).max(0))
        .unwrap_or(0);

    let burn_rate_per_hour = recent_1h_value as f64;
    let projected_exhaustion_hours = if used_percent >= 100.0 {
        Some(0.0)
    } else if burn_rate_per_hour > 0.0 {
        let remaining_value = if source != "local_estimation" {
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
            auto_source: None,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn templates_cover_all_reference_plans() {
        let templates = subscription_plan_templates();
        assert_eq!(templates.len(), 23);
        let ids: Vec<&str> = templates.iter().map(|t| t.template_id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "claude_pro",
                "claude_team",
                "claude_max_5x",
                "claude_max_20x",
                "codex_plus",
                "codex_team",
                "codex_pro",
                "google_ai_free",
                "google_ai_plus",
                "google_ai_pro",
                "google_ai_ultra_5x",
                "google_ai_ultra",
                "gemini_enterprise",
                "kimi_code_plus",
                "kimi_code_allegretto",
                "kimi_code_max",
                "kimi_code_ultra",
                "minimax_coding_plus",
                "minimax_coding_plan",
                "minimax_token_plan",
                "qcode_daily_plan",
                "qcode_pro_plan",
                "workbuddy_pro",
            ]
        );
        let ag_pro = templates
            .iter()
            .find(|t| t.template_id == "google_ai_pro")
            .expect("ag_pro");
        assert_eq!(ag_pro.monthly_fee_micro_usd, 19_990_000);
        let ag_ultra_5x = templates
            .iter()
            .find(|t| t.template_id == "google_ai_ultra_5x")
            .expect("ag_ultra_5x");
        assert_eq!(ag_ultra_5x.monthly_fee_micro_usd, 99_990_000);
        let ag_ultra_20x = templates
            .iter()
            .find(|t| t.template_id == "google_ai_ultra")
            .expect("ag_ultra_20x");
        assert_eq!(ag_ultra_20x.monthly_fee_micro_usd, 199_990_000);

        // Kimi Code templates must all include 30d monthly limit window
        for kimi_tpl in templates
            .iter()
            .filter(|t| t.provider_id == provider_ids::KIMI_CODE)
        {
            assert!(
                kimi_tpl.windows.iter().any(|w| w.window_id == "30d"),
                "Kimi template {} missing 30d window",
                kimi_tpl.template_id
            );
        }
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

        let w5 = evaluate_window(&pro.provider_id, &pro.windows[0], &samples, &[], now_ms);
        assert_eq!(w5.source, "local_estimation");
        assert!((w5.used_percent - 80.0).abs() < 0.01);
        assert_eq!(w5.alert_level, "warning");
        assert_eq!(w5.resets_in_seconds, 2 * 3600);
        let proj = w5.projected_exhaustion_hours.expect("projected");
        assert!((proj - 0.5).abs() < 0.01);

        let w7 = evaluate_window(&pro.provider_id, &pro.windows[1], &samples, &[], now_ms);
        assert_eq!(w7.used_value, 22_000_000);

        // SubscriptionPlanStatus must NOT expose local_estimation windows when no official quota exists.
        let plan_status = evaluate_subscription_plan(&pro, &samples, &[], now_ms);
        assert!(plan_status.windows.is_empty());
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
            source: Some("local_log".to_string()),
            plan_hint: Some("plus".to_string()),
        }];

        let status = evaluate_subscription_plan(&codex, &[], &obs, now_ms);
        assert_eq!(status.windows.len(), 1);
        let w5 = &status.windows[0];
        assert_eq!(w5.source, "official_log_snapshot");
        assert!((w5.used_percent - 42.5).abs() < 0.01);
        assert_eq!(w5.resets_in_seconds, 3600);
    }

    #[test]
    fn parses_ai_subscription_catalog_document_and_normalizes_ultra_name() {
        let base_plans: Vec<serde_json::Value> = subscription_plan_templates()
            .into_iter()
            .take(12)
            .map(|t| {
                let mut val = serde_json::to_value(&t).expect("json");
                val.as_object_mut()
                    .expect("obj")
                    .insert("status".to_string(), serde_json::json!("active"));
                val
            })
            .collect();
        let mut all_plans = base_plans;
        all_plans.push(serde_json::json!({
            "template_id": "cursor_pro_legacy",
            "provider_id": "cursor",
            "plan_name": "Cursor Pro (Legacy)",
            "monthly_fee_micro_usd": 20000000,
            "windows": [],
            "description": "Legacy",
            "status": "legacy"
        }));
        all_plans.push(serde_json::json!({
            "template_id": "google_ai_ultra_override",
            "provider_id": "antigravity",
            "plan_name": "Ultra Extra",
            "monthly_fee_micro_usd": 199990000,
            "windows": [],
            "description": "Test",
            "status": "active"
        }));
        // Replace google_ai_ultra with old v0.2.4 name to verify normalization
        for item in &mut all_plans {
            if item.get("template_id").and_then(|v| v.as_str()) == Some("google_ai_ultra") {
                item["plan_name"] = serde_json::json!("Google AI Ultra 20x");
                item["description"] = serde_json::json!("Google AI Ultra 20x flagship");
            }
        }
        let doc = serde_json::json!({
            "schema_version": "1.0.0",
            "plans": all_plans
        })
        .to_string();

        let parsed = parse_subscription_catalog_json(&doc).expect("parse catalog");
        assert!(!parsed.iter().any(|p| p.template_id == "cursor_pro_legacy"));
        let ultra = parsed
            .iter()
            .find(|p| p.template_id == "google_ai_ultra")
            .expect("google_ai_ultra");
        assert_eq!(ultra.plan_name, "Google AI Ultra");
        assert_eq!(ultra.description, "Google AI Ultra flagship");
    }

    #[test]
    fn antigravity_official_cli_pool_windows_attach_without_collapsing() {
        let ultra = plan_from_template("google_ai_ultra");
        let now_ms = 1_760_000_000_000i64;
        let obs = vec![
            ObservedRateLimitWindow {
                provider_id: provider_ids::ANTIGRAVITY.to_string(),
                window_name: "gemini-weekly".to_string(),
                used_percent: 63.86,
                window_minutes: Some(10_080),
                resets_at_ms: Some(now_ms + 86_400_000),
                observed_at_ms: now_ms - 5_000,
                source: Some("official_cli".to_string()),
                plan_hint: None,
            },
            ObservedRateLimitWindow {
                provider_id: provider_ids::ANTIGRAVITY.to_string(),
                window_name: "gemini-5h".to_string(),
                used_percent: 36.51,
                window_minutes: Some(300),
                resets_at_ms: Some(now_ms + 3_600_000),
                observed_at_ms: now_ms - 5_000,
                source: Some("official_cli".to_string()),
                plan_hint: None,
            },
            ObservedRateLimitWindow {
                provider_id: provider_ids::ANTIGRAVITY.to_string(),
                window_name: "3p-weekly".to_string(),
                used_percent: 0.45,
                window_minutes: Some(10_080),
                resets_at_ms: Some(now_ms + 90_000_000),
                observed_at_ms: now_ms - 5_000,
                source: Some("official_cli".to_string()),
                plan_hint: None,
            },
            ObservedRateLimitWindow {
                provider_id: provider_ids::ANTIGRAVITY.to_string(),
                window_name: "3p-5h".to_string(),
                used_percent: 0.0,
                window_minutes: Some(300),
                resets_at_ms: Some(now_ms + 7_200_000),
                observed_at_ms: now_ms - 5_000,
                source: Some("official_cli".to_string()),
                plan_hint: None,
            },
            // Expired official_cli bucket must be ignored
            ObservedRateLimitWindow {
                provider_id: provider_ids::ANTIGRAVITY.to_string(),
                window_name: "gemini-expired".to_string(),
                used_percent: 99.0,
                window_minutes: Some(300),
                resets_at_ms: Some(now_ms - 1_000),
                observed_at_ms: now_ms - 60_000,
                source: Some("official_cli".to_string()),
                plan_hint: None,
            },
        ];

        let status = evaluate_subscription_plan(&ultra, &[], &obs, now_ms);
        assert_eq!(status.windows.len(), 4);
        assert_eq!(status.windows[0].window_id, "gemini-5h");
        assert_eq!(status.windows[0].label, "Gemini 模型池 · 5h 限额");
        assert_eq!(status.windows[0].source, "official_cli");
        assert_eq!(status.windows[0].resets_in_seconds, 3600);
        assert_eq!(status.windows[1].window_id, "gemini-weekly");
        assert_eq!(status.windows[1].label, "Gemini 模型池 · 7d 周限额");
        assert_eq!(status.windows[2].window_id, "3p-5h");
        assert_eq!(status.windows[2].label, "Claude / GPT 模型池 · 5h 限额");
        assert_eq!(status.windows[3].window_id, "3p-weekly");
        assert_eq!(status.windows[3].label, "Claude / GPT 模型池 · 7d 周限额");

        let evaluated_templates = evaluate_subscription_templates_from_list(
            vec![SubscriptionPlanTemplate {
                template_id: "google_ai_ultra".to_string(),
                provider_id: provider_ids::ANTIGRAVITY.to_string(),
                plan_name: "Google AI Ultra".to_string(),
                monthly_fee_micro_usd: 199_990_000,
                windows: Vec::new(),
                description: "Catalog plan with empty rules".to_string(),
                live_windows: Vec::new(),
            }],
            &HashMap::new(),
            &obs,
            now_ms,
        );
        assert_eq!(evaluated_templates[0].live_windows.len(), 4);
    }
}
