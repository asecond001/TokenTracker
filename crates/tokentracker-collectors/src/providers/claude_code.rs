use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokentracker_core::domain::{
    compute_event_id, provider_ids, sanitize_project_label, NormalizedUsageEvent,
};

use crate::error::CollectorResult;
use crate::providers::{
    collapse_duplicate_events_keep_last, compute_tps, get_i64, now_ms, parse_timestamp_ms,
    user_home_dir, FileParseContext, ParsedLineResult, ProviderLogParser,
};

#[derive(Debug, Default, Clone)]
pub struct ClaudeCodeParser;

impl ClaudeCodeParser {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderLogParser for ClaudeCodeParser {
    fn provider_id(&self) -> &str {
        provider_ids::CLAUDE_CODE
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        let Some(home) = user_home_dir() else {
            return Vec::new();
        };
        vec![home.join(".claude").join("projects")]
    }

    fn matches_file(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
    }

    fn init_context(&self, path: &Path, device_id: &str) -> FileParseContext {
        let session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("claude-session")
            .to_string();
        let parent_label = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(sanitize_claude_project_dir)
            .unwrap_or_else(|| "workspace".to_string());

        FileParseContext {
            file_path: path.to_string_lossy().to_string(),
            device_id: device_id.to_string(),
            session_id,
            project_label: parent_label,
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

        if let Some(sid) = val.get("sessionId").and_then(|v| v.as_str()) {
            if !sid.trim().is_empty() {
                ctx.session_id = sid.trim().to_string();
            }
        }
        if let Some(cwd) = val.get("cwd").and_then(|v| v.as_str()) {
            ctx.project_label = sanitize_project_label(Some(cwd));
        }

        let ts_ms = parse_timestamp_ms(val.get("timestamp")).unwrap_or_else(now_ms);
        let entry_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if entry_type == "user" || entry_type == "tool_result" {
            ctx.last_prompt_ts_ms = Some(ts_ms);
            return None;
        }

        if entry_type != "assistant" {
            return None;
        }

        let message = val.get("message")?;
        let usage = message.get("usage")?;
        let model_id = message
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("claude-sonnet-4-5")
            .to_string();

        let input_tokens = get_i64(usage, &["input_tokens"]);
        let output_tokens = get_i64(usage, &["output_tokens"]);
        let cache_write_tokens = get_i64(usage, &["cache_creation_input_tokens"]);
        let cache_read_tokens = get_i64(usage, &["cache_read_input_tokens"]);
        let reasoning_tokens = get_i64(usage, &["thinking_tokens", "reasoning_tokens"]);

        let total = input_tokens
            + output_tokens
            + cache_write_tokens
            + cache_read_tokens
            + reasoning_tokens;
        if total == 0 || model_id == "<synthetic>" {
            return None;
        }

        let message_id = message
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let (duration_ms, ttft_ms) = if let Some(msg_id) = message_id {
            let turn = ctx.resolve_message_turn(msg_id, ts_ms);
            let anchor = turn.turn_prompt_ts_ms.unwrap_or(turn.first_chunk_ts_ms);
            let delta = ts_ms - anchor;
            let dur = if delta > 0 && delta <= 600_000 {
                Some(delta)
            } else {
                None
            };
            let ttft = turn.turn_prompt_ts_ms.and_then(|p| {
                let d = turn.first_chunk_ts_ms - p;
                if d > 0 && d <= 600_000 {
                    Some(d)
                } else {
                    None
                }
            });
            (dur, ttft)
        } else {
            let dur = ctx
                .last_prompt_ts_ms
                .take()
                .and_then(|prev| {
                    let delta = ts_ms - prev;
                    if delta > 0 && delta <= 600_000 {
                        Some(delta)
                    } else {
                        None
                    }
                });
            (dur, None)
        };
        let effective_output = output_tokens + reasoning_tokens;
        let output_tps = compute_tps(effective_output, duration_ms);

        // Prefer Anthropic's globally unique `message.id` so multi-block assistant records
        // referencing the same API response collapse idempotently rather than double-counting.
        let step_key = message_id
            .map(|id| (id.to_string(), 0i64))
            .unwrap_or_else(|| {
                let uuid = val
                    .get("uuid")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("line-{}", ctx.line_number));
                (uuid, ts_ms)
            });

        let event_id = compute_event_id(
            provider_ids::CLAUDE_CODE,
            &ctx.session_id,
            &step_key.0,
            step_key.1,
        );

        Some(ParsedLineResult {
            event: Some(NormalizedUsageEvent {
                event_id,
                device_id: ctx.device_id.clone(),
                provider_id: provider_ids::CLAUDE_CODE.to_string(),
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
        })
    }

    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        collapse_duplicate_events_keep_last(events)
    }
}

fn sanitize_claude_project_dir(encoded: &str) -> String {
    let trimmed = encoded.trim();
    if trimmed.is_empty() {
        return "workspace".to_string();
    }
    trimmed
        .rsplit(['-', '_', '/', '\\'])
        .find(|seg| !seg.is_empty())
        .unwrap_or(trimmed)
        .chars()
        .take(48)
        .collect()
}

/// Read-only summary parsed from `~/.claude/stats-cache.json` for historical cold-start reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClaudeStatsCacheSummary {
    pub version: Option<i64>,
    pub total_sessions: i64,
    pub total_messages: i64,
    pub daily_activity_days: usize,
    pub model_totals: HashMap<String, i64>,
}

/// Read `~/.claude/stats-cache.json` strictly read-only with shared read access.
pub fn read_claude_stats_cache(path: &Path) -> CollectorResult<Option<ClaudeStatsCacheSummary>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.share_mode(0x0000_0001 | 0x0000_0002 | 0x0000_0004);
    }
    let mut file = opts.open(path)?;
    let mut raw = String::new();
    file.read_to_string(&mut raw)?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let Ok(val) = serde_json::from_str::<Value>(&raw) else {
        return Ok(None);
    };

    let version = val.get("version").and_then(|v| v.as_i64());
    let total_sessions = get_i64(&val, &["totalSessions", "total_sessions"]);
    let total_messages = get_i64(&val, &["totalMessages", "total_messages"]);
    let daily_activity_days = val
        .get("dailyActivity")
        .and_then(|v| v.as_array())
        .map(Vec::len)
        .unwrap_or(0);

    let mut model_totals = HashMap::new();
    if let Some(model_usage) = val.get("modelUsage").and_then(|v| v.as_object()) {
        for (model, stats) in model_usage {
            let sum = get_i64(stats, &["inputTokens", "input_tokens"])
                + get_i64(stats, &["outputTokens", "output_tokens"])
                + get_i64(stats, &["cacheReadInputTokens", "cache_read_input_tokens"])
                + get_i64(stats, &["cacheCreationInputTokens", "cache_creation_input_tokens"]);
            model_totals.insert(model.clone(), sum);
        }
    }

    Ok(Some(ClaudeStatsCacheSummary {
        version,
        total_sessions,
        total_messages,
        daily_activity_days,
        model_totals,
    }))
}

