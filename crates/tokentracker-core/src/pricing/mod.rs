use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::{Decimal, RoundingStrategy};

pub mod remote_sync;
pub use remote_sync::{
    compute_catalog_hash, format_exchange_rate, parse_litellm_catalog, parse_models_dev_catalog,
    parse_remote_exchange_rate_json,
};

use crate::domain::{provider_ids, CurrencyCode, ModelPricingEntry, NormalizedUsageEvent};
use crate::error::{CoreError, CoreResult};

const ONE_MILLION: i64 = 1_000_000;

/// Helper to build a built-in pricing entry using USD per 1M tokens.
#[allow(clippy::too_many_arguments)]
fn entry(
    model_pattern: &str,
    provider_id: &str,
    input_usd_per_m: f64,
    output_usd_per_m: f64,
    cache_read_usd_per_m: f64,
    cache_write_usd_per_m: f64,
    reasoning_usd_per_m: f64,
) -> ModelPricingEntry {
    let to_micro = |usd: f64| -> i64 { (usd * 1_000_000.0).round() as i64 };
    ModelPricingEntry {
        model_pattern: model_pattern.to_string(),
        provider_id: provider_id.to_string(),
        input_per_m_micro_usd: to_micro(input_usd_per_m),
        output_per_m_micro_usd: to_micro(output_usd_per_m),
        cache_read_per_m_micro_usd: to_micro(cache_read_usd_per_m),
        cache_write_per_m_micro_usd: to_micro(cache_write_usd_per_m),
        reasoning_per_m_micro_usd: to_micro(reasoning_usd_per_m),
        is_user_override: false,
        effective_from: "2026-01-01".to_string(),
    }
}

/// Built-in official public model pricing catalog (micro-USD per 1M tokens).
pub fn default_pricing_catalog() -> Vec<ModelPricingEntry> {
    vec![
        // Anthropic Claude models (Claude Code)
        entry("claude-opus-4-6", provider_ids::CLAUDE_CODE, 15.0, 75.0, 1.50, 18.75, 75.0),
        entry("claude-opus-4-5", provider_ids::CLAUDE_CODE, 15.0, 75.0, 1.50, 18.75, 75.0),
        entry("claude-opus-4", provider_ids::CLAUDE_CODE, 15.0, 75.0, 1.50, 18.75, 75.0),
        entry("claude-sonnet-4-6", provider_ids::CLAUDE_CODE, 3.0, 15.0, 0.30, 3.75, 15.0),
        entry("claude-sonnet-4-5", provider_ids::CLAUDE_CODE, 3.0, 15.0, 0.30, 3.75, 15.0),
        entry("claude-sonnet-4", provider_ids::CLAUDE_CODE, 3.0, 15.0, 0.30, 3.75, 15.0),
        entry("claude-3-7-sonnet", provider_ids::CLAUDE_CODE, 3.0, 15.0, 0.30, 3.75, 15.0),
        entry("claude-3-5-sonnet", provider_ids::CLAUDE_CODE, 3.0, 15.0, 0.30, 3.75, 15.0),
        entry("claude-haiku-4-5", provider_ids::CLAUDE_CODE, 1.0, 5.0, 0.10, 1.25, 5.0),
        entry("claude-3-5-haiku", provider_ids::CLAUDE_CODE, 0.80, 4.0, 0.08, 1.00, 4.0),
        entry("_default_claude_code", provider_ids::CLAUDE_CODE, 3.0, 15.0, 0.30, 3.75, 15.0),
        // OpenAI / Codex models
        entry("gpt-5.3-codex", provider_ids::CODEX, 2.0, 16.0, 0.50, 0.0, 16.0),
        entry("gpt-5.2-codex", provider_ids::CODEX, 1.75, 14.0, 0.175, 0.0, 14.0),
        entry("gpt-5.1-codex", provider_ids::CODEX, 1.25, 10.0, 0.125, 0.0, 10.0),
        entry("gpt-5-codex", provider_ids::CODEX, 1.25, 10.0, 0.125, 0.0, 10.0),
        entry("gpt-5", provider_ids::CODEX, 1.25, 10.0, 0.125, 0.0, 10.0),
        entry("gpt-5-mini", provider_ids::CODEX, 0.25, 2.0, 0.025, 0.0, 2.0),
        entry("codex-mini-latest", provider_ids::CODEX, 1.50, 6.0, 0.375, 0.0, 6.0),
        entry("o3", provider_ids::CODEX, 2.0, 8.0, 0.50, 0.0, 8.0),
        entry("o4-mini", provider_ids::CODEX, 1.10, 4.40, 0.275, 0.0, 4.40),
        entry("gpt-4.1", provider_ids::CODEX, 2.0, 8.0, 0.50, 0.0, 8.0),
        entry("gpt-4o", provider_ids::CODEX, 2.50, 10.0, 1.25, 0.0, 10.0),
        entry("_default_codex", provider_ids::CODEX, 1.25, 10.0, 0.125, 0.0, 10.0),
        // Google Gemini / Antigravity models
        entry("gemini-3.1-pro", provider_ids::ANTIGRAVITY, 2.0, 12.0, 0.20, 0.0, 12.0),
        entry("gemini-3-pro", provider_ids::ANTIGRAVITY, 2.0, 12.0, 0.20, 0.0, 12.0),
        entry("gemini-3-flash", provider_ids::ANTIGRAVITY, 0.50, 3.0, 0.05, 0.0, 3.0),
        entry("gemini-2.5-pro", provider_ids::ANTIGRAVITY, 1.25, 10.0, 0.125, 0.0, 10.0),
        entry("gemini-2.5-flash", provider_ids::ANTIGRAVITY, 0.30, 2.50, 0.03, 0.0, 2.50),
        entry("antigravity-planner", provider_ids::ANTIGRAVITY, 1.25, 10.0, 0.125, 0.0, 10.0),
        entry("antigravity-default", provider_ids::ANTIGRAVITY, 1.25, 10.0, 0.125, 0.0, 10.0),
        entry("_default_antigravity", provider_ids::ANTIGRAVITY, 1.25, 10.0, 0.125, 0.0, 10.0),
        // Public multi-vendor baseline models (used after remote sync or direct model queries)
        entry("deepseek-chat", "deepseek", 0.28, 0.42, 0.028, 0.0, 0.42),
        entry("deepseek-reasoner", "deepseek", 0.55, 2.19, 0.14, 0.0, 2.19),
        entry("_default_deepseek", "deepseek", 0.28, 0.42, 0.028, 0.0, 0.42),
        entry("kimi-k2", "kimi_code", 0.60, 2.50, 0.15, 0.0, 2.50),
        entry("_default_kimi_code", "kimi_code", 0.60, 2.50, 0.15, 0.0, 2.50),
        entry("minimax-m2", "minimax_code", 0.30, 1.20, 0.03, 0.0, 1.20),
        entry("_default_minimax_code", "minimax_code", 0.30, 1.20, 0.03, 0.0, 1.20),
    ]
}

