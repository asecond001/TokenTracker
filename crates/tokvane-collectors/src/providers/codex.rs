use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;
use tokvane_core::domain::{
    compute_event_id, provider_ids, sanitize_project_label, sanitize_session_title,
    NormalizedUsageEvent, ObservedRateLimitWindow, ProviderBalanceSnapshot, SessionTitleRecord,
};

use crate::providers::{
    compute_tps, extract_text_from_content, get_i64, now_ms, parse_timestamp_ms, user_home_dir,
    CodexRawTotals, FileParseContext, ParsedLineResult, ProviderLogParser,
};
use crate::watcher::open_shared_readonly;

const INITIAL_CODEX_SCAN_MAX_LINES: usize = 256;
const INITIAL_CODEX_SCAN_MAX_BYTES: usize = 512 * 1024;

#[derive(Default)]
struct InitialCodexMetadata {
    session_id: Option<String>,
    project_label: Option<String>,
    model: Option<String>,
    initial_title: Option<(String, String, i64)>,
    is_background: bool,
    background_ts_ms: i64,
}

fn extract_codex_model(payload: &Value) -> Option<String> {
    payload
        .get("model")
        .or_else(|| payload.pointer("/collaboration_mode/settings/model"))
        .or_else(|| payload.pointer("/info/model"))
        .or_else(|| payload.pointer("/settings/model"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn is_background_title_str(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "guardian review" | "codex-auto-review" | "auto-review" | "auto review"
    )
}

fn is_background_model_name(model: &str) -> bool {
    let lower = model.trim().to_ascii_lowercase();
    lower.contains("auto-review") || lower.contains("auto_review")
}

fn is_codex_background_meta(payload: &Value) -> bool {
    if let Some(src) = payload.get("source") {
        match src {
            Value::String(s) => {
                let lower = s.to_ascii_lowercase();
                if lower.contains("subagent")
                    || lower.contains("guardian")
                    || lower.contains("auto_review")
                    || lower.contains("auto-review")
                {
                    return true;
                }
            }
            Value::Object(map)
                if map.contains_key("subagent")
                    || map.contains_key("guardian")
                    || map.contains_key("auto_review") =>
            {
                return true;
            }
            _ => {}
        }
    }
    if let Some(role) = payload
        .get("agent_role")
        .or_else(|| payload.get("role"))
        .and_then(|v| v.as_str())
    {
        let lower = role.to_ascii_lowercase();
        if lower.contains("guardian") || lower.contains("auto_review") || lower.contains("auto-review")
        {
            return true;
        }
    }
    false
}

fn scan_initial_codex_metadata(path: &Path) -> InitialCodexMetadata {
    let Ok(file) = open_shared_readonly(path) else {
        return InitialCodexMetadata::default();
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut meta = InitialCodexMetadata::default();
    let mut bytes_read = 0usize;

    for _ in 0..INITIAL_CODEX_SCAN_MAX_LINES {
        line.clear();
        let Ok(n) = reader.read_line(&mut line) else {
            break;
        };
        if n == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(n);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if bytes_read >= INITIAL_CODEX_SCAN_MAX_BYTES {
                break;
            }
            continue;
        }
        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
            let ts_ms = parse_timestamp_ms(val.get("timestamp")).unwrap_or(0);
            let top_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(payload) = val.get("payload") {
                if let Some(m) = extract_codex_model(payload) {
                    if is_background_model_name(&m) {
                        meta.is_background = true;
                        meta.background_ts_ms = meta.background_ts_ms.max(ts_ms);
                    }
                    if meta.model.is_none() || top_type == "turn_context" {
                        meta.model = Some(m);
                    }
                }
                match top_type {
                    "session_meta" => {
                        if let Some(id) = payload.get("id").and_then(|v| v.as_str()) {
                            let clean = id.trim();
                            if !clean.is_empty() {
                                meta.session_id = Some(clean.to_string());
                            }
                        }
                        if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                            meta.project_label = Some(sanitize_project_label(Some(cwd)));
                        }
                        if is_codex_background_meta(payload) {
                            meta.is_background = true;
                            meta.background_ts_ms = meta.background_ts_ms.max(ts_ms);
                        }
                        if let Some(title) = payload
                            .get("title")
                            .or_else(|| payload.get("name"))
                            .and_then(|v| v.as_str())
                        {
                            if is_background_title_str(title) {
                                meta.is_background = true;
                                meta.background_ts_ms = meta.background_ts_ms.max(ts_ms);
                            } else if let Some(clean) = sanitize_session_title(title) {
                                meta.initial_title = Some((clean, "explicit".to_string(), ts_ms));
                            }
                        }
                    }
                    "turn_context" => {
                        if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                            if meta.project_label.is_none() {
                                meta.project_label = Some(sanitize_project_label(Some(cwd)));
                            }
                        }
                    }
                    "response_item" => {
                        if payload.get("role").and_then(|v| v.as_str()) == Some("user") {
                            if let Some(text) = extract_codex_user_prompt(payload.get("content")) {
                                if text
                                    .to_ascii_lowercase()
                                    .starts_with("you are judging one planned coding-agent action")
                                {
                                    meta.is_background = true;
                                    meta.background_ts_ms = meta.background_ts_ms.max(ts_ms);
                                } else if meta.initial_title.is_none() {
                                    meta.initial_title = Some((text, "derived".to_string(), ts_ms));
                                }
                            }
                        }
                    }
                    "event_msg"
                        if payload.get("type").and_then(|v| v.as_str()) == Some("user_message") =>
                    {
                        if let Some(msg) = extract_text_from_content(
                            payload
                                .get("message")
                                .or_else(|| payload.get("text"))
                                .or_else(|| payload.get("content")),
                        ) {
                            if msg
                                .to_ascii_lowercase()
                                .starts_with("you are judging one planned coding-agent action")
                            {
                                meta.is_background = true;
                                meta.background_ts_ms = meta.background_ts_ms.max(ts_ms);
                            } else if !is_codex_injected_preamble(&msg) {
                                if let Some(clean) = sanitize_session_title(&msg) {
                                    let replace = match &meta.initial_title {
                                        None => true,
                                        Some((_, src, prev_ts)) => {
                                            src == "derived" || ts_ms < *prev_ts
                                        }
                                    };
                                    if replace {
                                        meta.initial_title =
                                            Some((clean, "user_prompt".to_string(), ts_ms));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if meta.session_id.is_some()
            && meta.model.is_some()
            && (meta.initial_title.is_some() || meta.is_background)
        {
            break;
        }
        if bytes_read >= INITIAL_CODEX_SCAN_MAX_BYTES {
            break;
        }
    }

    meta
}

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
        let fallback_sid = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("codex-session")
            .to_string();
        let meta = scan_initial_codex_metadata(path);

        let mut ctx = FileParseContext {
            file_path: path.to_string_lossy().to_string(),
            device_id: device_id.to_string(),
            session_id: meta.session_id.unwrap_or(fallback_sid),
            project_label: meta.project_label.unwrap_or_else(|| "workspace".to_string()),
            current_model: meta.model,
            session_title: meta.initial_title,
            ..Default::default()
        };
        if meta.is_background {
            ctx.mark_background_session("Guardian review", meta.background_ts_ms);
        }
        if ctx.session_title.is_some() {
            ctx.session_title_dirty = true;
        }
        ctx
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
                if let Some(model) = extract_codex_model(payload) {
                    if is_background_model_name(&model) {
                        ctx.mark_background_session("Guardian review", ts_ms);
                    }
                    ctx.current_model = Some(model);
                }
                if is_codex_background_meta(payload) {
                    ctx.mark_background_session("Guardian review", ts_ms);
                }
                if let Some(title) = payload
                    .get("title")
                    .or_else(|| payload.get("name"))
                    .and_then(|v| v.as_str())
                {
                    if is_background_title_str(title) {
                        ctx.mark_background_session(title, ts_ms);
                    } else {
                        ctx.record_session_title(title, "explicit", ts_ms);
                    }
                }
                None
            }
            "turn_context" => {
                if let Some(model) = extract_codex_model(payload) {
                    if is_background_model_name(&model) {
                        ctx.mark_background_session("Guardian review", ts_ms);
                    }
                    ctx.current_model = Some(model);
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
                if let Some(model) = extract_codex_model(payload) {
                    if is_background_model_name(&model) {
                        ctx.mark_background_session("Guardian review", ts_ms);
                    }
                    ctx.current_model = Some(model);
                }
                let role = payload.get("role").and_then(|v| v.as_str()).unwrap_or("");
                if role == "user" {
                    ctx.last_prompt_ts_ms = Some(ts_ms);
                    if let Some(text) = extract_codex_user_prompt(payload.get("content")) {
                        if text
                            .to_ascii_lowercase()
                            .starts_with("you are judging one planned coding-agent action")
                        {
                            ctx.mark_background_session("Guardian review", ts_ms);
                        } else {
                            ctx.record_session_title(&text, "derived", ts_ms);
                        }
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
                        if msg
                            .to_ascii_lowercase()
                            .starts_with("you are judging one planned coding-agent action")
                        {
                            ctx.mark_background_session("Guardian review", ts_ms);
                        } else if !is_codex_injected_preamble(&msg) {
                            ctx.record_session_title(&msg, "user_prompt", ts_ms);
                        }
                    }
                    return None;
                }
                if ev_type != "token_count" {
                    return None;
                }

                let rate_limits = extract_rate_limits(payload, ts_ms);
                let balance = extract_codex_credits(payload, ts_ms);

                let Some(info) = payload.get("info").filter(|i| !i.is_null()) else {
                    return if rate_limits.is_empty() && balance.is_none() {
                        None
                    } else {
                        Some(ParsedLineResult {
                            event: None,
                            rate_limits,
                            balance,
                        })
                    };
                };

                if let Some(model) = extract_codex_model(payload).or_else(|| extract_codex_model(info))
                {
                    if is_background_model_name(&model) {
                        ctx.mark_background_session("Guardian review", ts_ms);
                    }
                    ctx.current_model = Some(model);
                }

                let raw_totals = info.get("total_token_usage").map(read_codex_totals);
                if let (Some(curr_tot), Some(prev_tot)) = (raw_totals, ctx.last_codex_totals) {
                    if curr_tot == prev_tot {
                        // Identical repeated heartbeat snapshot; still pass rate_limits/credits if present.
                        return if rate_limits.is_empty() && balance.is_none() {
                            None
                        } else {
                            Some(ParsedLineResult {
                                event: None,
                                rate_limits,
                                balance,
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
                    return if rate_limits.is_empty() && balance.is_none() {
                        None
                    } else {
                        Some(ParsedLineResult {
                            event: None,
                            rate_limits,
                            balance,
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
                    .unwrap_or_else(|| "codex-unknown".to_string());
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
                    balance,
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
        || lower.starts_with("the following is the codex agent history")
        || lower.starts_with("the following is a summary of")
        || lower.starts_with("another language model started to solve this problem")
        || lower.starts_with("[plan review")
        || lower.starts_with("<context_summary>")
        || lower.starts_with("<compacted_context>")
}

fn extract_codex_user_prompt(content: Option<&Value>) -> Option<String> {
    let val = content?;
    match val {
        Value::String(s) if !is_codex_injected_preamble(s) => sanitize_session_title(s),
        Value::Array(items) => {
            for item in items {
                if let Some(s) = item
                    .get("text")
                    .or_else(|| item.get("input_text"))
                    .and_then(|v| v.as_str())
                {
                    if !is_codex_injected_preamble(s) {
                        if let Some(clean) = sanitize_session_title(s) {
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
    let Some(rl) = payload
        .get("rate_limits")
        .or_else(|| payload.get("info").and_then(|i| i.get("rate_limits")))
        .and_then(|v| v.as_object())
    else {
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

fn parse_credit_amount_micro(val: &Value) -> Option<(i64, bool)> {
    if let Some(f) = val.as_f64().filter(|n| n.is_finite() && *n >= 0.0) {
        return Some(((f * 1_000_000.0).round() as i64, false));
    }
    let raw = val.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    let is_usd = raw.starts_with('$');
    let cleaned: String = raw
        .trim_start_matches('$')
        .chars()
        .filter(|c| *c != ',' && !c.is_whitespace())
        .collect();
    let parsed: f64 = cleaned.parse().ok()?;
    if !parsed.is_finite() || parsed < 0.0 {
        return None;
    }
    Some(((parsed * 1_000_000.0).round() as i64, is_usd))
}

fn extract_codex_credits(payload: &Value, observed_at_ms: i64) -> Option<ProviderBalanceSnapshot> {
    let rl = payload
        .get("rate_limits")
        .or_else(|| payload.get("info").and_then(|i| i.get("rate_limits")))?;
    let credits = rl.get("credits").and_then(|v| v.as_object())?;
    let unlimited = credits
        .get("unlimited")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_credits = credits
        .get("has_credits")
        .or_else(|| credits.get("hasCredits"))
        .and_then(|v| v.as_bool());

    if unlimited {
        return Some(ProviderBalanceSnapshot {
            provider_id: provider_ids::CODEX.to_string(),
            currency: "CREDITS_UNLIMITED".to_string(),
            total_balance_micro: Some(0),
            granted_balance_micro: None,
            topped_up_balance_micro: None,
            daily_quota_used_micro: None,
            daily_quota_limit_micro: None,
            fetched_at_ms: observed_at_ms,
        });
    }

    if let Some((micro, is_usd)) = credits.get("balance").and_then(parse_credit_amount_micro) {
        if has_credits == Some(false) && micro == 0 {
            return None;
        }
        let currency = if is_usd { "USD" } else { "CREDITS" };
        return Some(ProviderBalanceSnapshot {
            provider_id: provider_ids::CODEX.to_string(),
            currency: currency.to_string(),
            total_balance_micro: Some(micro),
            granted_balance_micro: None,
            topped_up_balance_micro: None,
            daily_quota_used_micro: None,
            daily_quota_limit_micro: None,
            fetched_at_ms: observed_at_ms,
        });
    }
    None
}

/// Read-only loader for Codex's official `~/.codex/session_index.jsonl` explicit thread names.
pub fn load_codex_session_index_titles(roots: &[PathBuf]) -> Vec<SessionTitleRecord> {
    load_codex_session_index_titles_from_roots(roots, true)
}

fn load_codex_session_index_titles_from_roots(
    roots: &[PathBuf],
    include_default_home: bool,
) -> Vec<SessionTitleRecord> {
    let mut index_paths = BTreeSet::new();
    for root in roots {
        let direct = root.join("session_index.jsonl");
        if direct.is_file() {
            index_paths.insert(direct);
        }
        if let Some(parent) = root.parent() {
            let parent_idx = parent.join("session_index.jsonl");
            if parent_idx.is_file() {
                index_paths.insert(parent_idx);
            }
        }
    }
    if include_default_home {
        if let Some(home) = user_home_dir() {
            let default_idx = home.join(".codex").join("session_index.jsonl");
            if default_idx.is_file() {
                index_paths.insert(default_idx);
            }
        }
    }

    let mut latest_by_session: HashMap<String, SessionTitleRecord> = HashMap::new();
    for path in index_paths {
        let Ok(file) = open_shared_readonly(&path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        while reader.read_line(&mut line).ok().is_some_and(|n| n > 0) {
            if let Ok(val) = serde_json::from_str::<Value>(line.trim()) {
                let sid = val
                    .get("id")
                    .or_else(|| val.get("session_id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let raw_name = val
                    .get("thread_name")
                    .or_else(|| val.get("title"))
                    .and_then(|v| v.as_str());
                if let (Some(sid), Some(raw_name)) = (sid, raw_name) {
                    if let Some(clean_title) = sanitize_session_title(raw_name) {
                        let ts_ms = parse_timestamp_ms(val.get("updated_at")).unwrap_or(0);
                        let key = format!("{}:{}", provider_ids::CODEX, sid);
                        let should_insert = latest_by_session
                            .get(&key)
                            .is_none_or(|prev| ts_ms >= prev.updated_at_ms);
                        let source = if is_background_title_str(&clean_title) {
                            "background"
                        } else {
                            "explicit"
                        };
                        if should_insert {
                            latest_by_session.insert(
                                key.clone(),
                                SessionTitleRecord {
                                    session_key: key,
                                    provider_id: provider_ids::CODEX.to_string(),
                                    title: clean_title,
                                    source: source.to_string(),
                                    updated_at_ms: ts_ms,
                                },
                            );
                        }
                    }
                }
            }
            line.clear();
        }
    }

    latest_by_session.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn turn_context_does_not_overwrite_user_prompt_ts_and_extracts_title() {
        let parser = CodexParser::new();
        let mut ctx = parser.init_context(Path::new("C:/tmp/.codex/sessions/rollout-2026.jsonl"), "dev-1");

        let meta = r#"{"timestamp":"2026-11-16T01:00:00.000Z","type":"session_meta","payload":{"id":"codex-sess-1","cwd":"C:/Projects/TokenVane"}}"#;
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

    #[test]
    fn codex_ignores_compact_preamble_extracts_credits_and_reads_session_index() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("mkdir");
        let idx_path = dir.path().join("session_index.jsonl");
        std::fs::write(
            &idx_path,
            concat!(
                r#"{"id":"codex-s1","thread_name":"Old Title","updated_at":"2026-11-16T01:00:00Z"}"#,
                "\n",
                r#"{"id":"codex-s1","thread_name":"Renamed PayG Feature Thread","updated_at":"2026-11-16T02:00:00Z"}"#,
                "\n"
            ),
        )
        .expect("write session index");

        let index_titles =
            load_codex_session_index_titles_from_roots(std::slice::from_ref(&sessions_dir), false);
        assert_eq!(index_titles.len(), 1);
        assert_eq!(index_titles[0].session_key, "codex:codex-s1");
        assert_eq!(index_titles[0].title, "Renamed PayG Feature Thread");
        assert_eq!(index_titles[0].source, "explicit");

        let parser = CodexParser::new();
        let mut ctx = parser.init_context(&sessions_dir.join("rollout-1.jsonl"), "dev-1");
        let compact_msg = r#"{"timestamp":"2026-11-16T03:00:00.000Z","type":"response_item","payload":{"role":"user","content":[{"type":"input_text","text":"The following is the Codex agent history. Some earlier turns were compacted..."}]}}"#;
        let first_real_msg = r#"{"timestamp":"2026-11-16T00:10:00.000Z","type":"event_msg","payload":{"type":"user_message","message":"Fix Codex credits extraction"}}"#;
        let second_msg = r#"{"timestamp":"2026-11-16T04:00:00.000Z","type":"event_msg","payload":{"type":"user_message","message":"Thanks, now run tests"}}"#;

        assert!(parser.parse_line(compact_msg, &mut ctx).is_none());
        assert_eq!(ctx.session_title, None);

        assert!(parser.parse_line(first_real_msg, &mut ctx).is_none());
        assert!(parser.parse_line(second_msg, &mut ctx).is_none());
        assert_eq!(
            ctx.session_title.as_ref().map(|t| t.0.as_str()),
            Some("Fix Codex credits extraction")
        );

        let tok_with_credits = r#"{"timestamp":"2026-11-16T04:00:05.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"plan_type":"plus","credits":{"has_credits":true,"unlimited":false,"balance":"766.76"}},"info":null}}"#;
        let parsed = parser
            .parse_line(tok_with_credits, &mut ctx)
            .expect("credits parsed");
        let bal = parsed.balance.expect("balance snapshot");
        assert_eq!(bal.currency, "CREDITS");
        assert_eq!(bal.total_balance_micro, Some(766_760_000));
    }

    #[test]
    fn codex_never_fabricates_gpt5_codex_and_marks_guardian_background_sessions() {
        let parser = CodexParser::new();
        let mut ctx_no_model = parser.init_context(Path::new("C:/tmp/nonexistent/rollout-no-model.jsonl"), "dev-1");
        let tok_no_model = r#"{"timestamp":"2026-11-16T04:10:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":0}}}}"#;
        let ev_unknown = parser
            .parse_line(tok_no_model, &mut ctx_no_model)
            .and_then(|r| r.event)
            .expect("event without explicit model");
        assert_eq!(ev_unknown.model_id, "codex-unknown");

        // Extracts nested collaboration_mode.settings.model and subagent/guardian background metadata
        let mut ctx_collab = parser.init_context(Path::new("C:/tmp/nonexistent/rollout-collab.jsonl"), "dev-1");
        let turn_nested = r#"{"timestamp":"2026-11-16T04:11:00.000Z","type":"turn_context","payload":{"collaboration_mode":{"mode":"default","settings":{"model":"gpt-5.4","reasoning_effort":"high"}}}}"#;
        assert!(parser.parse_line(turn_nested, &mut ctx_collab).is_none());
        let tok_nested = r#"{"timestamp":"2026-11-16T04:11:05.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":0}}}}"#;
        let ev_54 = parser
            .parse_line(tok_nested, &mut ctx_collab)
            .and_then(|r| r.event)
            .expect("nested model event");
        assert_eq!(ev_54.model_id, "gpt-5.4");

        let mut ctx_bg = parser.init_context(Path::new("C:/tmp/nonexistent/rollout-guardian.jsonl"), "dev-1");
        let bg_meta = r#"{"timestamp":"2026-11-16T04:12:00.000Z","type":"session_meta","payload":{"id":"codex-guardian-1","source":{"subagent":"guardian"},"cwd":"C:/Projects/TokenVane"}}"#;
        let bg_turn = r#"{"timestamp":"2026-11-16T04:12:01.000Z","type":"turn_context","payload":{"model":"codex-auto-review"}}"#;
        assert!(parser.parse_line(bg_meta, &mut ctx_bg).is_none());
        assert!(parser.parse_line(bg_turn, &mut ctx_bg).is_none());
        assert!(ctx_bg.is_background_session);
        let title_rec = ctx_bg
            .take_session_title_record(provider_ids::CODEX)
            .expect("background title");
        assert_eq!(title_rec.source, "background");

        // session_index.jsonl Guardian review entries emit source = "background"
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("mkdir");
        std::fs::write(
            dir.path().join("session_index.jsonl"),
            r#"{"id":"codex-guardian-1","thread_name":"Guardian review","updated_at":"2026-11-16T04:12:05Z"}"#,
        )
        .expect("write idx");
        let idx =
            load_codex_session_index_titles_from_roots(std::slice::from_ref(&sessions_dir), false);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].source, "background");
    }
}
