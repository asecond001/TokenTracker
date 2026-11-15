use std::path::{Path, PathBuf};

use serde_json::Value;
use tokentracker_core::domain::{compute_event_id, provider_ids, NormalizedUsageEvent};

use crate::providers::{
    compute_tps, get_i64, now_ms, parse_timestamp_ms, user_home_dir, FileParseContext,
    ParsedLineResult, ProviderLogParser,
};

#[derive(Debug, Default, Clone)]
pub struct AntigravityParser;

impl AntigravityParser {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderLogParser for AntigravityParser {
    fn provider_id(&self) -> &'static str {
        provider_ids::ANTIGRAVITY
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        let Some(home) = user_home_dir() else {
            return Vec::new();
        };
        vec![home.join(".gemini").join("antigravity").join("brain")]
    }

    fn matches_file(&self, path: &Path) -> bool {
        // Strictly parse compact `transcript.jsonl` inside `.system_generated/logs/`,
        // skipping `transcript_full.jsonl` to prevent duplicate counting of the same conversation.
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !file_name.eq_ignore_ascii_case("transcript.jsonl") {
            return false;
        }
        let parent_is_logs = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("logs"));
        parent_is_logs
    }

    fn init_context(&self, path: &Path, device_id: &str) -> FileParseContext {
        // Path shape: .../<conversation-id>/.system_generated/logs/transcript.jsonl
        let session_id = path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("antigravity-conv")
            .to_string();

        FileParseContext {
            file_path: path.to_string_lossy().to_string(),
            device_id: device_id.to_string(),
            session_id,
            project_label: "antigravity".to_string(),
            current_model: Some("antigravity-planner".to_string()),
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

        let ts_ms = parse_timestamp_ms(val.get("created_at").or_else(|| val.get("timestamp")))
            .unwrap_or_else(now_ms);

        let prev_step_ts = ctx.last_step_ts_ms.replace(ts_ms);

        let source = val.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let step_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        // Extract explicit token counters when publicly recorded in transcript.jsonl
        let input_tokens = get_i64(&val, &["input_tokens", "prompt_tokens"]);
        let output_tokens = get_i64(&val, &["output_tokens", "completion_tokens"]);
        let cache_read_tokens = get_i64(&val, &["cache_read_tokens", "cached_tokens"]);
        let cache_write_tokens = get_i64(&val, &["cache_write_tokens"]);
        let reasoning_tokens = get_i64(&val, &["reasoning_tokens", "thinking_tokens"]);
        let total_tokens = input_tokens
            + output_tokens
            + cache_read_tokens
            + cache_write_tokens
            + reasoning_tokens;

        let upper_type = step_type.to_ascii_uppercase();
        let is_model_step = step_type.eq_ignore_ascii_case("PLANNER_RESPONSE")
            || upper_type.ends_with("_RESPONSE")
            || total_tokens > 0;
        let _ = source;
        if !is_model_step {
            return None;
        }

        let explicit_dur = val
            .get("duration_ms")
            .and_then(|v| v.as_i64())
            .filter(|&d| d > 0);
        let duration_ms = explicit_dur.or_else(|| {
            prev_step_ts.and_then(|prev| {
                let delta = ts_ms - prev;
                if delta > 0 && delta <= 600_000 {
                    Some(delta)
                } else {
                    None
                }
            })
        });

        let observable_only = total_tokens == 0;
        let effective_output = output_tokens + reasoning_tokens;
        let output_tps = compute_tps(effective_output, duration_ms);

        let model_id = val
            .get("model")
            .or_else(|| val.get("model_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("antigravity-planner")
            .to_string();

        let step_key = val
            .get("step_index")
            .and_then(|v| v.as_i64())
            .map(|idx| format!("step-{idx}"))
            .unwrap_or_else(|| format!("line-{}", ctx.line_number));

        let event_id = compute_event_id(
            provider_ids::ANTIGRAVITY,
            &ctx.session_id,
            &step_key,
            ts_ms,
        );

        Some(ParsedLineResult {
            event: Some(NormalizedUsageEvent {
                event_id,
                device_id: ctx.device_id.clone(),
                provider_id: provider_ids::ANTIGRAVITY.to_string(),
                session_id: ctx.session_id.clone(),
                project_label: ctx.project_label.clone(),
                model_id,
                timestamp_ms: ts_ms,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
                reasoning_tokens,
                observable_only,
                duration_ms,
                ttft_ms: val.get("ttft_ms").and_then(|v| v.as_i64()),
                output_tps,
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            }),
            rate_limits: Vec::new(),
        })
    }
}
