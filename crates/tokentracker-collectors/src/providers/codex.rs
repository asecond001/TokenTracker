use std::path::{Path, PathBuf};

use serde_json::Value;
use tokentracker_core::domain::{
    compute_event_id, provider_ids, sanitize_project_label, NormalizedUsageEvent,
    ObservedRateLimitWindow,
};

use crate::providers::{
    compute_tps, extract_text_from_content, get_i64, now_ms, parse_timestamp_ms, user_home_dir,
    CodexRawTotals, FileParseContext, ParsedLineResult, ProviderLogParser,
};

#[derive(Debug, Default, Clone)]
pub struct CodexParser;

impl CodexParser {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderLogParser for CodexParser {
    fn provider_id(&self) -> &str {
        provider_ids::CODEX
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        let Some(home) = user_home_dir() else {
            return Vec::new();
        };
        vec![home.join(".codex").join("sessions")]
    }

    fn matches_file(&self, path: &Path) -> bool {
        let is_jsonl = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"));
        let is_rollout = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rollout-"));
        is_jsonl && is_rollout
    }

    fn init_context(&self, path: &Path, device_id: &str) -> FileParseContext {
        let session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("codex-session")
            .to_string();

        FileParseContext {
            file_path: path.to_string_lossy().to_string(),
            device_id: device_id.to_string(),
            session_id,
            project_label: "workspace".to_string(),
            current_model: Some("gpt-5-codex".to_string()),
            ..Default::default()
        }
    }

    fn parse_line(&self, line: &str, ctx: &mut FileParseContext) -> Option<ParsedLineResult> {
        ctx.line_number += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            return None;
        };

        let ts_ms = parse_timestamp_ms(val.get("timestamp")).unwrap_or_else(now_ms);
        let top_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let payload = val.get("payload")?;

