use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde_json::Value;
use tokentracker_core::domain::{
    compute_event_id, provider_ids, sanitize_session_title, NormalizedUsageEvent,
    ObservedRateLimitWindow,
};

use crate::providers::{
    collapse_duplicate_events_keep_last, compute_tps, get_i64, now_ms, parse_timestamp_ms,
    user_home_dir, FileParseContext, ParsedLineResult, ProviderLogParser,
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

/// Extract an explicit subscription / tier hint from settings or JSON tier metadata (NEVER from model names like Gemini Pro).
fn normalize_explicit_antigravity_tier(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "unknown" {
        return None;
    }
    if lower.contains("ultra") && (lower.contains("5x") || lower.contains("99")) {
        Some("ultra_5x".to_string())
    } else if lower.contains("ultra") {
        Some("ultra".to_string())
    } else if lower.contains("enterprise") || lower.contains("business") {
        Some("enterprise".to_string())
    } else if lower.contains("plus") {
        Some("plus".to_string())
    } else if lower.contains("free") || lower.contains("standard-tier") {
        Some("free".to_string())
    } else if lower == "pro"
        || lower.contains("g1-pro")
        || lower.contains("google ai pro")
        || lower.contains("google_ai_pro")
    {
        Some("pro".to_string())
    } else {
        None
    }
}

/// Infer Ultra/Enterprise tier ONLY from Ultra-exclusive model identifiers (e.g. Carbon backend);
/// never infer Pro from "Gemini Pro" models because Pro models are shared by Ultra & Pro plans.
fn infer_antigravity_tier_from_model_name(model_name: &str) -> Option<String> {
    let lower = model_name.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "none" {
        return None;
    }
    if lower.contains("ultra") && (lower.contains("5x") || lower.contains("99")) {
        Some("ultra_5x".to_string())
    } else if lower.contains("ultra") || lower.contains("carbon") || lower.contains("g1-ultra") {
        Some("ultra".to_string())
    } else if lower.contains("enterprise") {
        Some("enterprise".to_string())
    } else {
        None
    }
}

fn parse_settings_json_for_plan_hint(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let mut file = open_shared_readonly(path).ok()?;
    let mut raw = String::new();
    file.read_to_string(&mut raw).ok()?;
    let val = serde_json::from_str::<Value>(&raw).ok()?;

    for key in [
        "subscriptionTier",
        "subscription_tier",
        "planTier",
        "plan_tier",
        "quotaTier",
        "quota_tier",
        "userTier",
        "user_tier",
        "tier",
        "plan",
        "membership",
    ] {
        if let Some(s) = val.get(key).and_then(|v| v.as_str()) {
            if let Some(hint) = normalize_explicit_antigravity_tier(s) {
                return Some(hint);
            }
        }
    }

    for nested_val in [
        val.pointer("/agent/preferredModelTier"),
        val.pointer("/agent/preferred_model_tier"),
        val.pointer("/models/orchestratorBackend"),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(s) = nested_val.as_str() {
            if let Some(hint) = normalize_explicit_antigravity_tier(s) {
                return Some(hint);
            }
        }
    }

    None
}

/// Inspect documented public Antigravity `settings.json` locations read-only for subscription tier hints.
fn resolve_antigravity_plan_hint(transcript_path: &Path) -> Option<String> {
    // Check ancestor directories (e.g., <root>/settings.json or ~/.gemini/antigravity-cli/settings.json)
    let mut ancestor = transcript_path.parent();
    for _ in 0..5 {
        let Some(dir) = ancestor else {
            break;
        };
        if let Some(hint) = parse_settings_json_for_plan_hint(&dir.join("settings.json")) {
            return Some(hint);
        }
        ancestor = dir.parent();
    }

    if let Some(home) = user_home_dir() {
        let gemini_dir = home.join(".gemini");
        for candidate in [
            gemini_dir.join("antigravity-cli").join("settings.json"),
            gemini_dir.join("antigravity").join("settings.json"),
            gemini_dir.join("settings.json"),
        ] {
            if let Some(hint) = parse_settings_json_for_plan_hint(&candidate) {
                return Some(hint);
            }
        }
    }

    None
}

fn extract_antigravity_rate_limits(
    val: &Value,
    observed_at_ms: i64,
    fallback_hint: Option<&str>,
) -> Vec<ObservedRateLimitWindow> {
    let explicit_hint = val
        .get("plan_tier")
        .or_else(|| val.get("subscription_tier"))
        .or_else(|| val.get("tier"))
        .or_else(|| val.get("plan"))
        .and_then(|v| v.as_str())
        .and_then(normalize_explicit_antigravity_tier)
        .or_else(|| fallback_hint.map(ToString::to_string));

    let mut out = Vec::new();
    let Some(rl_obj) = val
        .get("rate_limits")
        .or_else(|| val.get("rate_limit"))
        .or_else(|| val.get("quota"))
        .and_then(|v| v.as_object())
    else {
        return out;
    };

    let obj_hint = rl_obj
        .get("plan_type")
        .or_else(|| rl_obj.get("tier"))
        .or_else(|| rl_obj.get("plan"))
        .and_then(|v| v.as_str())
        .and_then(normalize_explicit_antigravity_tier)
        .or(explicit_hint);

    for (window_name, win_val) in rl_obj {
        let Some(win_obj) = win_val.as_object() else {
            continue;
        };
        let used_percent = win_obj
            .get("used_percent")
            .or_else(|| win_obj.get("usedPercent"))
            .and_then(|v| v.as_f64())
            .or_else(|| {
                let limit = win_obj.get("limit")?.as_f64()?;
                if limit <= 0.0 {
                    return None;
                }
                if let Some(used) = win_obj.get("used").and_then(|v| v.as_f64()) {
                    Some(((used / limit) * 100.0).clamp(0.0, 100.0))
                } else {
                    let rem = win_obj.get("remaining")?.as_f64()?;
                    Some((((limit - rem).max(0.0) / limit) * 100.0).clamp(0.0, 100.0))
                }
            });
        let Some(pct) = used_percent.filter(|p| p.is_finite()) else {
            continue;
        };
        let window_minutes = win_obj.get("window_minutes").and_then(|v| v.as_i64());
        let resets_at_ms = parse_timestamp_ms(
            win_obj
                .get("resets_at")
                .or_else(|| win_obj.get("reset_at"))
                .or_else(|| win_obj.get("resetTime")),
        )
        .or_else(|| {
            win_obj
                .get("resets_in_seconds")
                .and_then(|v| v.as_i64())
                .map(|secs| observed_at_ms + secs.max(0) * 1000)
        });
        out.push(ObservedRateLimitWindow {
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            window_name: window_name.clone(),
            used_percent: pct.clamp(0.0, 100.0),
            window_minutes,
            resets_at_ms,
            observed_at_ms,
            source: Some("local_log".to_string()),
            plan_hint: obj_hint.clone(),
        });
    }

    out
}

fn extract_antigravity_user_request(content: &str) -> Option<&str> {
    let extracted = if let Some(s) = content.find("<USER_REQUEST>") {
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
    let trimmed = extracted.trim();
    if trimmed.is_empty() || trimmed.to_ascii_lowercase().starts_with("[plan review") {
        None
    } else {
        Some(trimmed)
    }
}

#[derive(Default)]
struct InitialAntigravityMetadata {
    active_model: Option<String>,
    initial_title: Option<(String, String, i64)>,
    model_tier_hint: Option<String>,
}

/// Scan an existing `transcript.jsonl` read-only from byte 0 so incremental tailing from any
/// offset knows the active model, initial first-message title, and Ultra/Carbon tier signal.
fn scan_initial_antigravity_metadata(path: &Path) -> InitialAntigravityMetadata {
    let Ok(file) = open_shared_readonly(path) else {
        return InitialAntigravityMetadata::default();
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut meta = InitialAntigravityMetadata::default();

    while reader.read_line(&mut line).ok().is_some_and(|n| n > 0) {
        if let Ok(val) = serde_json::from_str::<Value>(line.trim()) {
            let ts_ms = parse_timestamp_ms(val.get("created_at").or_else(|| val.get("timestamp")))
                .unwrap_or(0);
            let source = val.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let step_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

            if let Some(title) = val
                .get("title")
                .or_else(|| val.get("conversation_title"))
                .and_then(|v| v.as_str())
                .and_then(sanitize_session_title)
            {
                meta.initial_title = Some((title, "explicit".to_string(), ts_ms));
            } else if meta.initial_title.is_none()
                && !source.eq_ignore_ascii_case("SYSTEM")
                && (step_type.eq_ignore_ascii_case("USER_INPUT")
                    || source.eq_ignore_ascii_case("USER_EXPLICIT"))
            {
                if let Some(user_req) = val
                    .get("content")
                    .and_then(|v| v.as_str())
                    .and_then(extract_antigravity_user_request)
                    .and_then(sanitize_session_title)
                {
                    meta.initial_title = Some((user_req, "user_prompt".to_string(), ts_ms));
                }
            }

            if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                if content.contains(SETTINGS_CHANGE_OPEN) && content.contains("Model Selection") {
                    if let Some(model) = extract_model_selection_from_content(content) {
                        if let Some(h) = infer_antigravity_tier_from_model_name(&model) {
                            meta.model_tier_hint = Some(h);
                        }
                        meta.active_model = Some(model);
                    }
                }
            }

            if meta.model_tier_hint.is_none() {
                if let Some(explicit_m) = val
                    .get("model")
                    .or_else(|| val.get("model_id"))
                    .and_then(|v| v.as_str())
                {
                    meta.model_tier_hint = infer_antigravity_tier_from_model_name(explicit_m);
                }
            }
        }
        line.clear();
    }

    meta
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

        let initial_meta = scan_initial_antigravity_metadata(path);
        let initial_model = initial_meta
            .active_model
            .unwrap_or_else(|| DEFAULT_ANTIGRAVITY_MODEL.to_string());
        let detected_plan_hint =
            resolve_antigravity_plan_hint(path).or(initial_meta.model_tier_hint);
        let has_initial_title = initial_meta.initial_title.is_some();

        FileParseContext {
            file_path: path.to_string_lossy().to_string(),
            device_id: device_id.to_string(),
            session_id,
            project_label: "antigravity".to_string(),
            current_model: Some(initial_model),
            detected_plan_hint,
            session_title: initial_meta.initial_title,
            session_title_dirty: has_initial_title,
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

        if let Some(explicit_tier) = val
            .get("plan_tier")
            .or_else(|| val.get("subscription_tier"))
            .or_else(|| val.get("tier"))
            .and_then(|v| v.as_str())
            .and_then(normalize_explicit_antigravity_tier)
        {
            if ctx.detected_plan_hint.as_deref() != Some(explicit_tier.as_str()) {
                ctx.detected_plan_hint = Some(explicit_tier);
                ctx.plan_hint_emitted = false;
            }
        }

        let content_opt = val.get("content").and_then(|v| v.as_str());
        if let Some(content) = content_opt {
            if let Some(selected_model) = extract_model_selection_from_content(content) {
                if let Some(model_hint) = infer_antigravity_tier_from_model_name(&selected_model) {
                    if ctx.detected_plan_hint.is_none() {
                        ctx.detected_plan_hint = Some(model_hint);
                        ctx.plan_hint_emitted = false;
                    }
                }
                ctx.current_model = Some(selected_model);
            }
        }

        let mut rate_limits =
            extract_antigravity_rate_limits(&val, ts_ms, ctx.detected_plan_hint.as_deref());
        if !rate_limits.is_empty() {
            ctx.plan_hint_emitted = true;
        }

        // When a `GENERIC` tool execution step immediately follows a `PLANNER_RESPONSE` that had
        // `tool_calls`, its `created_at` marks the exact completion of the model's generation step.
        if step_type.eq_ignore_ascii_case("GENERIC") {
            if let Some(mut pending) = ctx.pending_antigravity_event.take() {
                let decode_dur = ts_ms - pending.timestamp_ms;
                if decode_dur > 0 && decode_dur <= 600_000 {
                    pending.duration_ms = Some(decode_dur);
                    let eff_out = pending.output_tokens + pending.reasoning_tokens;
                    pending.output_tps = compute_tps(eff_out, Some(decode_dur));
                    return Some(ParsedLineResult {
                        event: Some(pending),
                        rate_limits,
                        balance: None,
                    });
                }
            }
        }

        let is_user_step = !source.eq_ignore_ascii_case("SYSTEM")
            && (step_type.eq_ignore_ascii_case("USER_INPUT")
                || source.eq_ignore_ascii_case("USER_EXPLICIT"));
        if is_user_step {
            ctx.pending_antigravity_event = None;
            ctx.last_prompt_ts_ms = Some(ts_ms);
            if let Some(user_req) = content_opt.and_then(extract_antigravity_user_request) {
                ctx.record_session_title(user_req, "user_prompt", ts_ms);
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
            return if rate_limits.is_empty() {
                None
            } else {
                Some(ParsedLineResult {
                    event: None,
                    rate_limits,
                    balance: None,
                })
            };
        }

        ctx.pending_antigravity_event = None;

        // `CHECKPOINT` / `SYSTEM` compaction steps record cumulative context compaction tokens,
        // not live streaming generation; never compute generation duration or TPS on them.
        let is_compaction_or_system = step_type.eq_ignore_ascii_case("CHECKPOINT")
            || source.eq_ignore_ascii_case("SYSTEM");
        let has_tool_calls = val
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());

        let explicit_dur = val
            .get("duration_ms")
            .and_then(|v| v.as_i64())
            .filter(|&d| d > 0);
        let explicit_ttft = val.get("ttft_ms").and_then(|v| v.as_i64());

        let effective_output = output_tokens + reasoning_tokens;
        let prompt_delta = if is_compaction_or_system {
            None
        } else {
            ctx.last_prompt_ts_ms.take().and_then(|prompt_ts| {
                let delta = ts_ms - prompt_ts;
                if delta > 0 && delta <= 600_000 {
                    Some((prompt_ts, delta))
                } else {
                    None
                }
            })
        };

        let (duration_ms, ttft_ms) = if is_compaction_or_system {
            (None, None)
        } else if explicit_dur.is_some() {
            (explicit_dur, explicit_ttft)
        } else if has_tool_calls {
            // `created_at` on `PLANNER_RESPONSE` with `tool_calls` is the step start timestamp;
            // the following `GENERIC` step's `created_at` will supply the true generation duration.
            let ttft = explicit_ttft
                .or_else(|| prompt_delta.and_then(|(_, d)| (d <= 60_000).then_some(d)));
            (None, ttft)
        } else {
            let dur = prompt_delta.and_then(|(prompt_ts, delta)| {
                let both_second_quantized = ts_ms % 1_000 == 0 && prompt_ts % 1_000 == 0;
                if both_second_quantized && delta <= 2_000 && effective_output >= 80 {
                    None
                } else {
                    Some(delta)
                }
            });
            (dur, explicit_ttft)
        };

        let observable_only = total_tokens == 0;
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

        if !ctx.plan_hint_emitted {
            if let Some(hint) = ctx
                .detected_plan_hint
                .clone()
                .or_else(|| infer_antigravity_tier_from_model_name(&model_id))
            {
                ctx.plan_hint_emitted = true;
                rate_limits.push(ObservedRateLimitWindow {
                    provider_id: provider_ids::ANTIGRAVITY.to_string(),
                    window_name: "tier_hint".to_string(),
                    used_percent: 0.0,
                    window_minutes: None,
                    resets_at_ms: None,
                    observed_at_ms: ts_ms,
                    source: Some("local_log_hint".to_string()),
                    plan_hint: Some(hint),
                });
            }
        }

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

        let event = NormalizedUsageEvent {
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
            ttft_ms,
            output_tps,
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        };

        if !is_compaction_or_system && has_tool_calls && duration_ms.is_none() && !observable_only {
            ctx.pending_antigravity_event = Some(event.clone());
        }

        Some(ParsedLineResult {
            event: Some(event),
            rate_limits,
            balance: None,
        })
    }

    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        collapse_duplicate_events_keep_last(events)
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

    #[test]
    fn planner_response_with_tool_calls_uses_generic_step_completion_and_ignores_checkpoint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings_path = dir.path().join("settings.json");
        std::fs::write(
            &settings_path,
            r#"{"useG1Credits":true,"subscriptionTier":"Google AI Ultra 5x"}"#,
        )
        .expect("write settings");

        let transcript = dir
            .path()
            .join("brain")
            .join("conv-99")
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        std::fs::create_dir_all(transcript.parent().expect("parent")).expect("mkdir");
        std::fs::write(&transcript, "").expect("touch transcript");

        let parser = AntigravityParser::new();
        let mut ctx = parser.init_context(&transcript, "dev-1");
        assert_eq!(ctx.detected_plan_hint.as_deref(), Some("ultra_5x"));

        // 1. User prompt at 01:13:07Z, PLANNER_RESPONSE (with tool_calls, 400 tokens) also starts at 01:13:07Z,
        //    and GENERIC tool step starts at 01:13:23Z (16s true decode duration -> 25.0 tok/s).
        let user_step = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:13:07Z","content":"<USER_REQUEST>Audit speed</USER_REQUEST>"}"#;
        let planner_step = r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-16T01:13:07Z","input_tokens":19000,"output_tokens":400,"tool_calls":[{"name":"read_file"}]}"#;
        let generic_step = r#"{"step_index":2,"source":"MODEL","type":"GENERIC","status":"DONE","created_at":"2026-11-16T01:13:23Z","content":"Created At: 2026-11-16T09:13:23+08:00"}"#;
        let checkpoint_step = r#"{"step_index":3,"source":"SYSTEM","type":"CHECKPOINT","status":"DONE","created_at":"2026-11-16T01:13:24Z","input_tokens":490000,"output_tokens":2800,"content":"Compaction summary"}"#;

        assert!(parser.parse_line(user_step, &mut ctx).is_none());
        let p_res = parser.parse_line(planner_step, &mut ctx).expect("planner");
        assert_eq!(p_res.rate_limits.len(), 1);
        assert_eq!(p_res.rate_limits[0].plan_hint.as_deref(), Some("ultra_5x"));
        let p_ev = p_res.event.expect("planner event");
        assert_eq!(p_ev.duration_ms, None);
        assert_eq!(p_ev.output_tps, None);

        let g_ev = parser
            .parse_line(generic_step, &mut ctx)
            .and_then(|r| r.event)
            .expect("upgraded planner event on generic step");
        assert_eq!(g_ev.event_id, p_ev.event_id);
        assert_eq!(g_ev.duration_ms, Some(16_000));
        assert!((g_ev.output_tps.expect("valid tps") - 25.0).abs() < 1e-6);

        let cp_ev = parser
            .parse_line(checkpoint_step, &mut ctx)
            .and_then(|r| r.event)
            .expect("checkpoint event");
        assert_eq!(cp_ev.duration_ms, None);
        assert_eq!(cp_ev.output_tps, None);

        let finalized = parser.finalize_batch(vec![p_ev, g_ev, cp_ev]);
        assert_eq!(finalized.len(), 2);
        assert_eq!(finalized[0].duration_ms, Some(16_000));
        assert!((finalized[0].output_tps.expect("final tps") - 25.0).abs() < 1e-6);
        assert_eq!(finalized[1].output_tps, None);
    }

    #[test]
    fn locks_first_user_prompt_title_and_avoids_false_pro_from_g1_or_gemini_pro() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings_path = dir.path().join("settings.json");
        std::fs::write(&settings_path, r#"{"useG1Credits":true}"#).expect("write settings");
        assert_eq!(parse_settings_json_for_plan_hint(&settings_path), None);

        std::fs::write(
            &settings_path,
            r#"{"agent":{"preferredModelTier":"g1-ultra-tier"}}"#,
        )
        .expect("write ultra settings");
        assert_eq!(
            parse_settings_json_for_plan_hint(&settings_path).as_deref(),
            Some("ultra")
        );

        assert_eq!(infer_antigravity_tier_from_model_name("Gemini 3.1 Pro (High)"), None);
        assert_eq!(
            infer_antigravity_tier_from_model_name("Orchestrator (Carbon)").as_deref(),
            Some("ultra")
        );

        let transcript = dir
            .path()
            .join("brain")
            .join("conv-first-msg")
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        std::fs::create_dir_all(transcript.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &transcript,
            concat!(
                r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:00:00Z","content":"<USER_REQUEST>Build initial dark glass layout</USER_REQUEST>\n<USER_SETTINGS_CHANGE>Model Selection from None to Gemini 3.1 Pro (High).</USER_SETTINGS_CHANGE>"}"#,
                "\n",
                r#"{"step_index":1,"source":"SYSTEM","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:01:00Z","content":"[plan review] internal handoff"}"#,
                "\n",
                r#"{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:05:00Z","content":"<USER_REQUEST>Continue step 3</USER_REQUEST>"}"#,
                "\n",
            ),
        )
        .expect("write transcript");

        let parser = AntigravityParser::new();
        let mut ctx = parser.init_context(&transcript, "dev-1");
        assert_eq!(
            ctx.session_title.as_ref().map(|t| t.0.as_str()),
            Some("Build initial dark glass layout")
        );

        // Even feeding later turn or mid-file warmup keeps first sentence locked
        ctx.lock_non_explicit_title = true;
        assert!(parser
            .parse_line(
                r#"{"step_index":3,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T02:00:00Z","content":"<USER_REQUEST>Latest message must not win</USER_REQUEST>"}"#,
                &mut ctx,
            )
            .is_none());
        assert_eq!(
            ctx.session_title.as_ref().map(|t| t.0.as_str()),
            Some("Build initial dark glass layout")
        );
    }
}
