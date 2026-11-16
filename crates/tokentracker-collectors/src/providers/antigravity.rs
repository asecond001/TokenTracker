use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;
use tokentracker_core::domain::{compute_event_id, provider_ids, NormalizedUsageEvent};

use crate::providers::{
    compute_tps, get_i64, now_ms, parse_timestamp_ms, user_home_dir, FileParseContext,
    ParsedLineResult, ProviderLogParser,
};
use crate::watcher::open_shared_readonly;

const DEFAULT_ANTIGRAVITY_MODEL: &str = "antigravity-planner";
const SETTINGS_CHANGE_OPEN: &str = "<USER_SETTINGS_CHANGE>";
const SETTINGS_CHANGE_CLOSE: &str = "</USER_SETTINGS_CHANGE>";

/// Extract the selected model name from Antigravity's `<USER_SETTINGS_CHANGE>` block
/// inside `USER_INPUT` content, e.g.:
/// `The user changed setting \`Model Selection\` from None to Claude Sonnet 5.5 (Medium). ...`
fn extract_model_selection_from_content(content: &str) -> Option<String> {
    let start = content.find(SETTINGS_CHANGE_OPEN)? + SETTINGS_CHANGE_OPEN.len();
    let rest = &content[start..];
    let block = match rest.find(SETTINGS_CHANGE_CLOSE) {
        Some(end) => &rest[..end],
        None => rest,
    };

    let setting_pos = block.find("Model Selection")?;
    let after_setting = &block[setting_pos + "Model Selection".len()..];
    let to_pos = after_setting.find(" to ")?;
    let after_to = after_setting[to_pos + " to ".len()..].trim_start();

    // Stop at newline or sentence boundary (". " or trailing "."), preserving dots in version numbers like "5.5".
    let line_part = after_to.lines().next().unwrap_or(after_to).trim();
    let before_sentence_end = line_part
        .split_once(". ")
        .map(|(head, _)| head)
        .unwrap_or(line_part)
        .trim_end_matches('.')
        .trim();

    if before_sentence_end.is_empty() || before_sentence_end.eq_ignore_ascii_case("none") {
        return None;
    }

    Some(before_sentence_end.to_string())
}

/// Scan an existing `transcript.jsonl` read-only for `<USER_SETTINGS_CHANGE>` entries so
/// incremental tailing from a non-zero byte offset still knows the active session model.
fn resolve_initial_model_from_file(path: &Path) -> Option<String> {
    let file = open_shared_readonly(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut active_model: Option<String> = None;

    while reader.read_line(&mut line).ok()? > 0 {
        if line.contains(SETTINGS_CHANGE_OPEN) && line.contains("Model Selection") {
            if let Ok(val) = serde_json::from_str::<Value>(&line) {
                if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                    if let Some(model) = extract_model_selection_from_content(content) {
                        active_model = Some(model);
                    }
                }
            }
        }
        line.clear();
    }

    active_model
}

#[derive(Debug, Default, Clone)]
pub struct AntigravityParser;

