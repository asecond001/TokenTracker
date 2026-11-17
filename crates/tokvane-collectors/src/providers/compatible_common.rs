use std::path::{Path, PathBuf};

use serde_json::Value;
use tokvane_core::domain::{
    compute_event_id, sanitize_project_label, NormalizedUsageEvent,
};

use crate::providers::{
    compute_tps, extract_text_from_content, get_i64, now_ms, parse_timestamp_ms, user_home_dir,
    FileParseContext, ParsedLineResult,
};

#[derive(Debug, Clone, Copy)]
pub struct CompatibleParserProfile {
    pub provider_id: &'static str,
    pub default_model: &'static str,
    pub prefer_routed_model: bool,
}

pub fn appdata_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
}

pub fn local_appdata_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| user_home_dir().map(|h| h.join(".local").join("share")))
}

pub fn init_generic_context(
    path: &Path,
    device_id: &str,
    default_session_prefix: &str,
    default_model: &str,
) -> FileParseContext {
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_session_prefix)
        .to_string();

    let project_label = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .filter(|n| !n.eq_ignore_ascii_case("sessions") && !n.eq_ignore_ascii_case("logs"))
        .map(|n| n.chars().take(48).collect())
        .unwrap_or_else(|| "workspace".to_string());

    FileParseContext {
        file_path: path.to_string_lossy().to_string(),
        device_id: device_id.to_string(),
        session_id,
        project_label,
        current_model: Some(default_model.to_string()),
        ..Default::default()
    }
}

pub fn default_home_and_appdata_roots(subpaths: &[&str], appdata_subpaths: &[&str]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = user_home_dir() {
        for sub in subpaths {
            roots.push(home.join(sub));
        }
    }
    if let Some(app) = appdata_dir() {
        for sub in appdata_subpaths {
            roots.push(app.join(sub));
        }
    }
    roots
}

fn non_empty_str(val: Option<&Value>) -> Option<&str> {
    val.and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn first_str<'a>(obj: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for k in keys {
        if let Some(s) = non_empty_str(obj.get(*k)) {
            return Some(s);
        }
    }
    None
}

fn extract_routed_or_model(
    root: &Value,
    envelope: Option<&Value>,
    prefer_routed: bool,
) -> Option<String> {
    let routed_keys = [
        "routed_model",
        "backend_model",
        "actual_model",
        "upstream_model",
        "target_model",
    ];
    let std_keys = ["model", "model_id", "model_name", "modelName"];
    let nested_payload = envelope.and_then(|e| e.get("payload"));

    if prefer_routed {
        if let Some(s) = first_str(root, &routed_keys) {
            return Some(s.to_string());
        }
        if let Some(env) = envelope {
            if let Some(s) = first_str(env, &routed_keys) {
                return Some(s.to_string());
            }
        }
        if let Some(payload) = nested_payload {
            if let Some(s) = first_str(payload, &routed_keys) {
                return Some(s.to_string());
            }
        }
        if let Some(meta) = root.get("metadata") {
            if let Some(s) = first_str(meta, &routed_keys) {
                return Some(s.to_string());
            }
        }
    }

    if let Some(payload) = nested_payload {
        if let Some(s) = first_str(payload, &std_keys) {
            return Some(s.to_string());
        }
    }
    if let Some(env) = envelope {
        if let Some(s) = first_str(env, &std_keys) {
            return Some(s.to_string());
        }
    }
    if let Some(s) = first_str(root, &std_keys) {
        return Some(s.to_string());
    }
    None
}