        match top_type {
            "session_meta" => {
                if let Some(id) = payload.get("id").and_then(|v| v.as_str()) {
                    if !id.trim().is_empty() {
                        ctx.session_id = id.trim().to_string();
                    }
                }
                if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                    ctx.project_label = sanitize_project_label(Some(cwd));
                }
                if let Some(model) = payload.get("model").and_then(|v| v.as_str()) {
                    if !model.trim().is_empty() {
                        ctx.current_model = Some(model.trim().to_string());
                    }
                }
                if let Some(title) = payload
                    .get("title")
                    .or_else(|| payload.get("name"))
                    .and_then(|v| v.as_str())
                {
                    ctx.record_session_title(title, "explicit", ts_ms);
                }
                None
            }
            "turn_context" => {
                if let Some(model) = payload.get("model").and_then(|v| v.as_str()) {
                    if !model.trim().is_empty() {
                        ctx.current_model = Some(model.trim().to_string());
                    }
                }
                if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                    ctx.project_label = sanitize_project_label(Some(cwd));
                }
                // Only seed prompt timestamp from turn_context when no explicit user turn exists yet;
                // otherwise mid-turn turn_context emissions right before token_count cause fake TPS spikes.
                if ctx.last_prompt_ts_ms.is_none() {
                    ctx.last_prompt_ts_ms = Some(ts_ms);
                }
                None
            }
            "response_item" => {
                let role = payload.get("role").and_then(|v| v.as_str()).unwrap_or("");
                if role == "user" {
                    ctx.last_prompt_ts_ms = Some(ts_ms);
                    if let Some(text) = extract_codex_user_prompt(payload.get("content")) {
                        ctx.record_session_title(&text, "derived", ts_ms);
                    }
                }
                None
            }
            "event_msg" => {
                let ev_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if ev_type == "user_message" {
                    ctx.last_prompt_ts_ms = Some(ts_ms);
                    if let Some(msg) = extract_text_from_content(
                        payload
                            .get("message")
                            .or_else(|| payload.get("text"))
                            .or_else(|| payload.get("content")),
                    ) {
                        ctx.record_session_title(&msg, "user_prompt", ts_ms);
                    }
                    return None;
                }
                if ev_type != "token_count" {
                    return None;
                }

                let rate_limits = extract_rate_limits(payload, ts_ms);

                let Some(info) = payload.get("info").filter(|i| !i.is_null()) else {
                    return if rate_limits.is_empty() {
                        None
                    } else {
                        Some(ParsedLineResult {
                            event: None,
                            rate_limits,
                        })
                    };
                };

                if let Some(model) = info
                    .get("model")
                    .or_else(|| payload.get("model"))
                    .and_then(|v| v.as_str())
                {
                    if !model.trim().is_empty() {
                        ctx.current_model = Some(model.trim().to_string());
                    }
                }

                let raw_totals = info.get("total_token_usage").map(read_codex_totals);
                if let (Some(curr_tot), Some(prev_tot)) = (raw_totals, ctx.last_codex_totals) {
                    if curr_tot == prev_tot {
                        // Identical repeated heartbeat snapshot; still pass rate_limits if present.
                        return if rate_limits.is_empty() {
                            None
                        } else {
                            Some(ParsedLineResult {
                                event: None,
                                rate_limits,
                            })
                        };
                    }
                }

                let delta_raw = if let Some(last_usage) = info.get("last_token_usage") {
                    read_codex_totals(last_usage)
                } else if let Some(curr_tot) = raw_totals {
                    match ctx.last_codex_totals {
                        Some(prev) => CodexRawTotals {
                            input_tokens: curr_tot.input_tokens.saturating_sub(prev.input_tokens),
                            cached_input_tokens: curr_tot
                                .cached_input_tokens
                                .saturating_sub(prev.cached_input_tokens),
                            output_tokens: curr_tot
                                .output_tokens
                                .saturating_sub(prev.output_tokens),
                            reasoning_output_tokens: curr_tot
                                .reasoning_output_tokens
                                .saturating_sub(prev.reasoning_output_tokens),
                        },
                        None => curr_tot,
                    }
                } else {
                    CodexRawTotals::default()
                };

                if let Some(curr_tot) = raw_totals {
                    ctx.last_codex_totals = Some(curr_tot);
                }

                // Normalize OpenAI's inclusive token counters into disjoint components:
                // OpenAI `input_tokens` includes `cached_input_tokens`, and `output_tokens`
                // includes `reasoning_output_tokens`.
                let cache_read_tokens = delta_raw.cached_input_tokens.min(delta_raw.input_tokens);
                let uncached_input = delta_raw.input_tokens.saturating_sub(cache_read_tokens);
                let reasoning_tokens = delta_raw
                    .reasoning_output_tokens
                    .min(delta_raw.output_tokens);
                let non_reasoning_output = delta_raw.output_tokens.saturating_sub(reasoning_tokens);

                let total = uncached_input + non_reasoning_output + cache_read_tokens + reasoning_tokens;
                if total == 0 {
                    return if rate_limits.is_empty() {
                        None
                    } else {
                        Some(ParsedLineResult {
                            event: None,
                            rate_limits,
                        })
                    };
                }

                let explicit_dur = info
                    .get("duration_ms")
                    .or_else(|| payload.get("duration_ms"))
                    .and_then(|v| v.as_i64())
                    .filter(|&d| d > 0);
                let duration_ms = explicit_dur.or_else(|| {
                    ctx.last_prompt_ts_ms.take().and_then(|prev| {
                        let d = ts_ms - prev;
                        if d > 0 && d <= 600_000 {
                            Some(d)
                        } else {
                            None
                        }
                    })
                });
                let effective_output = non_reasoning_output + reasoning_tokens;
                let output_tps = compute_tps(effective_output, duration_ms);

                let model_id = ctx
                    .current_model
                    .clone()
                    .unwrap_or_else(|| "gpt-5-codex".to_string());
                let step_key = raw_totals
                    .map(|t| {
                        format!(
                            "tot-{}-{}-{}-{}",
                            t.input_tokens,
                            t.cached_input_tokens,
                            t.output_tokens,
                            t.reasoning_output_tokens
                        )
                    })
                    .unwrap_or_else(|| format!("line-{}", ctx.line_number));
                let event_id = compute_event_id(
                    provider_ids::CODEX,
                    &ctx.session_id,
                    &step_key,
                    ts_ms,
                );

                Some(ParsedLineResult {
                    event: Some(NormalizedUsageEvent {
                        event_id,
                        device_id: ctx.device_id.clone(),
                        provider_id: provider_ids::CODEX.to_string(),
                        session_id: ctx.session_id.clone(),
                        project_label: ctx.project_label.clone(),
                        model_id,
                        timestamp_ms: ts_ms,
                        input_tokens: uncached_input,
                        output_tokens: non_reasoning_output,
                        cache_read_tokens,
                        cache_write_tokens: 0,
                        reasoning_tokens,
                        observable_only: false,
                        duration_ms,
                        ttft_ms: None,
                        output_tps,
                        estimated_cost_micro_usd: 0,
                        cache_saved_micro_usd: 0,
                    }),
                    rate_limits,
                })
            }
            _ => None,
        }
    }
}

