use serde_json::Value;

use crate::domain::{
    compute_event_id, compute_tps, parse_timestamp_ms_json, CustomProtocolKind,
    CustomProviderRule, CustomRuleTestResult, ModelPricingEntry, NormalizedUsageEvent,
};
use crate::pricing::apply_pricing_to_event;

fn normalize_pointer(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('/') {
        Some(trimmed.to_string())
    } else {
        Some(format!("/{trimmed}"))
    }
}

fn pointer_value<'a>(root: &'a Value, path: Option<&str>) -> Option<&'a Value> {
    let normalized = normalize_pointer(path?)?;
    root.pointer(&normalized)
}

fn value_as_str(v: Option<&Value>) -> Option<String> {
    let val = v?;
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    } else if let Some(n) = val.as_i64() {
        return Some(n.to_string());
    }
    None
}

fn value_as_i64(v: Option<&Value>) -> Option<i64> {
    let val = v?;
    if let Some(i) = val.as_i64() {
        return Some(i.max(0));
    }
    if let Some(u) = val.as_u64() {
        return Some((u.min(i64::MAX as u64)) as i64);
    }
    if let Some(f) = val.as_f64() {
        if f.is_finite() && f >= 0.0 {
            return Some(f.round() as i64);
        }
    }
    if let Some(s) = val.as_str() {
        if let Ok(parsed) = s.trim().parse::<i64>() {
            return Some(parsed.max(0));
        }
    }
    None
}

pub fn resolve_custom_provider_id(rule: &CustomProviderRule) -> String {
    if let Some(override_id) = rule
        .mapping_spec
        .provider_id_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return override_id.to_string();
    }
    let clean = rule.rule_id.trim();
    if clean.starts_with("custom_") {
        clean.to_string()
    } else if clean.is_empty() {
        "custom_compatible".to_string()
    } else {
        format!("custom_{clean}")
    }
}

struct ExtractLineCtx<'a> {
    provider_id: &'a str,
    device_id: &'a str,
    session_fallback: &'a str,
    project_label: &'a str,
    fallback_ts_ms: i64,
    line_number: u64,
}

