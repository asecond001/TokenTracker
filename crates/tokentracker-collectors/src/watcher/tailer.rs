use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tokentracker_core::domain::{IngestionCursor, NormalizedUsageEvent, ObservedRateLimitWindow};
use tokentracker_core::Storage;

use crate::error::CollectorResult;
use crate::providers::ProviderLogParser;

const FLUSH_BATCH_EVENTS: usize = 2_000;
const READ_BUF_CAPACITY: usize = 64 * 1024;

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
    file.seek(SeekFrom::Start(start_offset))?;
    let mut reader = BufReader::with_capacity(READ_BUF_CAPACITY, file);

    let mut ctx = parser.init_context(path, device_id);
    let mut committed_offset = start_offset;
    let mut lines_processed = 0usize;
    let mut events_inserted = 0usize;
    let mut events_deduplicated = 0usize;
    let mut rate_limits_updated = 0usize;

    let mut pending_events: Vec<NormalizedUsageEvent> = Vec::with_capacity(FLUSH_BATCH_EVENTS);
    let mut pending_rates: Vec<ObservedRateLimitWindow> = Vec::new();
    let mut raw_buf: Vec<u8> = Vec::with_capacity(4096);

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

        if pending_events.len() >= FLUSH_BATCH_EVENTS {
            let batch_events = parser.finalize_batch(std::mem::take(&mut pending_events));
            let batch_rates = std::mem::take(&mut pending_rates);
            let batch_rate_count = batch_rates.len();
            let cursor = IngestionCursor {
                source_path: source_path.clone(),
                provider_id: parser.provider_id().to_string(),
                file_identity: file_identity.clone(),
                byte_offset: committed_offset,
                last_line_hash: last_line_hash.clone(),
                updated_at_ms: Utc::now().timestamp_millis(),
            };
            let outcome =
                storage.ingest_events_with_cursor(&batch_events, Some(&cursor), &batch_rates)?;
            events_inserted += outcome.inserted;
            events_deduplicated += outcome.deduplicated;
            rate_limits_updated += batch_rate_count;
        }
    }

    if committed_offset > start_offset
        || reset_detected
        || !pending_events.is_empty()
        || !pending_rates.is_empty()
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
        let outcome =
            storage.ingest_events_with_cursor(&final_events, Some(&cursor), &pending_rates)?;
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