/// Resolve the most specific pricing rule for `(provider_id, model_id)`.
/// Priority:
/// 1. User overrides (`is_user_override = true`) matching exact model > longest prefix/substring.
/// 2. Built-in rules matching exact model > longest prefix/substring.
/// 3. Provider-level `_default_<provider>` fallback.
pub fn resolve_pricing<'a>(
    catalog: &'a [ModelPricingEntry],
    provider_id: &str,
    model_id: &str,
) -> Option<&'a ModelPricingEntry> {
    let model_lower = model_id.trim().to_ascii_lowercase();
    let provider_lower = provider_id.trim().to_ascii_lowercase();

    let mut best: Option<((u8, u8, usize), &'a ModelPricingEntry)> = None;

    for item in catalog {
        let pat_lower = item.model_pattern.trim().to_ascii_lowercase();
        if pat_lower.starts_with("_default_") {
            continue;
        }
        let provider_match = item.provider_id.eq_ignore_ascii_case(&provider_lower)
            || item.provider_id.is_empty()
            || item.provider_id == "*";
        if !provider_match {
            continue;
        }

        let match_kind = if model_lower == pat_lower {
            Some(3u8)
        } else if model_lower.starts_with(&pat_lower) {
            Some(2u8)
        } else if !pat_lower.is_empty() && model_lower.contains(&pat_lower) {
            Some(1u8)
        } else {
            None
        };

        if let Some(kind) = match_kind {
            let override_rank = u8::from(item.is_user_override);
            let score = (override_rank, kind, pat_lower.len());
            if best.is_none_or(|(best_score, _)| score > best_score) {
                best = Some((score, item));
            }
        }
    }

    if let Some((_, found)) = best {
        return Some(found);
    }

    let default_pattern = format!("_default_{provider_lower}");
    catalog
        .iter()
        .find(|item| item.model_pattern.eq_ignore_ascii_case(&default_pattern))
        .or_else(|| {
            catalog
                .iter()
                .find(|item| item.provider_id.eq_ignore_ascii_case(&provider_lower))
        })
}

/// Fixed-point cost evaluation result (micro-USD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputedCost {
    pub estimated_cost_micro_usd: i64,
    pub cache_saved_micro_usd: i64,
}

/// Compute cost and prompt-cache savings using exact `rust_decimal::Decimal` arithmetic.
pub fn compute_event_cost(
    pricing: &ModelPricingEntry,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    reasoning_tokens: i64,
) -> CoreResult<ComputedCost> {
    let million = Decimal::from(ONE_MILLION);

    let term = |tokens: i64, rate_per_m: i64| -> Decimal {
        let t = Decimal::from(tokens.max(0));
        let r = Decimal::from(rate_per_m.max(0));
        (t * r) / million
    };

    let total_micro_dec = term(input_tokens, pricing.input_per_m_micro_usd)
        + term(output_tokens, pricing.output_per_m_micro_usd)
        + term(cache_read_tokens, pricing.cache_read_per_m_micro_usd)
        + term(cache_write_tokens, pricing.cache_write_per_m_micro_usd)
        + term(reasoning_tokens, pricing.reasoning_per_m_micro_usd);

    let rounded_cost = total_micro_dec
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
        .ok_or_else(|| CoreError::Validation("Cost overflowed i64 micro-USD".to_string()))?;

    let saved_rate_diff =
        (pricing.input_per_m_micro_usd - pricing.cache_read_per_m_micro_usd).max(0);
    let saved_dec = term(cache_read_tokens, saved_rate_diff);
    let rounded_saved = saved_dec
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
        .ok_or_else(|| CoreError::Validation("Cache savings overflowed i64 micro-USD".to_string()))?;

    Ok(ComputedCost {
        estimated_cost_micro_usd: rounded_cost,
        cache_saved_micro_usd: rounded_saved,
    })
}