fn extract_openai_style(
    root: &Value,
    rule: &CustomProviderRule,
    ctx: &ExtractLineCtx<'_>,
) -> Result<NormalizedUsageEvent, String> {
    let response_obj = root
        .get("response")
        .or_else(|| root.get("body"))
        .unwrap_or(root);
    let usage = response_obj
        .get("usage")
        .or_else(|| root.get("usage"))
        .ok_or_else(|| "Missing 'usage' object in OpenAI JSONL record".to_string())?;

    let model_id = value_as_str(pointer_value(
        root,
        rule.mapping_spec.model_path.as_deref(),
    ))
    .or_else(|| value_as_str(response_obj.get("model")))
    .or_else(|| value_as_str(root.get("model")))
    .ok_or_else(|| "Missing 'model' field in OpenAI JSONL record".to_string())?;

    let timestamp_ms = parse_timestamp_ms_json(pointer_value(
        root,
        rule.mapping_spec.timestamp_path.as_deref(),
    ))
    .or_else(|| parse_timestamp_ms_json(root.get("timestamp")))
    .or_else(|| parse_timestamp_ms_json(root.get("created_at")))
    .or_else(|| parse_timestamp_ms_json(root.get("created")))
    .or_else(|| parse_timestamp_ms_json(response_obj.get("created")))
    .or_else(|| parse_timestamp_ms_json(root.get("ts")))
    .unwrap_or(ctx.fallback_ts_ms);

    let raw_input = value_as_i64(usage.get("prompt_tokens"))
        .or_else(|| value_as_i64(usage.get("input_tokens")))
        .unwrap_or(0);
    let raw_output = value_as_i64(usage.get("completion_tokens"))
        .or_else(|| value_as_i64(usage.get("output_tokens")))
        .unwrap_or(0);
    let cached = value_as_i64(usage.pointer("/prompt_tokens_details/cached_tokens"))
        .or_else(|| value_as_i64(usage.pointer("/input_tokens_details/cached_tokens")))
        .or_else(|| value_as_i64(usage.get("cached_tokens")))
        .or_else(|| value_as_i64(usage.get("prompt_cache_hit_tokens")))
        .unwrap_or(0);
    let reasoning = value_as_i64(usage.pointer("/completion_tokens_details/reasoning_tokens"))
        .or_else(|| value_as_i64(usage.pointer("/output_tokens_details/reasoning_tokens")))
        .or_else(|| value_as_i64(usage.get("reasoning_tokens")))
        .unwrap_or(0);

    let input_tokens = if raw_input >= cached {
        raw_input - cached
    } else {
        raw_input
    };
    let output_tokens = if raw_output >= reasoning {
        raw_output - reasoning
    } else {
        raw_output
    };

    if input_tokens + output_tokens + cached + reasoning <= 0 {
        return Err("Token counters in OpenAI 'usage' are all zero".to_string());
    }

    let session_id = value_as_str(pointer_value(
        root,
        rule.mapping_spec.session_id_path.as_deref(),
    ))
    .or_else(|| value_as_str(root.get("session_id")))
    .or_else(|| value_as_str(root.get("sessionId")))
    .unwrap_or_else(|| ctx.session_fallback.to_string());

    let request_id = value_as_str(pointer_value(
        root,
        rule.mapping_spec.request_id_path.as_deref(),
    ))
    .or_else(|| value_as_str(response_obj.get("id")))
    .or_else(|| value_as_str(root.get("id")))
    .unwrap_or_else(|| format!("line-{}", ctx.line_number));

    let duration_ms = value_as_i64(pointer_value(
        root,
        rule.mapping_spec.duration_ms_path.as_deref(),
    ))
    .or_else(|| value_as_i64(root.get("duration_ms")))
    .or_else(|| value_as_i64(root.get("latency_ms")));
    let ttft_ms = value_as_i64(pointer_value(
        root,
        rule.mapping_spec.ttft_ms_path.as_deref(),
    ))
    .or_else(|| value_as_i64(root.get("ttft_ms")));

    let output_tps = compute_tps(output_tokens + reasoning, duration_ms);

    Ok(NormalizedUsageEvent {
        event_id: compute_event_id(ctx.provider_id, &session_id, &request_id, timestamp_ms),
        device_id: ctx.device_id.to_string(),
        provider_id: ctx.provider_id.to_string(),
        session_id,
        project_label: ctx.project_label.to_string(),
        model_id,
        timestamp_ms,
        input_tokens,
        output_tokens,
        cache_read_tokens: cached,
        cache_write_tokens: 0,
        reasoning_tokens: reasoning,
        observable_only: false,
        duration_ms,
        ttft_ms,
        output_tps,
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    })
}