impl AntigravityParser {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderLogParser for AntigravityParser {
    fn provider_id(&self) -> &str {
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

        let initial_model =
            resolve_initial_model_from_file(path).unwrap_or_else(|| DEFAULT_ANTIGRAVITY_MODEL.to_string());

        FileParseContext {
            file_path: path.to_string_lossy().to_string(),
            device_id: device_id.to_string(),
            session_id,
            project_label: "antigravity".to_string(),
            current_model: Some(initial_model),
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

        ctx.last_step_ts_ms = Some(ts_ms);

        let source = val.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let step_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if let Some(title) = val
            .get("title")
            .or_else(|| val.get("conversation_title"))
            .and_then(|v| v.as_str())
        {
            ctx.record_session_title(title, "explicit", ts_ms);
        }

        let content_opt = val.get("content").and_then(|v| v.as_str());
        if let Some(content) = content_opt {
            if let Some(selected_model) = extract_model_selection_from_content(content) {
                ctx.current_model = Some(selected_model);
            }
        }

        let is_user_step = step_type.eq_ignore_ascii_case("USER_INPUT")
            || source.eq_ignore_ascii_case("USER_EXPLICIT");
        if is_user_step {
            ctx.last_prompt_ts_ms = Some(ts_ms);
            if let Some(content) = content_opt {
                let user_req = if let Some(s) = content.find("<USER_REQUEST>") {
                    let rest = &content[s + "<USER_REQUEST>".len()..];
                    rest.split_once("</USER_REQUEST>")
                        .map(|(head, _)| head)
                        .unwrap_or(rest)
                } else if let Some(start_sc) = content.find(SETTINGS_CHANGE_OPEN) {
                    if let Some(end_sc) = content[start_sc..].find(SETTINGS_CHANGE_CLOSE) {
                        &content[start_sc + end_sc + SETTINGS_CHANGE_CLOSE.len()..]
                    } else {
                        content
                    }
                } else {
                    content
                };
                ctx.record_session_title(user_req, "derived", ts_ms);
            }
        }

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
        if !is_model_step {
            return None;
        }

        let explicit_dur = val
            .get("duration_ms")
            .and_then(|v| v.as_i64())
            .filter(|&d| d > 0);
        // Only fall back to explicit USER_INPUT prompt timestamp, never arbitrary preceding system/tool steps.
        let duration_ms = explicit_dur.or_else(|| {
            ctx.last_prompt_ts_ms.take().and_then(|prompt_ts| {
                let delta = ts_ms - prompt_ts;
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
            .map(ToString::to_string)
            .or_else(|| ctx.current_model.clone())
            .unwrap_or_else(|| DEFAULT_ANTIGRAVITY_MODEL.to_string());

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_model_selection_preserving_version_dots_and_ignoring_plain_text() {
        let claude = "<USER_REQUEST>hi</USER_REQUEST>\n<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from None to Claude Sonnet 5.5 (Medium). No need to comment on this change.\n</USER_SETTINGS_CHANGE>";
        assert_eq!(
            extract_model_selection_from_content(claude).as_deref(),
            Some("Claude Sonnet 5.5 (Medium)")
        );

        let hydrogen = "<USER_SETTINGS_CHANGE>The user changed setting `Model Selection` from None to Gemini 4 Hydrogen (Medium).</USER_SETTINGS_CHANGE>";
        assert_eq!(
            extract_model_selection_from_content(hydrogen).as_deref(),
            Some("Gemini 4 Hydrogen (Medium)")
        );

        let orchestrator = "<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from Gemini 4 Argon (Medium) to Orchestrator (Argon). No need to comment.\n</USER_SETTINGS_CHANGE>";
        assert_eq!(
            extract_model_selection_from_content(orchestrator).as_deref(),
            Some("Orchestrator (Argon)")
        );

        let plain_prose = "<USER_REQUEST>The user changed setting `Model Selection` from None to FakeModel.</USER_REQUEST>";
        assert_eq!(extract_model_selection_from_content(plain_prose), None);
    }

    #[test]
    fn tracks_model_changes_across_session_and_prefers_explicit_model_field() {
        let parser = AntigravityParser::new();
        let mut ctx = parser.init_context(
            Path::new("/tmp/conv-abc/.system_generated/logs/transcript.jsonl"),
            "dev-1",
        );

        let line1 = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-15T16:34:58Z","content":"<USER_REQUEST>hello</USER_REQUEST>\n<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from None to Claude Sonnet 5.5 (Medium). No need to comment.\n</USER_SETTINGS_CHANGE>"}"#;
        assert!(parser.parse_line(line1, &mut ctx).is_none());

        let line2 = r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-15T16:35:00Z","input_tokens":1000,"output_tokens":50}"#;
        let ev1 = parser
            .parse_line(line2, &mut ctx)
            .and_then(|r| r.event)
            .expect("planner response");
        assert_eq!(ev1.model_id, "Claude Sonnet 5.5 (Medium)");

        let line3 = r#"{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-15T16:36:00Z","content":"<USER_SETTINGS_CHANGE>The user changed setting `Model Selection` from Claude Sonnet 5.5 (Medium) to Gemini 4 Hydrogen (Medium).</USER_SETTINGS_CHANGE>"}"#;
        assert!(parser.parse_line(line3, &mut ctx).is_none());

        let line4 = r#"{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-15T16:36:05Z","input_tokens":500,"output_tokens":20}"#;
        let ev2 = parser
            .parse_line(line4, &mut ctx)
            .and_then(|r| r.event)
            .expect("second response");
        assert_eq!(ev2.model_id, "Gemini 4 Hydrogen (Medium)");

        let line5 = r#"{"step_index":4,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-15T16:36:10Z","model":"gemini-3-pro","input_tokens":200,"output_tokens":10}"#;
        let ev3 = parser
            .parse_line(line5, &mut ctx)
            .and_then(|r| r.event)
            .expect("explicit model response");
        assert_eq!(ev3.model_id, "gemini-3-pro");

        // Session title extracted cleanly from <USER_REQUEST>, ignoring preceding non-user step timing
        assert_eq!(
            ctx.session_title.as_ref().map(|t| t.0.as_str()),
            Some("hello")
        );
        // Step 4 followed step 3 without a new USER_INPUT prompt: must NOT divide 10 tokens by 5s prev_step
        assert_eq!(ev3.duration_ms, None);
    }
}
