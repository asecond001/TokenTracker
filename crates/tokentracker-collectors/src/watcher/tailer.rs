use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tokentracker_core::domain::{
    IngestionCursor, NormalizedUsageEvent, ObservedRateLimitWindow, SessionTitleRecord,
};
use tokentracker_core::Storage;

use crate::error::CollectorResult;
use crate::providers::ProviderLogParser;

const FLUSH_BATCH_EVENTS: usize = 2_000;
const READ_BUF_CAPACITY: usize = 64 * 1024;
const WARMUP_TAIL_BYTES: u64 = 64 * 1024;
const HEADER_SCAN_MAX_BYTES: u64 = 128 * 1024;
const HEADER_SCAN_MAX_LINES: usize = 64;

#[derive(Debug, Clone, Default)]
pub struct TailFileOutcome {
    pub bytes_read: u64,
    pub lines_processed: usize,
    pub events_inserted: usize,
    pub events_deduplicated: usize,
    pub rate_limits_updated: usize,
    pub reset_detected: bool,
}

/// Open a log file strictly read-only with full Windows shared access so host CLIs/IDEs
/// are never blocked from appending, renaming, or rotating the file.
pub fn open_shared_readonly(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ (1) | FILE_SHARE_WRITE (2) | FILE_SHARE_DELETE (4)
        opts.share_mode(0x0000_0001 | 0x0000_0002 | 0x0000_0004);
    }
    opts.open(path)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Compute a stable file identity from creation timestamp (when available) + SHA-256 of the
/// first complete `\n`-terminated line. Returns `None` if no complete line has been written yet.
fn compute_file_identity(path: &Path) -> CollectorResult<Option<(String, u64)>> {
    let mut file = open_shared_readonly(path)?;
    let meta = file.metadata()?;
    let file_len = meta.len();
    if file_len == 0 {
        return Ok(None);
    }

    let created_ns = meta
        .created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut reader = BufReader::with_capacity(8192, &mut file);
    let mut first_line = Vec::new();
    let n = reader.read_until(b'\n', &mut first_line)?;
    if n == 0 || !first_line.ends_with(b"\n") {
        // First line is still mid-write; wait until newline lands.
        return Ok(None);
    }

    let line_hash = sha256_hex(&first_line);
    let identity = format!("{created_ns}:{}", &line_hash[..24]);
    Ok(Some((identity, file_len)))
}

