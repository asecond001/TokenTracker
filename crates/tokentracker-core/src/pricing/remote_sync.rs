use std::collections::BTreeMap;

use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::domain::{provider_ids, ModelPricingEntry};
use crate::error::{CoreError, CoreResult};

const MICRO_PER_USD: i64 = 1_000_000;
const MICRO_PER_M_FROM_PER_TOKEN: i64 = 1_000_000_000_000;
const MAX_RATE_PER_M_MICRO_USD: i64 = 1_000_000_000; // $1,000 / 1M tokens sanity cap
const MIN_EXCHANGE_RATE: &str = "3.5";
const MAX_EXCHANGE_RATE: &str = "15.0";
const MIN_CATALOG_ENTRIES: usize = 5;

/// Parse an upstream exchange rate JSON payload (Frankfurter or Open Exchange Rates).
/// Requires `/rates/CNY` to be a finite decimal within `[3.5, 15.0]`.
pub fn parse_remote_exchange_rate_json(raw_json: &str) -> CoreResult<Decimal> {
    let root: Value = serde_json::from_str(raw_json)?;
    let cny_val = root
        .get("rates")
        .and_then(|rates| rates.get("CNY"))
        .ok_or_else(|| {
            CoreError::Validation("Exchange rate payload missing rates.CNY".to_string())
        })?;

    let rate = parse_json_decimal(cny_val).ok_or_else(|| {
        CoreError::Validation("Exchange rate rates.CNY is not a valid decimal".to_string())
    })?;

    let min_rate = MIN_EXCHANGE_RATE
        .parse::<Decimal>()
        .map_err(CoreError::Decimal)?;
    let max_rate = MAX_EXCHANGE_RATE
        .parse::<Decimal>()
        .map_err(CoreError::Decimal)?;

    if rate < min_rate || rate > max_rate {
        return Err(CoreError::Validation(format!(
            "Exchange rate {rate} outside valid sanity range [{min_rate}, {max_rate}]"
        )));
    }

    Ok(rate
        .round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero)
        .normalize())
}

/// Format a validated USD->CNY decimal exchange rate with up to 4 decimal places.
pub fn format_exchange_rate(rate: Decimal) -> String {
    let normalized = rate
        .round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero)
        .normalize();
    normalized.to_string()
}

/// Parse a `models.dev` catalog JSON payload (`/api.json`, costs expressed in USD per 1M tokens).
pub fn parse_models_dev_catalog(
    raw_json: &str,
    effective_date: &str,
) -> CoreResult<Vec<ModelPricingEntry>> {
    let root: Value = serde_json::from_str(raw_json)?;
    let root_obj = root.as_object().ok_or_else(|| {
        CoreError::Validation("models.dev payload must be a JSON object".to_string())
    })?;

    let mut candidates: BTreeMap<String, (u8, ModelPricingEntry)> = BTreeMap::new();

    for (vendor_key, vendor_val) in root_obj {
        let Some(vendor_obj) = vendor_val.as_object() else {
            continue;
        };
        let vendor_id = vendor_obj
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(vendor_key.as_str());
        let Some((mapped_provider, vendor_priority)) = map_upstream_vendor(vendor_id) else {
            continue;
        };
        let Some(models_obj) = vendor_obj.get("models").and_then(Value::as_object) else {
            continue;
        };

        for (model_key, model_val) in models_obj {
            let Some(model_obj) = model_val.as_object() else {
                continue;
            };
            if let Some(modalities) = model_obj.get("modalities").and_then(Value::as_object) {
                if let Some(outputs) = modalities.get("output").and_then(Value::as_array) {
                    let has_text = outputs
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|m| m.eq_ignore_ascii_case("text"));
                    if !outputs.is_empty() && !has_text {
                        continue;
                    }
                }
            }

            let raw_model_id = model_obj
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(model_key.as_str());
            let Some(model_pattern) = normalize_model_pattern(raw_model_id) else {
                continue;
            };
            if vendor_priority > 0 && !model_pattern.starts_with("gemini") {
                continue;
            }

            let Some(cost_obj) = model_obj.get("cost").and_then(Value::as_object) else {
                continue;
            };

            let Some(input) = parse_scaled_micro(cost_obj.get("input"), MICRO_PER_USD) else {
                continue;
            };
            let Some(output) = parse_scaled_micro(cost_obj.get("output"), MICRO_PER_USD) else {
                continue;
            };
            if input < 0 || output < 0 || (input == 0 && output == 0) {
                continue;
            }

            let cache_read = parse_scaled_micro(cost_obj.get("cache_read"), MICRO_PER_USD)
                .filter(|&v| v >= 0)
                .unwrap_or(0);
            let cache_write = parse_scaled_micro(cost_obj.get("cache_write"), MICRO_PER_USD)
                .filter(|&v| v >= 0)
                .unwrap_or(input);
            let reasoning = parse_scaled_micro(cost_obj.get("reasoning"), MICRO_PER_USD)
                .filter(|&v| v > 0)
                .unwrap_or(output);

            if !within_sanity_bounds(input, output, cache_read, cache_write, reasoning) {
                continue;
            }

            let entry = ModelPricingEntry {
                model_pattern: model_pattern.clone(),
                provider_id: mapped_provider.to_string(),
                input_per_m_micro_usd: input,
                output_per_m_micro_usd: output,
                cache_read_per_m_micro_usd: cache_read,
                cache_write_per_m_micro_usd: cache_write,
                reasoning_per_m_micro_usd: reasoning,
                is_user_override: false,
                effective_from: effective_date.to_string(),
            };
            insert_candidate(&mut candidates, model_pattern, vendor_priority, entry);
        }
    }

    finalize_candidates(candidates, "models.dev")
}

