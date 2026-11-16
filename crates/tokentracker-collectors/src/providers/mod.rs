pub mod antigravity;
pub mod claude_code;
pub mod codex;
pub mod compatible_common;
pub mod custom_compatible;
pub mod deepseek_harness;
pub mod kimi_code;
pub mod minimax_code;
pub mod official_api;
pub mod qcode;
pub mod workbuddy;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use tokentracker_core::domain::{
    sanitize_session_title, NormalizedUsageEvent, ObservedRateLimitWindow, SessionTitleRecord,
};

/// Active turn timestamps & baseline output counter retained per streaming `message.id`
/// so multi-chunk responses measure true turn duration instead of inter-chunk tail deltas.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActiveMessageTurn {
    pub turn_prompt_ts_ms: Option<i64>,
    pub first_chunk_ts_ms: i64,
    pub first_chunk_output_tokens: i64,
}

/// Per-file streaming parser context maintained across lines during a scan/tail pass.
#[derive(Debug, Default)]
pub struct FileParseContext {
    pub file_path: String,
    pub device_id: String,
    pub session_id: String,
    pub project_label: String,
    pub current_model: Option<String>,
    pub last_prompt_ts_ms: Option<i64>,
    pub last_step_ts_ms: Option<i64>,
    /// Tracks Codex cumulative `total_token_usage` to compute deltas and drop duplicate snapshots.
    pub last_codex_totals: Option<CodexRawTotals>,
    /// Bounded LRU-like ring of active streaming `message.id` turns for Claude Code logs.
    pub active_message_turns: Vec<(String, ActiveMessageTurn)>,
    /// Tracks cumulative context token checkpoint (e.g. Kimi CLI `_usage` records) to derive step deltas.
    pub last_context_tokens: Option<i64>,
    /// Sanitized session display title `(title, source, updated_at_ms)` discovered during scan.
    pub session_title: Option<(String, String, i64)>,
    pub session_title_dirty: bool,
    pub line_number: u64,
}

impl FileParseContext {
    pub const MAX_ACTIVE_MESSAGE_TURNS: usize = 32;

    /// Resolve or initialize turn timing metadata for a streaming `message.id`.
    pub fn resolve_message_turn(
        &mut self,
        message_id: &str,
        ts_ms: i64,
        effective_output_tokens: i64,
    ) -> ActiveMessageTurn {
        if let Some(pos) = self
            .active_message_turns
            .iter()
            .position(|(id, _)| id == message_id)
        {
            let mut turn = self.active_message_turns[pos].1;
            if ts_ms < turn.first_chunk_ts_ms {
                turn.first_chunk_ts_ms = ts_ms;
                turn.first_chunk_output_tokens = effective_output_tokens;
                self.active_message_turns[pos].1 = turn;
            }
            return turn;
        }
        let turn_prompt_ts_ms = self
            .last_prompt_ts_ms
            .take()
            .filter(|&p| p < ts_ms && ts_ms - p <= 600_000);
        let turn = ActiveMessageTurn {
            turn_prompt_ts_ms,
            first_chunk_ts_ms: ts_ms,
            first_chunk_output_tokens: effective_output_tokens,
        };
        if self.active_message_turns.len() >= Self::MAX_ACTIVE_MESSAGE_TURNS {
            self.active_message_turns.remove(0);
        }
        self.active_message_turns
            .push((message_id.to_string(), turn));
        turn
    }

    /// Record a candidate session title (explicit titles always override prompt-derived titles;
    /// derived titles prefer the earliest user prompt observed in the session).
    pub fn record_session_title(&mut self, raw_title: &str, source: &str, ts_ms: i64) -> bool {
        let Some(clean) = sanitize_session_title(raw_title) else {
            return false;
        };
        let norm_source = if source == "explicit" {
            "explicit"
        } else {
            "derived"
        };
        let should_replace = match &self.session_title {
            None => true,
            Some((_, prev_source, prev_ts)) => {
                norm_source == "explicit" || (prev_source != "explicit" && ts_ms < *prev_ts)
            }
        };
        if should_replace {
            self.session_title = Some((clean, norm_source.to_string(), ts_ms));
            self.session_title_dirty = true;
            true
        } else {
            false
        }
    }