fn extract_anthropic_style(
    root: &Value,
    rule: &CustomProviderRule,
    ctx: &ExtractLineCtx<'_>,
) -> Result<NormalizedUsageEvent, String> {
    let msg_obj = root
        .get("message")
        .or_else(|| root.get("response"))
        .unwrap_or(root);
    let usage = msg_obj
        .get("usage")
        .or_else(|| root.get("usage"))
        .ok_or_else(|| "Missing 'usage' object in Anthropic JSONL record".to_string())?;

    let model_id = value_as_str(pointer_value(
        root,
        rule.mapping_spec.model_path.as_deref(),
    ))
    .or_else(|| value_as_str(msg_obj.get("model")))
    .or_else(|| value_as_str(root.get("model")))
    .ok_or_else(|| "Missing 'model' field in Anthropic JSONL record".to_string())?;

    let timestamp_ms = parse_timestamp_ms_json(pointer_value(
        root,
        rule.mapping_spec.timestamp_path.as_deref(),
    ))
    .or_else(|| parse_timestamp_ms_json(root.get("timestamp")))
    .or_else(|| parse_timestamp_ms_json(root.get("created_at")))
    .or_else(|| parse_timestamp_ms_json(root.get("ts")))
    .unwrap_or(ctx.fallback_ts_ms);

    let input_tokens = value_as_i64(usage.get("input_tokens")).unwrap_or(0);
    let output_tokens = value_as_i64(usage.get("output_tokens")).unwrap_or(0);
    let cache_read_tokens = value_as_i64(usage.get("cache_read_input_tokens"))
        .or_else(|| value_as_i64(usage.get("cache_read_tokens")))
        .unwrap_or(0);
    let cache_write_tokens = value_as_i64(usage.get("cache_creation_input_tokens"))
        .or_else(|| value_as_i64(usage.get("cache_write_tokens")))
        .unwrap_or(0);

    if input_tokens + output_tokens + cache_read_tokens + cache_write_tokens <= 0 {
        return Err("Token counters in Anthropic 'usage' are all zero".to_string());
    }

    let session_id = value_as_str(pointer_value(
        root,
        rule.mapping_spec.session_id_path.as_deref(),
    ))
    .or_else(|| value_as_str(root.get("sessionId")))
    .or_else(|| value_as_str(root.get("session_id")))
    .unwrap_or_else(|| ctx.session_fallback.to_string());

    let request_id = value_as_str(pointer_value(
        root,
        rule.mapping_spec.request_id_path.as_deref(),
    ))
    .or_else(|| value_as_str(msg_obj.get("id")))
    .or_else(|| value_as_str(root.get("uuid")))
    .or_else(|| value_as_str(root.get("id")))
    .unwrap_or_else(|| format!("line-{}", ctx.line_number));

    let duration_ms = value_as_i64(pointer_value(
        root,
        rule.mapping_spec.duration_ms_path.as_deref(),
    ))
    .or_else(|| value_as_i64(root.get("duration_ms")));
    let ttft_ms = value_as_i64(pointer_value(
        root,
        rule.mapping_spec.ttft_ms_path.as_deref(),
    ))
    .or_else(|| value_as_i64(root.get("ttft_ms")));

    let output_tps = compute_tps(output_tokens, duration_ms);

    Ok(NormalizedUsageEvent {
        event_id: compute_event_id(ctx.provider_id, &session_id, &request_id, timestamp_ms),
        device_id: ctx.device_id.to_string(),
        provider_id: ctx.provider_id.to_string(),
        session_id,
        project_label: ctx.project_label.to_string(),
        model_id,
        timestamp_ms,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms,
        ttft_ms,
        output_tps,
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    })
}

fn extract_declarative_style(
    root: &Value,
    rule: &CustomProviderRule,
    ctx: &ExtractLineCtx<'_>,
) -> Result<NormalizedUsageEvent, String> {
    let spec = &rule.mapping_spec;
    let model_id = value_as_str(pointer_value(root, spec.model_path.as_deref()))
        .ok_or_else(|| "Declarative model_path did not match a non-empty string".to_string())?;

    let timestamp_ms = parse_timestamp_ms_json(pointer_value(root, spec.timestamp_path.as_deref()))
        .unwrap_or(ctx.fallback_ts_ms);

    let input_tokens =
        value_as_i64(pointer_value(root, spec.input_tokens_path.as_deref())).unwrap_or(0);
    let output_tokens =
        value_as_i64(pointer_value(root, spec.output_tokens_path.as_deref())).unwrap_or(0);
    let cache_read_tokens =
        value_as_i64(pointer_value(root, spec.cache_read_tokens_path.as_deref())).unwrap_or(0);
    let cache_write_tokens =
        value_as_i64(pointer_value(root, spec.cache_write_tokens_path.as_deref())).unwrap_or(0);
    let reasoning_tokens =
        value_as_i64(pointer_value(root, spec.reasoning_tokens_path.as_deref())).unwrap_or(0);

    if input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens
        <= 0
    {
        return Err(
            "All mapped declarative token counters resolved to 0 or missing".to_string(),
        );
    }

    let session_id = value_as_str(pointer_value(root, spec.session_id_path.as_deref()))
        .unwrap_or_else(|| ctx.session_fallback.to_string());
    let request_id = value_as_str(pointer_value(root, spec.request_id_path.as_deref()))
        .unwrap_or_else(|| format!("line-{}", ctx.line_number));
    let duration_ms = value_as_i64(pointer_value(root, spec.duration_ms_path.as_deref()));
    let ttft_ms = value_as_i64(pointer_value(root, spec.ttft_ms_path.as_deref()));
    let output_tps = compute_tps(output_tokens + reasoning_tokens, duration_ms);

    Ok(NormalizedUsageEvent {
        event_id: compute_event_id(ctx.provider_id, &session_id, &request_id, timestamp_ms),
        device_id: ctx.device_id.to_string(),
        provider_id: ctx.provider_id.to_string(),
        session_id,
        project_label: ctx.project_label.to_string(),
        model_id,
        timestamp_ms,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        reasoning_tokens,
        observable_only: false,
        duration_ms,
        ttft_ms,
        output_tps,
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    })
}