pub fn default_claude_stats_cache_path() -> Option<PathBuf> {
    user_home_dir().map(|h| h.join(".claude").join("stats-cache.json"))
}

// Keep unused `File` import warning away on non-windows or ensure clean imports
#[allow(dead_code)]
fn _touch_file_type(_: Option<File>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn compute_tps_rejects_sub_300ms_small_outputs_and_spikes() {
        assert_eq!(compute_tps(500, Some(100)), None);
        assert_eq!(compute_tps(7, Some(1_000)), None);
        assert_eq!(compute_tps(5_000, Some(1_000)), None);
        let ok = compute_tps(120, Some(2_000)).expect("valid tps");
        assert!((ok - 60.0).abs() < 1e-6);
    }

    #[test]
    fn multi_chunk_streaming_preserves_turn_duration_and_ttft() {
        let parser = ClaudeCodeParser;
        let path = Path::new("C:/tmp/.claude/projects/alpha/sess-stream.jsonl");
        let mut ctx = parser.init_context(path, "dev-1");

        let prompt = r#"{"type":"user","timestamp":"2026-11-10T10:00:00.000Z","message":{"role":"user","content":"hi"}}"#;
        let chunk1 = r#"{"type":"assistant","timestamp":"2026-11-10T10:00:00.250Z","message":{"id":"msg_stream_1","model":"claude-sonnet-4-5","usage":{"input_tokens":1000,"output_tokens":10,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        let chunk2 = r#"{"type":"assistant","timestamp":"2026-11-10T10:00:02.000Z","message":{"id":"msg_stream_1","model":"claude-sonnet-4-5","usage":{"input_tokens":1000,"output_tokens":160,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;

        assert!(parser.parse_line(prompt, &mut ctx).is_none());
        let e1 = parser
            .parse_line(chunk1, &mut ctx)
            .and_then(|r| r.event)
            .expect("chunk1");
        assert_eq!(e1.duration_ms, Some(250));
        assert_eq!(e1.ttft_ms, Some(250));
        // chunk1 duration < 300ms => output_tps rejected
        assert_eq!(e1.output_tps, None);

        let e2 = parser
            .parse_line(chunk2, &mut ctx)
            .and_then(|r| r.event)
            .expect("chunk2");
        assert_eq!(e2.duration_ms, Some(2_000));
        assert_eq!(e2.ttft_ms, Some(250));
        let tps = e2.output_tps.expect("chunk2 tps");
        assert!((tps - 80.0).abs() < 1e-6);

        let collapsed = parser.finalize_batch(vec![e1, e2]);
        assert_eq!(collapsed.len(), 1);
        assert_eq!(collapsed[0].output_tokens, 160);
        assert_eq!(collapsed[0].duration_ms, Some(2_000));
        assert_eq!(collapsed[0].ttft_ms, Some(250));
        assert!((collapsed[0].output_tps.expect("final tps") - 80.0).abs() < 1e-6);
    }

    #[test]
    fn multi_chunk_without_prompt_uses_first_chunk_anchor_and_rejects_fast_tail() {
        let parser = ClaudeCodeParser;
        let path = Path::new("C:/tmp/.claude/projects/alpha/sess-noprompt.jsonl");
        let mut ctx = parser.init_context(path, "dev-1");

        let chunk1 = r#"{"type":"assistant","timestamp":"2026-11-10T10:00:01.000Z","message":{"id":"msg_np","model":"claude-sonnet-4-5","usage":{"input_tokens":500,"output_tokens":5}}}"#;
        let fast_chunk = r#"{"type":"assistant","timestamp":"2026-11-10T10:00:01.100Z","message":{"id":"msg_np","model":"claude-sonnet-4-5","usage":{"input_tokens":500,"output_tokens":80}}}"#;
        let final_chunk = r#"{"type":"assistant","timestamp":"2026-11-10T10:00:03.000Z","message":{"id":"msg_np","model":"claude-sonnet-4-5","usage":{"input_tokens":500,"output_tokens":120}}}"#;

        let e1 = parser.parse_line(chunk1, &mut ctx).and_then(|r| r.event).expect("c1");
        assert_eq!(e1.duration_ms, None);
        assert_eq!(e1.output_tps, None);

        let e2 = parser.parse_line(fast_chunk, &mut ctx).and_then(|r| r.event).expect("c2");
        assert_eq!(e2.duration_ms, Some(100));
        assert_eq!(e2.output_tps, None);

        let e3 = parser.parse_line(final_chunk, &mut ctx).and_then(|r| r.event).expect("c3");
        assert_eq!(e3.duration_ms, Some(2_000));
        assert_eq!(e3.ttft_ms, None);
        let tps = e3.output_tps.expect("final tps");
        assert!((tps - 60.0).abs() < 1e-6);
    }
}