/// Parse a LiteLLM `model_prices_and_context_window.json` payload (costs expressed in USD per 1 token).
pub fn parse_litellm_catalog(
    raw_json: &str,
    effective_date: &str,
) -> CoreResult<Vec<ModelPricingEntry>> {
    let root: Value = serde_json::from_str(raw_json)?;
    let root_obj = root.as_object().ok_or_else(|| {
        CoreError::Validation("LiteLLM payload must be a JSON object".to_string())
    })?;

    let mut candidates: BTreeMap<String, (u8, ModelPricingEntry)> = BTreeMap::new();

    for (model_key, model_val) in root_obj {
        let Some(obj) = model_val.as_object() else {
            continue;
        };

        if let Some(mode) = obj.get("mode").and_then(Value::as_str) {
            let mode_lower = mode.trim().to_ascii_lowercase();
            if !matches!(
                mode_lower.as_str(),
                "chat" | "completion" | "responses" | ""
            ) {
                continue;
            }
        }

        let vendor_hint = obj
            .get("litellm_provider")
            .and_then(Value::as_str)
            .or_else(|| model_key.split('/').next());
        let Some((mapped_provider, vendor_priority)) =
            vendor_hint.and_then(map_upstream_vendor)
        else {
            continue;
        };

        let Some(model_pattern) = normalize_model_pattern(model_key) else {
            continue;
        };
        if vendor_priority > 0 && !model_pattern.starts_with("gemini") {
            continue;
        }

        let Some(input) =
            parse_scaled_micro(obj.get("input_cost_per_token"), MICRO_PER_M_FROM_PER_TOKEN)
        else {
            continue;
        };
        let Some(output) =
            parse_scaled_micro(obj.get("output_cost_per_token"), MICRO_PER_M_FROM_PER_TOKEN)
        else {
            continue;
        };
        if input < 0 || output < 0 || (input == 0 && output == 0) {
            continue;
        }

        let cache_read = parse_scaled_micro(
            obj.get("cache_read_input_token_cost"),
            MICRO_PER_M_FROM_PER_TOKEN,
        )
        .filter(|&v| v >= 0)
        .unwrap_or(0);
        let cache_write = parse_scaled_micro(
            obj.get("cache_creation_input_token_cost"),
            MICRO_PER_M_FROM_PER_TOKEN,
        )
        .filter(|&v| v >= 0)
        .unwrap_or(input);
        let reasoning = parse_scaled_micro(
            obj.get("output_cost_per_reasoning_token"),
            MICRO_PER_M_FROM_PER_TOKEN,
        )
        .filter(|&v| v > 0)
        .unwrap_or(output);

        if !within_sanity_bounds(input, output, cache_read, cache_write, reasoning) {
            continue;
        }

        let entry = ModelPricingEntry {
            model_pattern: model_pattern.clone(),
            provider_id: mapped_provider.to_string(),
            input_per_m_micro_usd: input,
            output_per_m_micro_usd: output,
            cache_read_per_m_micro_usd: cache_read,
            cache_write_per_m_micro_usd: cache_write,
            reasoning_per_m_micro_usd: reasoning,
            is_user_override: false,
            effective_from: effective_date.to_string(),
        };
        insert_candidate(&mut candidates, model_pattern, vendor_priority, entry);
    }

    finalize_candidates(candidates, "litellm")
}