fn extract_with_reason(
    rule: &CustomProviderRule,
    raw_line: &str,
    device_id: &str,
    session_fallback: &str,
    project_label: &str,
    fallback_ts_ms: i64,
    line_number: u64,
) -> Result<NormalizedUsageEvent, String> {
    let trimmed = raw_line.trim();
    if trimmed.is_empty() {
        return Err("Empty line".to_string());
    }
    let root: Value =
        serde_json::from_str(trimmed).map_err(|e| format!("Invalid JSON object: {e}"))?;
    let provider_id = resolve_custom_provider_id(rule);
    let ctx = ExtractLineCtx {
        provider_id: &provider_id,
        device_id,
        session_fallback,
        project_label,
        fallback_ts_ms,
        line_number,
    };

    match rule.protocol_kind {
        CustomProtocolKind::OpenaiJsonl => extract_openai_style(&root, rule, &ctx),
        CustomProtocolKind::AnthropicJsonl => extract_anthropic_style(&root, rule, &ctx),
        CustomProtocolKind::DeclarativeJsonl => extract_declarative_style(&root, rule, &ctx),
    }
}

/// Extract one normalized usage event from a single raw JSONL line using a custom provider rule.
pub fn extract_custom_rule_line(
    rule: &CustomProviderRule,
    raw_line: &str,
    device_id: &str,
    session_fallback: &str,
    project_label: &str,
    fallback_ts_ms: i64,
    line_number: u64,
) -> Option<NormalizedUsageEvent> {
    extract_with_reason(
        rule,
        raw_line,
        device_id,
        session_fallback,
        project_label,
        fallback_ts_ms,
        line_number,
    )
    .ok()
}

