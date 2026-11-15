pub mod antigravity;
pub mod claude_code;
pub mod codex;
pub mod official_api;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use tokentracker_core::domain::{NormalizedUsageEvent, ObservedRateLimitWindow};

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
    pub line_number: u64,
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

/// Compliant local log parser contract for each Phase 1 provider.
pub trait ProviderLogParser: Send + Sync {
    fn provider_id(&self) -> &'static str;
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
    let dur = duration_ms?;
    if output_tokens <= 0 || dur <= 0 {
        return None;
    }
    let tps = output_tokens as f64 / (dur as f64 / 1000.0);
    if tps.is_finite() && tps > 0.0 && tps <= 5_000.0 {
        Some(tps)
    } else {
        None
    }
}

pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