fn is_codex_injected_preamble(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("# agents.md")
        || lower.starts_with("agents.md instructions")
        || lower.contains("<instructions>")
        || trimmed.starts_with("# 工作偏好")
        || trimmed.contains("这些是跨项目的工作偏好")
}

fn extract_codex_user_prompt(content: Option<&Value>) -> Option<String> {
    let val = content?;
    match val {
        Value::String(s) if !is_codex_injected_preamble(s) => {
            tokentracker_core::domain::sanitize_session_title(s)
        }
        Value::Array(items) => {
            for item in items {
                if let Some(s) = item
                    .get("text")
                    .or_else(|| item.get("input_text"))
                    .and_then(|v| v.as_str())
                {
                    if !is_codex_injected_preamble(s) {
                        if let Some(clean) = tokentracker_core::domain::sanitize_session_title(s) {
                            return Some(clean);
                        }
                    }
                }
            }
            None
        }
        _ => extract_text_from_content(Some(val)).filter(|s| !is_codex_injected_preamble(s)),
    }
}

fn read_codex_totals(obj: &Value) -> CodexRawTotals {
    CodexRawTotals {
        input_tokens: get_i64(obj, &["input_tokens"]),
        cached_input_tokens: get_i64(obj, &["cached_input_tokens"]),
        output_tokens: get_i64(obj, &["output_tokens"]),
        reasoning_output_tokens: get_i64(obj, &["reasoning_output_tokens"]),
    }
}

fn extract_rate_limits(payload: &Value, observed_at_ms: i64) -> Vec<ObservedRateLimitWindow> {
    let Some(rl) = payload.get("rate_limits").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let plan_hint = rl
        .get("plan_type")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let mut out = Vec::new();
    for (window_name, win_val) in rl {
        if win_val.is_null() {
            continue;
        }
        let Some(used_percent) = win_val.get("used_percent").and_then(|v| v.as_f64()) else {
            continue;
        };
        let window_minutes = win_val.get("window_minutes").and_then(|v| v.as_i64());
        let resets_at_ms = parse_timestamp_ms(win_val.get("resets_at")).or_else(|| {
            win_val
                .get("resets_in_seconds")
                .and_then(|v| v.as_i64())
                .map(|secs| observed_at_ms + secs.max(0) * 1000)
        });
        out.push(ObservedRateLimitWindow {
            provider_id: provider_ids::CODEX.to_string(),
            window_name: window_name.clone(),
            used_percent,
            window_minutes,
            resets_at_ms,
            observed_at_ms,
            source: Some("local_log".to_string()),
            plan_hint: plan_hint.clone(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn turn_context_does_not_overwrite_user_prompt_ts_and_extracts_title() {
        let parser = CodexParser::new();
        let mut ctx = parser.init_context(Path::new("C:/tmp/.codex/sessions/rollout-2026.jsonl"), "dev-1");

        let meta = r#"{"timestamp":"2026-11-16T01:00:00.000Z","type":"session_meta","payload":{"id":"codex-sess-1","cwd":"C:/Projects/TokenTracker"}}"#;
        let user_item = r#"{"timestamp":"2026-11-16T01:00:01.000Z","type":"response_item","payload":{"role":"user","content":[{"type":"input_text","text":"<environment_context><cwd>C:/Projects</cwd></environment_context>\nImplement floating monitor"}]}}"#;
        let turn_ctx = r#"{"timestamp":"2026-11-16T01:00:04.800Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#;
        let tok_ev = r#"{"timestamp":"2026-11-16T01:00:05.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":500,"cached_input_tokens":0,"output_tokens":240,"reasoning_output_tokens":0},"total_token_usage":{"input_tokens":500,"cached_input_tokens":0,"output_tokens":240,"reasoning_output_tokens":0}}}}"#;

        assert!(parser.parse_line(meta, &mut ctx).is_none());
        assert!(parser.parse_line(user_item, &mut ctx).is_none());
        assert!(parser.parse_line(turn_ctx, &mut ctx).is_none());
        let ev = parser
            .parse_line(tok_ev, &mut ctx)
            .and_then(|r| r.event)
            .expect("token_count event");

        // Must measure duration from user prompt (4000ms), NOT from turn_context (200ms -> 1200 tok/s spike)
        assert_eq!(ev.duration_ms, Some(4_000));
        let tps = ev.output_tps.expect("valid tps");
        assert!((tps - 60.0).abs() < 1e-6);
        assert_eq!(
            ctx.session_title.as_ref().map(|t| t.0.as_str()),
            Some("Implement floating monitor")
        );
    }
}