pub fn extract_usage_tokens(usage: &Value) -> (i64, i64, i64, i64, i64) {
    let has_any = |keys: &[&str]| keys.iter().any(|k| usage.get(*k).is_some());

    let cache_write_tokens = get_i64(
        usage,
        &[
            "cache_creation_input_tokens",
            "cache_write_tokens",
            "cache_write_input_tokens",
            "input_cache_creation",
            "cacheCreationInputTokens",
            "cacheWriteTokens",
            "inputCacheCreation",
        ],
    );

    let nested_cached = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .or_else(|| usage.pointer("/promptTokensDetails/cachedTokens"))
        .or_else(|| usage.pointer("/inputTokensDetails/cachedTokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .max(0);

    let cache_hit = get_i64(
        usage,
        &[
            "prompt_cache_hit_tokens",
            "input_cache_read",
            "promptCacheHitTokens",
            "inputCacheRead",
        ],
    );
    let direct_cached = get_i64(
        usage,
        &[
            "cache_read_input_tokens",
            "cache_read_tokens",
            "cached_tokens",
            "cacheReadInputTokens",
            "cacheReadTokens",
            "cachedTokens",
        ],
    );
    let cache_read_tokens = cache_hit.max(direct_cached).max(nested_cached);

    let input_tokens = if has_any(&["prompt_cache_miss_tokens", "promptCacheMissTokens"]) {
        get_i64(usage, &["prompt_cache_miss_tokens", "promptCacheMissTokens"])
    } else if has_any(&["input_other", "inputOther"]) {
        get_i64(usage, &["input_other", "inputOther"])
    } else if has_any(&["cache_read_input_tokens", "cacheReadInputTokens"])
        && has_any(&["input_tokens", "inputTokens"])
    {
        // Anthropic convention: `input_tokens` already excludes cache_read / cache_creation.
        get_i64(usage, &["input_tokens", "inputTokens"])
    } else if has_any(&["prompt_tokens", "promptTokens"]) {
        // OpenAI convention: `prompt_tokens` includes `cached_tokens`.
        get_i64(usage, &["prompt_tokens", "promptTokens"]).saturating_sub(cache_read_tokens)
    } else {
        let raw_in = get_i64(usage, &["input_tokens", "inputTokens", "input"]);
        if has_any(&[
            "input_tokens_details",
            "inputTokensDetails",
            "cached_tokens",
            "cachedTokens",
        ]) {
            raw_in.saturating_sub(cache_read_tokens)
        } else {
            raw_in
        }
    };

    let nested_reasoning = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .or_else(|| usage.pointer("/output_tokens_details/reasoning_tokens"))
        .or_else(|| usage.pointer("/completionTokensDetails/reasoningTokens"))
        .or_else(|| usage.pointer("/outputTokensDetails/reasoningTokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .max(0);

    let reasoning_tokens = get_i64(
        usage,
        &[
            "reasoning_tokens",
            "thinking_tokens",
            "completion_reasoning_tokens",
            "reasoningTokens",
            "thinkingTokens",
        ],
    )
    .max(nested_reasoning);

    let raw_out = get_i64(
        usage,
        &[
            "output_tokens",
            "completion_tokens",
            "output",
            "outputTokens",
            "completionTokens",
        ],
    );
    let output_tokens = if reasoning_tokens > 0
        && raw_out >= reasoning_tokens
        && has_any(&[
            "completion_tokens",
            "completionTokens",
            "completion_tokens_details",
            "output_tokens_details",
            "completionTokensDetails",
            "outputTokensDetails",
        ]) {
        raw_out.saturating_sub(reasoning_tokens)
    } else {
        raw_out
    };

    (
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        reasoning_tokens,
    )
}

fn explicit_duration_ms(root: &Value, envelope: Option<&Value>) -> Option<i64> {
    for obj in [Some(root), envelope].into_iter().flatten() {
        let val = obj
            .get("duration_ms")
            .or_else(|| obj.get("latency_ms"))
            .or_else(|| obj.get("elapsed_ms"));
        if let Some(d) = val.and_then(|v| v.as_i64()).filter(|&d| d > 0 && d <= 600_000) {
            return Some(d);
        }
        if let Some(secs) = obj
            .get("duration_sec")
            .or_else(|| obj.get("duration_s"))
            .and_then(|v| v.as_f64())
            .filter(|s| s.is_finite() && *s > 0.0 && *s <= 600.0)
        {
            return Some((secs * 1000.0).round() as i64);
        }
    }
    None
}

fn explicit_ttft_ms(root: &Value, envelope: Option<&Value>) -> Option<i64> {
    for obj in [Some(root), envelope].into_iter().flatten() {
        let val = obj.get("ttft_ms").or_else(|| obj.get("first_token_ms"));
        if let Some(t) = val.and_then(|v| v.as_i64()).filter(|&t| t > 0 && t <= 600_000) {
            return Some(t);
        }
    }
    None
}

pub fn parse_compatible_jsonl_line(
    line: &str,
    ctx: &mut FileParseContext,
    profile: CompatibleParserProfile,
) -> Option<ParsedLineResult> {
    ctx.line_number += 1;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
        return None;
    };

    if let Some(sid) = first_str(&val, &["session_id", "sessionId", "conversation_id", "chat_id"])
    {
        ctx.session_id = sid.to_string();
    }
    let envelope = val
        .get("message")
        .or_else(|| val.get("params"))
        .or_else(|| val.get("response"))
        .or_else(|| val.get("payload"))
        .or_else(|| val.get("data"));
    let nested_payload = envelope.and_then(|e| e.get("payload"));

    if let Some(cwd) = first_str(
        &val,
        &["cwd", "workspace", "workspace_dir", "project_dir", "project_root", "work_dir"],
    )
    .or_else(|| {
        envelope.and_then(|e| {
            first_str(
                e,
                &["cwd", "workspace", "workspace_dir", "project_dir", "project_root", "work_dir"],
            )
        })
    })
    .or_else(|| {
        nested_payload.and_then(|p| {
            first_str(
                p,
                &["cwd", "workspace", "workspace_dir", "project_dir", "project_root", "work_dir"],
            )
        })
    }) {
        ctx.project_label = sanitize_project_label(Some(cwd));
    }

    if let Some(model) = extract_routed_or_model(&val, envelope, profile.prefer_routed_model) {
        ctx.current_model = Some(model);
    }

    let is_cumulative_scope = [Some(&val), envelope, nested_payload]
        .into_iter()
        .flatten()
        .filter_map(|obj| first_str(obj, &["scope"]))
        .any(|s| {
            s.eq_ignore_ascii_case("session")
                || s.eq_ignore_ascii_case("cumulative")
                || s.eq_ignore_ascii_case("total")
        });
    if is_cumulative_scope {
        return None;
    }

    let ts_ms = parse_timestamp_ms(
        val.get("timestamp")
            .or_else(|| val.get("created_at"))
            .or_else(|| val.get("time"))
            .or_else(|| val.get("ts"))
            .or_else(|| val.get("created"))
            .or_else(|| {
                envelope.and_then(|e| {
                    e.get("timestamp")
                        .or_else(|| e.get("created_at"))
                        .or_else(|| e.get("created"))
                })
            })
            .or_else(|| {
                nested_payload.and_then(|p| p.get("timestamp").or_else(|| p.get("created_at")))
            }),
    )
    .unwrap_or_else(now_ms);

    let role_or_type = first_str(&val, &["role", "type", "event"])
        .or_else(|| envelope.and_then(|e| first_str(e, &["role", "type", "event"])))
        .unwrap_or("")
        .to_ascii_lowercase();

    if let Some(explicit_title) = first_str(&val, &["title", "session_title"])
        .or_else(|| envelope.and_then(|e| first_str(e, &["title", "session_title"])))
        .or_else(|| nested_payload.and_then(|p| first_str(p, &["title", "session_title"])))
    {
        ctx.record_session_title(explicit_title, "explicit", ts_ms);
    }

    if matches!(
        role_or_type.as_str(),
        "user"
            | "prompt"
            | "tool_result"
            | "turnbegin"
            | "turn_begin"
            | "user_message"
            | "usermessage"
            | "human"
    ) {
        if !matches!(role_or_type.as_str(), "tool_result") {
            let prompt_text = extract_text_from_content(
                nested_payload
                    .and_then(|p| {
                        p.get("user_input")
                            .or_else(|| p.get("content"))
                            .or_else(|| p.get("prompt"))
                            .or_else(|| p.get("text"))
                            .or_else(|| p.get("input"))
                            .or_else(|| p.get("message"))
                    })
                    .or_else(|| {
                        envelope.and_then(|e| {
                            e.get("user_input")
                                .or_else(|| e.get("content"))
                                .or_else(|| e.get("prompt"))
                                .or_else(|| e.get("text"))
                                .or_else(|| e.get("input"))
                                .or_else(|| e.get("message"))
                        })
                    })
                    .or_else(|| {
                        val.get("user_input")
                            .or_else(|| val.get("content"))
                            .or_else(|| val.get("prompt"))
                            .or_else(|| val.get("text"))
                            .or_else(|| val.get("input"))
                            .or_else(|| val.pointer("/message/content"))
                            .or_else(|| val.pointer("/message/payload/user_input"))
                            .or_else(|| val.pointer("/params/payload/user_input"))
                    }),
            );
            if let Some(text) = prompt_text {
                ctx.record_session_title(&text, "user_prompt", ts_ms);
            }
        }
        ctx.last_prompt_ts_ms = Some(ts_ms);
        return None;
    }

    if ctx.session_title.is_none() {
        let inline_prompt = extract_text_from_content(
            nested_payload
                .and_then(|p| p.get("user_input").or_else(|| p.get("prompt")))
                .or_else(|| envelope.and_then(|e| e.get("user_input").or_else(|| e.get("prompt"))))
                .or_else(|| val.get("user_input").or_else(|| val.get("prompt"))),
        );
        if let Some(text) = inline_prompt {
            ctx.record_session_title(&text, "derived", ts_ms);
        }
    }

    let usage = envelope
        .and_then(|e| {
            e.get("usage")
                .or_else(|| e.get("token_usage"))
                .or_else(|| {
                    e.get("payload")
                        .and_then(|p| p.get("usage").or_else(|| p.get("token_usage")))
                })
        })
        .or_else(|| val.get("usage"))
        .or_else(|| val.get("token_usage"))
        .or_else(|| val.get("metrics").and_then(|m| m.get("usage")))?;

    let (input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens) =
        extract_usage_tokens(usage);
    let total = input_tokens
        + output_tokens
        + cache_read_tokens
        + cache_write_tokens
        + reasoning_tokens;
    if total == 0 {
        return None;
    }

    let model_id = extract_routed_or_model(&val, envelope, profile.prefer_routed_model)
        .or_else(|| ctx.current_model.clone())
        .unwrap_or_else(|| profile.default_model.to_string());
    if model_id == "<synthetic>" {
        return None;
    }

    let message_id = first_str(&val, &["request_id", "message_id", "response_id", "id", "uuid"])
        .or_else(|| envelope.and_then(|e| first_str(e, &["id", "request_id", "message_id"])));

    let explicit_dur = explicit_duration_ms(&val, envelope);
    let explicit_ttft = explicit_ttft_ms(&val, envelope);

    let effective_output = output_tokens + reasoning_tokens;
    let (duration_ms, ttft_ms, tps_tokens) = if let Some(dur) = explicit_dur {
        (Some(dur), explicit_ttft, effective_output)
    } else if let Some(msg_id) = message_id {
        let turn = ctx.resolve_message_turn(msg_id, ts_ms, effective_output);
        let anchor = turn.turn_prompt_ts_ms.unwrap_or(turn.first_chunk_ts_ms);
        let delta = ts_ms - anchor;
        let dur = (delta > 0 && delta <= 600_000).then_some(delta);
        let ttft = explicit_ttft.or_else(|| {
            turn.turn_prompt_ts_ms.and_then(|p| {
                let d = turn.first_chunk_ts_ms - p;
                (d > 0 && d <= 600_000).then_some(d)
            })
        });
        let tok = if turn.turn_prompt_ts_ms.is_some() {
            effective_output
        } else {
            effective_output.saturating_sub(turn.first_chunk_output_tokens)
        };
        (dur, ttft, tok)
    } else {
        let dur = ctx.last_prompt_ts_ms.take().and_then(|prev| {
            let delta = ts_ms - prev;
            (delta > 0 && delta <= 600_000).then_some(delta)
        });
        (dur, explicit_ttft, effective_output)
    };

    let output_tps = compute_tps(tps_tokens, duration_ms);

    let (step_key, step_ts) = if let Some(id) = message_id {
        (id.to_string(), 0i64)
    } else {
        (format!("line-{}", ctx.line_number), ts_ms)
    };

    let event_id = compute_event_id(profile.provider_id, &ctx.session_id, &step_key, step_ts);

    Some(ParsedLineResult {
        event: Some(NormalizedUsageEvent {
            event_id,
            device_id: ctx.device_id.clone(),
            provider_id: profile.provider_id.to_string(),
            session_id: ctx.session_id.clone(),
            project_label: ctx.project_label.clone(),
            model_id,
            timestamp_ms: ts_ms,
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
        }),
        rate_limits: Vec::new(),
        balance: None,
    })
}