/// Incrementally tail a single JSONL log file using byte-offset resume cursors,
/// streaming buffered I/O, partial-trailing-line safety, and atomic SQLite ingestion.
pub fn tail_file_incremental(
    storage: &Storage,
    parser: &dyn ProviderLogParser,
    path: &Path,
    device_id: &str,
) -> CollectorResult<TailFileOutcome> {
    let Some((file_identity, file_len)) = compute_file_identity(path)? else {
        return Ok(TailFileOutcome::default());
    };

    let source_path = path.to_string_lossy().to_string();
    let existing_cursor = storage.get_cursor(&source_path)?;

    let mut reset_detected = false;
    let mut start_offset: u64 = 0;
    let mut last_line_hash = String::new();

    if let Some(cur) = &existing_cursor {
        if file_len < cur.byte_offset || file_identity != cur.file_identity {
            // File truncated or rotated: rewind safely and rely on `event_id` deduplication.
            reset_detected = true;
            start_offset = 0;
        } else {
            start_offset = cur.byte_offset;
            last_line_hash = cur.last_line_hash.clone();
        }
    }

    if !reset_detected && start_offset >= file_len {
        return Ok(TailFileOutcome::default());
    }

    let mut file = open_shared_readonly(path)?;
    let mut ctx = parser.init_context(path, device_id);
    let mut raw_buf: Vec<u8> = Vec::with_capacity(4096);

    if start_offset > 0 {
        // 1. Lightweight header pass to restore session_id, project_label, model, and initial session title.
        file.seek(SeekFrom::Start(0))?;
        let header_limit = start_offset.min(HEADER_SCAN_MAX_BYTES);
        let mut header_pos = 0u64;
        let mut header_lines = 0usize;
        {
            let mut header_reader = BufReader::with_capacity(READ_BUF_CAPACITY, &mut file);
            while header_pos < header_limit && header_lines < HEADER_SCAN_MAX_LINES {
                raw_buf.clear();
                let n = header_reader.read_until(b'\n', &mut raw_buf)?;
                if n == 0 || header_pos + (n as u64) > start_offset || !raw_buf.ends_with(b"\n") {
                    break;
                }
                header_pos += n as u64;
                header_lines += 1;
                let line_str = String::from_utf8_lossy(&raw_buf);
                let _ = parser.parse_line(&line_str, &mut ctx);
            }
        }

        // Clear transient timing/delta state before the trailing warmup slice right ahead of start_offset.
        ctx.last_prompt_ts_ms = None;
        ctx.last_step_ts_ms = None;
        ctx.active_message_turns.clear();
        ctx.last_codex_totals = None;
        ctx.last_context_tokens = None;
        ctx.pending_antigravity_event = None;
        ctx.line_number = 0;

        // 2. Trailing warmup slice (up to 64 KB immediately before start_offset) to reconstruct
        // ongoing prompt/turn timestamps, streaming chunk baselines, and cumulative delta checkpoints.
        let warmup_start = start_offset.saturating_sub(WARMUP_TAIL_BYTES);
        file.seek(SeekFrom::Start(warmup_start))?;
        let mut warmup_pos = warmup_start;
        {
            let mut warmup_reader = BufReader::with_capacity(READ_BUF_CAPACITY, &mut file);
            if warmup_start > 0 {
                raw_buf.clear();
                let skipped = warmup_reader.read_until(b'\n', &mut raw_buf)?;
                warmup_pos += skipped as u64;
            }
            while warmup_pos < start_offset {
                raw_buf.clear();
                let n = warmup_reader.read_until(b'\n', &mut raw_buf)?;
                if n == 0 || warmup_pos + (n as u64) > start_offset || !raw_buf.ends_with(b"\n") {
                    break;
                }
                warmup_pos += n as u64;
                let line_str = String::from_utf8_lossy(&raw_buf);
                let _ = parser.parse_line(&line_str, &mut ctx);
            }
        }
        ctx.line_number = 0;
    }

    file.seek(SeekFrom::Start(start_offset))?;
    let mut reader = BufReader::with_capacity(READ_BUF_CAPACITY, file);

    let mut committed_offset = start_offset;
    let mut lines_processed = 0usize;
    let mut events_inserted = 0usize;
    let mut events_deduplicated = 0usize;
    let mut rate_limits_updated = 0usize;

    let mut pending_events: Vec<NormalizedUsageEvent> = Vec::with_capacity(FLUSH_BATCH_EVENTS);
    let mut pending_rates: Vec<ObservedRateLimitWindow> = Vec::new();
    let mut pending_titles: Vec<SessionTitleRecord> = Vec::new();

    loop {
        raw_buf.clear();
        let read_len = reader.read_until(b'\n', &mut raw_buf)?;
        if read_len == 0 {
            break;
        }
        if !raw_buf.ends_with(b"\n") {
            // Incomplete line currently being written by host tool; stop before advancing offset.
            break;
        }

        committed_offset += read_len as u64;
        lines_processed += 1;

        let line_str = String::from_utf8_lossy(&raw_buf);
        last_line_hash = sha256_hex(line_str.trim().as_bytes());

        if let Some(parsed) = parser.parse_line(&line_str, &mut ctx) {
            if let Some(ev) = parsed.event {
                pending_events.push(ev);
            }
            if !parsed.rate_limits.is_empty() {
                pending_rates.extend(parsed.rate_limits);
            }
        }
        if let Some(title_rec) = ctx.take_session_title_record(parser.provider_id()) {
            pending_titles.push(title_rec);
        }

        if pending_events.len() >= FLUSH_BATCH_EVENTS {
            let batch_events = parser.finalize_batch(std::mem::take(&mut pending_events));
            let batch_rates = std::mem::take(&mut pending_rates);
            let batch_titles = std::mem::take(&mut pending_titles);
            let batch_rate_count = batch_rates.len();
            let cursor = IngestionCursor {
                source_path: source_path.clone(),
                provider_id: parser.provider_id().to_string(),
                file_identity: file_identity.clone(),
                byte_offset: committed_offset,
                last_line_hash: last_line_hash.clone(),
                updated_at_ms: Utc::now().timestamp_millis(),
            };
            let outcome = storage.ingest_events_with_cursor_and_titles(
                &batch_events,
                Some(&cursor),
                &batch_rates,
                &batch_titles,
            )?;
            events_inserted += outcome.inserted;
            events_deduplicated += outcome.deduplicated;
            rate_limits_updated += batch_rate_count;
        }
    }

    if let Some(title_rec) = ctx.take_session_title_record(parser.provider_id()) {
        pending_titles.push(title_rec);
    }

    if committed_offset > start_offset
        || reset_detected
        || !pending_events.is_empty()
        || !pending_rates.is_empty()
        || !pending_titles.is_empty()
    {
        let final_events = parser.finalize_batch(pending_events);
        let final_rate_count = pending_rates.len();
        let cursor = IngestionCursor {
            source_path,
            provider_id: parser.provider_id().to_string(),
            file_identity,
            byte_offset: committed_offset,
            last_line_hash,
            updated_at_ms: Utc::now().timestamp_millis(),
        };
        let outcome = storage.ingest_events_with_cursor_and_titles(
            &final_events,
            Some(&cursor),
            &pending_rates,
            &pending_titles,
        )?;
        events_inserted += outcome.inserted;
        events_deduplicated += outcome.deduplicated;
        rate_limits_updated += final_rate_count;
    }

    Ok(TailFileOutcome {
        bytes_read: committed_offset.saturating_sub(start_offset),
        lines_processed,
        events_inserted,
        events_deduplicated,
        rate_limits_updated,
        reset_detected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::claude_code::ClaudeCodeParser;
    use std::io::Write;
    use tokentracker_core::analytics::AnalyticsFilter;

    #[test]
    fn mid_file_tail_warmup_prevents_tps_spike_and_preserves_initial_session_title() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("sess-warmup.jsonl");
        let storage = Storage::open_in_memory().expect("db");
        let dev = storage.local_device().expect("device").device_id;
        let parser = ClaudeCodeParser::new();

        // Write header user prompt + bulk filler + active streaming chunk1
        let line1 = "{\"type\":\"user\",\"sessionId\":\"sess-warmup\",\"cwd\":\"C:/Projects/TokenTracker\",\"timestamp\":\"2026-11-16T01:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"Investigate floating monitor TPS\"}}\n";
        let filler = "{\"type\":\"progress\",\"timestamp\":\"2026-11-16T01:00:01.000Z\",\"payload\":\"padding-line-for-offset-beyond-64kb\"}\n";
        let chunk1 = "{\"type\":\"assistant\",\"sessionId\":\"sess-warmup\",\"timestamp\":\"2026-11-16T01:05:00.000Z\",\"message\":{\"id\":\"msg_warm\",\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":500,\"output_tokens\":600}}}\n";

        {
            let mut f = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&log_path)
                .expect("write initial");
            f.write_all(line1.as_bytes()).expect("l1");
            for _ in 0..700 {
                f.write_all(filler.as_bytes()).expect("fill");
            }
            f.write_all(chunk1.as_bytes()).expect("c1");
            f.flush().expect("flush");
        }

        let out1 = tail_file_incremental(&storage, &parser, &log_path, &dev).expect("tail 1");
        assert_eq!(out1.events_inserted, 1);

        // Append trailing flush chunk2 (600ms after chunk1 with only +5 tokens)
        let chunk2 = "{\"type\":\"assistant\",\"sessionId\":\"sess-warmup\",\"timestamp\":\"2026-11-16T01:05:00.600Z\",\"message\":{\"id\":\"msg_warm\",\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":500,\"output_tokens\":605}}}\n";
        {
            let mut f = OpenOptions::new().append(true).open(&log_path).expect("append");
            f.write_all(chunk2.as_bytes()).expect("c2");
            f.flush().expect("flush");
        }

        let out2 = tail_file_incremental(&storage, &parser, &log_path, &dev).expect("tail 2");
        assert_eq!(out2.lines_processed, 1);

        // Verify upgraded token count (605), NO 1000+ tok/s spike (latest_output_tps must be None), and initial title preserved
        let now_ms = chrono::DateTime::parse_from_rfc3339("2026-11-16T01:05:10.000Z")
            .expect("now ts")
            .timestamp_millis();
        let ov = storage
            .query_overview(
                &AnalyticsFilter {
                    start_ms: now_ms - 3_600_000,
                    end_ms: now_ms,
                    provider_id: None,
                    device_id: None,
                    tz_offset_minutes: 480,
                    all_time: false,
                },
                now_ms,
            )
            .expect("overview");
        assert_eq!(ov.active_sessions.len(), 1);
        assert_eq!(ov.active_sessions[0].output_tokens, 605);
        assert_eq!(ov.active_sessions[0].latest_output_tps, None);
        assert_eq!(
            ov.active_sessions[0].session_title.as_deref(),
            Some("Investigate floating monitor TPS")
        );
    }
}