/// Compute a deterministic SHA-256 hash across pricing rates so unchanged catalogs avoid full table writes.
pub fn compute_catalog_hash(entries: &[ModelPricingEntry]) -> String {
    let mut rows: Vec<(String, String, i64, i64, i64, i64, i64)> = entries
        .iter()
        .map(|e| {
            (
                e.model_pattern.clone(),
                e.provider_id.clone(),
                e.input_per_m_micro_usd,
                e.output_per_m_micro_usd,
                e.cache_read_per_m_micro_usd,
                e.cache_write_per_m_micro_usd,
                e.reasoning_per_m_micro_usd,
            )
        })
        .collect();
    rows.sort();

    let mut hasher = Sha256::new();
    for (pat, prov, input, output, cache_read, cache_write, reasoning) in rows {
        hasher.update(pat.as_bytes());
        hasher.update(b"|");
        hasher.update(prov.as_bytes());
        hasher.update(b"|");
        hasher.update(input.to_le_bytes());
        hasher.update(output.to_le_bytes());
        hasher.update(cache_read.to_le_bytes());
        hasher.update(cache_write.to_le_bytes());
        hasher.update(reasoning.to_le_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

fn map_upstream_vendor(vendor: &str) -> Option<(&'static str, u8)> {
    match vendor.trim().to_ascii_lowercase().as_str() {
        "anthropic" => Some((provider_ids::CLAUDE_CODE, 0)),
        "openai" => Some((provider_ids::CODEX, 0)),
        "google" | "gemini" => Some((provider_ids::ANTIGRAVITY, 0)),
        "google-vertex"
        | "google_vertex"
        | "vertex_ai"
        | "vertex_ai-language-models" => Some((provider_ids::ANTIGRAVITY, 1)),
        "deepseek" => Some(("deepseek", 0)),
        "moonshot" | "moonshotai" | "kimi" => Some(("kimi_code", 0)),
        "minimax" => Some(("minimax_code", 0)),
        _ => None,
    }
}

fn normalize_model_pattern(raw: &str) -> Option<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    let stripped = trimmed.rsplit('/').next().unwrap_or(trimmed.as_str()).trim();
    if stripped.is_empty()
        || stripped == "sample_spec"
        || stripped.starts_with("_default_")
        || stripped.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some(stripped.to_string())
}

fn parse_json_decimal(val: &Value) -> Option<Decimal> {
    match val {
        Value::Number(num) => {
            let s = num.to_string();
            if s.contains('e') || s.contains('E') {
                Decimal::from_scientific(&s)
                    .ok()
                    .or_else(|| num.as_f64().and_then(Decimal::from_f64))
            } else {
                s.parse::<Decimal>()
                    .ok()
                    .or_else(|| num.as_f64().and_then(Decimal::from_f64))
            }
        }
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.contains('e') || trimmed.contains('E') {
                Decimal::from_scientific(trimmed).ok()
            } else {
                trimmed.parse::<Decimal>().ok()
            }
        }
        _ => None,
    }
}

fn parse_scaled_micro(val: Option<&Value>, multiplier: i64) -> Option<i64> {
    let dec = parse_json_decimal(val?)?;
    let scaled = dec.checked_mul(Decimal::from(multiplier))?;
    scaled
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
}

fn within_sanity_bounds(
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
) -> bool {
    [input, output, cache_read, cache_write, reasoning]
        .into_iter()
        .all(|v| (0..=MAX_RATE_PER_M_MICRO_USD).contains(&v))
}

fn insert_candidate(
    map: &mut BTreeMap<String, (u8, ModelPricingEntry)>,
    model_pattern: String,
    priority: u8,
    entry: ModelPricingEntry,
) {
    match map.get_mut(&model_pattern) {
        Some((existing_priority, existing_entry)) => {
            if priority < *existing_priority {
                *existing_priority = priority;
                *existing_entry = entry;
            }
        }
        None => {
            map.insert(model_pattern, (priority, entry));
        }
    }
}

fn finalize_candidates(
    candidates: BTreeMap<String, (u8, ModelPricingEntry)>,
    source_name: &str,
) -> CoreResult<Vec<ModelPricingEntry>> {
    let mut entries: Vec<ModelPricingEntry> = candidates.into_values().map(|(_, e)| e).collect();
    if entries.len() < MIN_CATALOG_ENTRIES {
        return Err(CoreError::Validation(format!(
            "Upstream catalog '{source_name}' yielded only {} valid models (minimum {MIN_CATALOG_ENTRIES})",
            entries.len()
        )));
    }
    entries.sort_by(|a, b| {
        a.provider_id
            .cmp(&b.provider_id)
            .then_with(|| a.model_pattern.cmp(&b.model_pattern))
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frankfurter_and_open_er_exchange_rates_with_sanity_bounds() {
        let frankfurter = r#"{"amount":1.0,"base":"USD","date":"2026-11-16","rates":{"CNY":7.1852}}"#;
        let parsed = parse_remote_exchange_rate_json(frankfurter).expect("frankfurter parses");
        assert_eq!(format_exchange_rate(parsed), "7.1852");

        let open_er = r#"{"result":"success","base_code":"USD","rates":{"CNY":7.20}}"#;
        let parsed_open = parse_remote_exchange_rate_json(open_er).expect("open_er parses");
        assert_eq!(format_exchange_rate(parsed_open), "7.2");

        let too_low = r#"{"rates":{"CNY":1.5}}"#;
        assert!(parse_remote_exchange_rate_json(too_low).is_err());

        let too_high = r#"{"rates":{"CNY":25.0}}"#;
        assert!(parse_remote_exchange_rate_json(too_high).is_err());

        let missing = r#"{"rates":{"EUR":0.92}}"#;
        assert!(parse_remote_exchange_rate_json(missing).is_err());
    }

    #[test]
    fn parses_models_dev_catalog_in_usd_per_million_and_filters_outliers() {
        let payload = r#"{
            "anthropic": {
                "id": "anthropic",
                "models": {
                    "claude-sonnet-4-5": {
                        "id": "claude-sonnet-4-5",
                        "cost": { "input": 3.0, "output": 15.0, "cache_read": 0.3, "cache_write": 3.75 }
                    },
                    "claude-opus-4-6": {
                        "id": "claude-opus-4-6",
                        "cost": { "input": 15.0, "output": 75.0, "cache_read": 1.5, "cache_write": 18.75, "reasoning": 75.0 }
                    },
                    "broken-expensive": {
                        "id": "broken-expensive",
                        "cost": { "input": 1500.0, "output": 5000.0 }
                    }
                }
            },
            "openai": {
                "id": "openai",
                "models": {
                    "gpt-5-codex": {
                        "id": "gpt-5-codex",
                        "cost": { "input": 1.25, "output": 10.0, "cache_read": 0.125 }
                    },
                    "gpt-5.2-codex": {
                        "id": "gpt-5.2-codex",
                        "cost": { "input": 1.75, "output": 14.0, "cache_read": 0.175 }
                    }
                }
            },
            "google": {
                "id": "google",
                "models": {
                    "gemini-3-flash": {
                        "id": "gemini-3-flash",
                        "cost": { "input": 0.5, "output": 3.0, "cache_read": 0.05 }
                    }
                }
            },
            "openrouter": {
                "id": "openrouter",
                "models": {
                    "anthropic/claude-sonnet-4-5": {
                        "id": "anthropic/claude-sonnet-4-5",
                        "cost": { "input": 99.0, "output": 99.0 }
                    }
                }
            }
        }"#;

        let entries = parse_models_dev_catalog(payload, "2026-11-16").expect("valid models.dev");
        assert_eq!(entries.len(), 5);
        assert!(entries.iter().all(|e| e.model_pattern != "broken-expensive"));

        let sonnet = entries
            .iter()
            .find(|e| e.model_pattern == "claude-sonnet-4-5")
            .expect("sonnet");
        assert_eq!(sonnet.provider_id, "claude_code");
        assert_eq!(sonnet.input_per_m_micro_usd, 3_000_000);
        assert_eq!(sonnet.output_per_m_micro_usd, 15_000_000);
        assert_eq!(sonnet.cache_read_per_m_micro_usd, 300_000);
        assert_eq!(sonnet.cache_write_per_m_micro_usd, 3_750_000);
        // Reasoning falls back to output price when omitted
        assert_eq!(sonnet.reasoning_per_m_micro_usd, 15_000_000);
    }

    #[test]
    fn parses_litellm_catalog_with_per_token_to_micro_usd_per_m_conversion() {
        let payload = r#"{
            "sample_spec": { "input_cost_per_token": 0.001, "output_cost_per_token": 0.002 },
            "claude-sonnet-4-5": {
                "litellm_provider": "anthropic",
                "mode": "chat",
                "input_cost_per_token": 0.000003,
                "output_cost_per_token": 0.000015,
                "cache_read_input_token_cost": 0.0000003,
                "cache_creation_input_token_cost": 0.00000375
            },
            "claude-haiku-4-5": {
                "litellm_provider": "anthropic",
                "mode": "chat",
                "input_cost_per_token": 0.000001,
                "output_cost_per_token": 0.000005,
                "cache_read_input_token_cost": 0.0000001
            },
            "openai/gpt-5-codex": {
                "litellm_provider": "openai",
                "mode": "responses",
                "input_cost_per_token": 1.25e-6,
                "output_cost_per_token": 1e-5,
                "cache_read_input_token_cost": 1.25e-7
            },
            "gemini/gemini-2.5-pro": {
                "litellm_provider": "gemini",
                "mode": "chat",
                "input_cost_per_token": 0.00000125,
                "output_cost_per_token": 0.00001
            },
            "deepseek/deepseek-chat": {
                "litellm_provider": "deepseek",
                "mode": "chat",
                "input_cost_per_token": 0.00000028,
                "output_cost_per_token": 0.00000042
            },
            "text-embedding-3-large": {
                "litellm_provider": "openai",
                "mode": "embedding",
                "input_cost_per_token": 0.00000013,
                "output_cost_per_token": 0.0
            }
        }"#;

        let entries = parse_litellm_catalog(payload, "2026-11-16").expect("valid litellm");
        assert_eq!(entries.len(), 5);
        assert!(entries.iter().all(|e| e.model_pattern != "text-embedding-3-large"));

        let sonnet = entries
            .iter()
            .find(|e| e.model_pattern == "claude-sonnet-4-5")
            .expect("sonnet");
        assert_eq!(sonnet.input_per_m_micro_usd, 3_000_000);
        assert_eq!(sonnet.output_per_m_micro_usd, 15_000_000);
        assert_eq!(sonnet.cache_read_per_m_micro_usd, 300_000);
        assert_eq!(sonnet.cache_write_per_m_micro_usd, 3_750_000);

        let codex = entries
            .iter()
            .find(|e| e.model_pattern == "gpt-5-codex")
            .expect("codex");
        assert_eq!(codex.provider_id, "codex");
        assert_eq!(codex.input_per_m_micro_usd, 1_250_000);
        assert_eq!(codex.output_per_m_micro_usd, 10_000_000);
        assert_eq!(codex.cache_read_per_m_micro_usd, 125_000);
    }

    #[test]
    fn catalog_hash_is_order_independent_and_sensitive_to_price_changes() {
        let e1 = ModelPricingEntry {
            model_pattern: "a".to_string(),
            provider_id: "codex".to_string(),
            input_per_m_micro_usd: 100,
            output_per_m_micro_usd: 200,
            cache_read_per_m_micro_usd: 10,
            cache_write_per_m_micro_usd: 0,
            reasoning_per_m_micro_usd: 200,
            is_user_override: false,
            effective_from: "2026-01-01".to_string(),
        };
        let mut e2 = e1.clone();
        e2.model_pattern = "b".to_string();

        assert_eq!(
            compute_catalog_hash(&[e1.clone(), e2.clone()]),
            compute_catalog_hash(&[e2.clone(), e1.clone()])
        );

        let mut changed = e2.clone();
        changed.output_per_m_micro_usd = 250;
        assert_ne!(
            compute_catalog_hash(&[e1, e2]),
            compute_catalog_hash(&[changed])
        );
    }
}