/// Enrich a `NormalizedUsageEvent` with calculated cost and cache savings.
pub fn apply_pricing_to_event(
    event: &mut NormalizedUsageEvent,
    catalog: &[ModelPricingEntry],
) -> CoreResult<()> {
    if event.observable_only && event.total_tokens() == 0 {
        event.estimated_cost_micro_usd = 0;
        event.cache_saved_micro_usd = 0;
        return Ok(());
    }

    let Some(rule) = resolve_pricing(catalog, &event.provider_id, &event.model_id) else {
        event.estimated_cost_micro_usd = 0;
        event.cache_saved_micro_usd = 0;
        return Ok(());
    };

    let computed = compute_event_cost(
        rule,
        event.input_tokens,
        event.output_tokens,
        event.cache_read_tokens,
        event.cache_write_tokens,
        event.reasoning_tokens,
    )?;

    event.estimated_cost_micro_usd = computed.estimated_cost_micro_usd;
    event.cache_saved_micro_usd = computed.cache_saved_micro_usd;
    Ok(())
}

/// Parse a string decimal exchange rate safely.
pub fn parse_exchange_rate(rate_str: &str) -> CoreResult<Decimal> {
    let rate = rate_str
        .trim()
        .parse::<Decimal>()
        .map_err(CoreError::Decimal)?;
    if rate <= Decimal::ZERO {
        return Err(CoreError::Validation(
            "USD to CNY exchange rate must be positive".to_string(),
        ));
    }
    Ok(rate)
}

/// Convert integer micro-USD into display currency (`USD` or `CNY`) with fixed-point rounding.
pub fn convert_micro_usd_to_currency(
    micro_usd: i64,
    currency: CurrencyCode,
    usd_to_cny_rate: Decimal,
) -> CoreResult<Decimal> {
    let usd = Decimal::from(micro_usd)
        / Decimal::from_i64(ONE_MILLION)
            .ok_or_else(|| CoreError::Validation("Decimal conversion failed".to_string()))?;

    let converted = match currency {
        CurrencyCode::Usd => usd,
        CurrencyCode::Cny => usd * usd_to_cny_rate,
    };

    Ok(converted
        .round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero)
        .normalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn exact_fixed_point_cost_avoids_float_drift() {
        let catalog = default_pricing_catalog();
        let sonnet = resolve_pricing(&catalog, "claude_code", "claude-sonnet-4-5-20250929")
            .expect("sonnet pricing exists");

        // 1,000,000 input ($3.00) + 500,000 output ($7.50) + 2,000,000 cache_read ($0.60) + 100,000 cache_write ($0.375)
        // Total = $11.475 = 11,475,000 micro-USD
        // Cache saved = 2,000,000 * ($3.00 - $0.30) / 1M = $5.40 = 5,400,000 micro-USD
        let res = compute_event_cost(sonnet, 1_000_000, 500_000, 2_000_000, 100_000, 0)
            .expect("compute succeeds");
        assert_eq!(res.estimated_cost_micro_usd, 11_475_000);
        assert_eq!(res.cache_saved_micro_usd, 5_400_000);
    }

    #[test]
    fn user_override_beats_builtin_model_rule() {
        let mut catalog = default_pricing_catalog();
        catalog.push(ModelPricingEntry {
            model_pattern: "claude-sonnet-4-5".to_string(),
            provider_id: "claude_code".to_string(),
            input_per_m_micro_usd: 1_500_000,
            output_per_m_micro_usd: 7_500_000,
            cache_read_per_m_micro_usd: 150_000,
            cache_write_per_m_micro_usd: 1_875_000,
            reasoning_per_m_micro_usd: 7_500_000,
            is_user_override: true,
            effective_from: "2026-11-01".to_string(),
        });

        let resolved =
            resolve_pricing(&catalog, "claude_code", "claude-sonnet-4-5-20250929").expect("found");
        assert!(resolved.is_user_override);
        assert_eq!(resolved.input_per_m_micro_usd, 1_500_000);
    }

    #[test]
    fn currency_conversion_is_exact() {
        let rate = Decimal::from_str("7.25").expect("valid decimal");
        let usd = convert_micro_usd_to_currency(1_500_000, CurrencyCode::Usd, rate)
            .expect("usd conversion");
        assert_eq!(usd.to_string(), "1.5");

        let cny = convert_micro_usd_to_currency(1_500_000, CurrencyCode::Cny, rate)
            .expect("cny conversion");
        assert_eq!(cny.to_string(), "10.875");
    }
}