/// Perform a dry-run extraction + pricing match for a single sanitized sample JSONL line.
pub fn test_custom_rule_line(
    rule: &CustomProviderRule,
    sample_line: &str,
    catalog: &[ModelPricingEntry],
) -> CustomRuleTestResult {
    let fallback_provider = resolve_custom_provider_id(rule);
    match extract_with_reason(
        rule,
        sample_line,
        "preview-device",
        "preview-session",
        "workspace",
        0,
        1,
    ) {
        Ok(mut event) => {
            let _ = apply_pricing_to_event(&mut event, catalog);
            CustomRuleTestResult {
                matched: true,
                provider_id: event.provider_id,
                model_id: Some(event.model_id),
                timestamp_ms: Some(event.timestamp_ms),
                input_tokens: event.input_tokens,
                output_tokens: event.output_tokens,
                cache_read_tokens: event.cache_read_tokens,
                cache_write_tokens: event.cache_write_tokens,
                reasoning_tokens: event.reasoning_tokens,
                estimated_cost_micro_usd: event.estimated_cost_micro_usd,
                diagnostic: "OK: extracted usage event & resolved pricing".to_string(),
            }
        }
        Err(reason) => CustomRuleTestResult {
            matched: false,
            provider_id: fallback_provider,
            model_id: None,
            timestamp_ms: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            estimated_cost_micro_usd: 0,
            diagnostic: reason,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DeclarativeMappingSpec;
    use crate::pricing::default_pricing_catalog;

    #[test]
    fn openai_anthropic_and_declarative_rules_extract_tokens_and_cost() {
        let catalog = default_pricing_catalog();

        let openai_rule = CustomProviderRule {
            rule_id: "local_vllm".to_string(),
            display_name: "Local vLLM".to_string(),
            enabled: true,
            protocol_kind: CustomProtocolKind::OpenaiJsonl,
            log_roots: vec![],
            mapping_spec: DeclarativeMappingSpec::default(),
            updated_at_ms: 0,
        };
        let openai_line = r#"{"id":"req-1","created_at":"2026-11-16T10:00:00Z","model":"deepseek-chat","usage":{"prompt_tokens":10000,"completion_tokens":2500,"prompt_tokens_details":{"cached_tokens":4000},"completion_tokens_details":{"reasoning_tokens":500}},"duration_ms":2000}"#;
        let res_openai = test_custom_rule_line(&openai_rule, openai_line, &catalog);
        assert!(res_openai.matched);
        assert_eq!(res_openai.provider_id, "custom_local_vllm");
        assert_eq!(res_openai.model_id.as_deref(), Some("deepseek-chat"));
        assert_eq!(res_openai.input_tokens, 6_000);
        assert_eq!(res_openai.cache_read_tokens, 4_000);
        assert_eq!(res_openai.output_tokens, 2_000);
        assert_eq!(res_openai.reasoning_tokens, 500);
        assert!(res_openai.estimated_cost_micro_usd > 0);

        let anthropic_rule = CustomProviderRule {
            rule_id: "bedrock_proxy".to_string(),
            display_name: "Bedrock Proxy".to_string(),
            enabled: true,
            protocol_kind: CustomProtocolKind::AnthropicJsonl,
            log_roots: vec![],
            mapping_spec: DeclarativeMappingSpec {
                provider_id_override: Some("custom_bedrock".to_string()),
                ..Default::default()
            },
            updated_at_ms: 0,
        };
        let anthropic_line = r#"{"timestamp":"2026-11-16T10:05:00Z","message":{"id":"msg-9","model":"claude-sonnet-4-5","usage":{"input_tokens":5000,"output_tokens":1200,"cache_read_input_tokens":20000,"cache_creation_input_tokens":1000}}}"#;
        let res_ant = test_custom_rule_line(&anthropic_rule, anthropic_line, &catalog);
        assert!(res_ant.matched);
        assert_eq!(res_ant.provider_id, "custom_bedrock");
        assert_eq!(res_ant.input_tokens, 5_000);
        assert_eq!(res_ant.cache_read_tokens, 20_000);
        assert_eq!(res_ant.cache_write_tokens, 1_000);
        assert!(res_ant.estimated_cost_micro_usd > 0);

        let decl_rule = CustomProviderRule {
            rule_id: "acme_agent".to_string(),
            display_name: "Acme Agent".to_string(),
            enabled: true,
            protocol_kind: CustomProtocolKind::DeclarativeJsonl,
            log_roots: vec![],
            mapping_spec: DeclarativeMappingSpec {
                timestamp_path: Some("/ts".to_string()),
                model_path: Some("meta/model_name".to_string()),
                session_id_path: Some("/meta/session".to_string()),
                request_id_path: Some("/meta/turn_id".to_string()),
                input_tokens_path: Some("/stats/in".to_string()),
                output_tokens_path: Some("/stats/out".to_string()),
                cache_read_tokens_path: Some("/stats/cached".to_string()),
                ..Default::default()
            },
            updated_at_ms: 0,
        };
        let decl_line = r#"{"ts":1763287500000,"meta":{"model_name":"kimi-k2","session":"sess-a","turn_id":"t-1"},"stats":{"in":8000,"out":1000,"cached":12000}}"#;
        let res_decl = test_custom_rule_line(&decl_rule, decl_line, &catalog);
        assert!(res_decl.matched);
        assert_eq!(res_decl.input_tokens, 8_000);
        assert_eq!(res_decl.output_tokens, 1_000);
        assert_eq!(res_decl.cache_read_tokens, 12_000);
        assert!(res_decl.estimated_cost_micro_usd > 0);

        let bad_line = r#"{"hello":"world"}"#;
        let res_bad = test_custom_rule_line(&decl_rule, bad_line, &catalog);
        assert!(!res_bad.matched);
        assert!(res_bad.diagnostic.contains("model_path"));
    }
}