    /// Build a [`SessionTitleRecord`] using the latest resolved `session_id` if dirty.
    pub fn take_session_title_record(&mut self, provider_id: &str) -> Option<SessionTitleRecord> {
        if !self.session_title_dirty || self.session_id.trim().is_empty() {
            return None;
        }
        self.session_title_dirty = false;
        let (title, source, updated_at_ms) = self.session_title.clone()?;
        Some(SessionTitleRecord {
            session_key: format!("{}:{}", provider_id, self.session_id),
            provider_id: provider_id.to_string(),
            title,
            source,
            updated_at_ms,
        })
    }
}

/// Extract readable plain text from OpenAI/Anthropic `content` fields (string, object, or block array).
pub(crate) fn extract_text_from_content(content: Option<&Value>) -> Option<String> {
    let val = content?;
    match val {
        Value::String(s) => Some(s.clone()),
        Value::Object(map) => map
            .get("text")
            .or_else(|| map.get("content"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        Value::Array(items) => {
            for item in items {
                if let Some(s) = item.as_str() {
                    if sanitize_session_title(s).is_some() {
                        return Some(s.to_string());
                    }
                }
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if matches!(item_type, "" | "text" | "input_text" | "output_text") {
                    if let Some(s) = item
                        .get("text")
                        .or_else(|| item.get("input_text"))
                        .and_then(|v| v.as_str())
                    {
                        if sanitize_session_title(s).is_some() {
                            return Some(s.to_string());
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CodexRawTotals {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
}

/// Output collected from parsing a batch of lines from a single file.
#[derive(Debug, Default)]
pub struct ParsedFileBatch {
    pub events: Vec<NormalizedUsageEvent>,
    pub rate_limits: Vec<ObservedRateLimitWindow>,
}

/// Compliant local log parser contract for built-in and declarative providers.
pub trait ProviderLogParser: Send + Sync {
    fn provider_id(&self) -> &str;
    fn default_roots(&self) -> Vec<PathBuf>;
    fn matches_file(&self, path: &Path) -> bool;
    fn init_context(&self, path: &Path, device_id: &str) -> FileParseContext;
    fn parse_line(&self, line: &str, ctx: &mut FileParseContext) -> Option<ParsedLineResult>;
    /// Optional post-batch deduplication (e.g. collapsing multiple streaming chunks for the same message ID).
    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        events
    }
}

#[derive(Debug, Default)]
pub struct ParsedLineResult {
    pub event: Option<NormalizedUsageEvent>,
    pub rate_limits: Vec<ObservedRateLimitWindow>,
}

/// Deduplicate events sharing the same `event_id` within a batch, keeping the latest record.
pub(crate) fn collapse_duplicate_events_keep_last(
    events: Vec<NormalizedUsageEvent>,
) -> Vec<NormalizedUsageEvent> {
    if events.len() <= 1 {
        return events;
    }
    let mut index_by_id: HashMap<String, usize> = HashMap::with_capacity(events.len());
    let mut deduped: Vec<NormalizedUsageEvent> = Vec::with_capacity(events.len());
    for ev in events {
        if let Some(&idx) = index_by_id.get(&ev.event_id) {
            deduped[idx] = ev;
        } else {
            let idx = deduped.len();
            index_by_id.insert(ev.event_id.clone(), idx);
            deduped.push(ev);
        }
    }
    deduped
}

/// Parse an ISO-8601 / RFC-3339 timestamp string or epoch seconds/ms number into Unix epoch milliseconds.
pub fn parse_timestamp_ms(val: Option<&Value>) -> Option<i64> {
    let v = val?;
    if let Some(num) = v.as_i64() {
        return Some(if num.abs() < 10_000_000_000 {
            num * 1000
        } else {
            num
        });
    }
    if let Some(num_f) = v.as_f64() {
        if num_f.is_finite() {
            let ms = if num_f.abs() < 10_000_000_000.0 {
                (num_f * 1000.0).round() as i64
            } else {
                num_f.round() as i64
            };
            return Some(ms);
        }
    }
    let s = v.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y/%m/%d %H:%M:%S") {
        return Some(ndt.and_utc().timestamp_millis());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(ndt.and_utc().timestamp_millis());
    }
    None
}

pub(crate) fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

pub(crate) fn get_i64(obj: &Value, keys: &[&str]) -> i64 {
    for k in keys {
        if let Some(v) = obj.get(*k) {
            if let Some(i) = v.as_i64() {
                return i.max(0);
            }
            if let Some(u) = v.as_u64() {
                return (u.min(i64::MAX as u64)) as i64;
            }
        }
    }
    0
}

pub(crate) fn compute_tps(output_tokens: i64, duration_ms: Option<i64>) -> Option<f64> {
    tokentracker_core::domain::compute_tps(output_tokens, duration_ms)
}

pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
