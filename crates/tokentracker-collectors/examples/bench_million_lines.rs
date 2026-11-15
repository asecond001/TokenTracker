use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use tempfile::tempdir;
use tokentracker_collectors::providers::claude_code::ClaudeCodeParser;
use tokentracker_collectors::watcher::tail_file_incremental;
use tokentracker_core::analytics::AnalyticsFilter;
use tokentracker_core::Storage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const TOTAL_LINES: usize = 1_000_000;
    const APPEND_LINES: usize = 1_000;

    let dir = tempdir()?;
    let db_path = dir.path().join("bench_1m.db");
    let log_path = dir.path().join("million_events.jsonl");

    let gen_start = Instant::now();
    {
        let file = File::create(&log_path)?;
        let mut writer = BufWriter::with_capacity(256 * 1024, file);
        let base_ts_sec = 1_760_000_000i64;
        for i in 0..TOTAL_LINES {
            let ts = base_ts_sec + (i as i64 % 86_400);
            if i % 2 == 0 {
                // User turn event
                writeln!(
                    writer,
                    "{{\"type\":\"user\",\"timestamp\":{},\"sessionId\":\"bench-sess-{}\",\"cwd\":\"C:\\\\Projects\\\\BenchApp\",\"uuid\":\"u-{}\"}}",
                    ts,
                    i / 200,
                    i
                )?;
            } else {
                // Assistant turn event with realistic usage
                let model = if i % 5 == 1 {
                    "claude-opus-4-6"
                } else {
                    "claude-sonnet-4-5"
                };
                writeln!(
                    writer,
                    "{{\"type\":\"assistant\",\"timestamp\":{},\"sessionId\":\"bench-sess-{}\",\"cwd\":\"C:\\\\Projects\\\\BenchApp\",\"uuid\":\"a-{}\",\"message\":{{\"id\":\"msg-{}\",\"model\":\"{}\",\"stop_reason\":\"end_turn\",\"usage\":{{\"input_tokens\":850,\"cache_creation_input_tokens\":120,\"cache_read_input_tokens\":4200,\"output_tokens\":240}}}}}}",
                    ts + 2,
                    i / 200,
                    i,
                    i,
                    model
                )?;
            }
        }
        writer.flush()?;
    }
    let file_size_mb = std::fs::metadata(&log_path)?.len() as f64 / (1024.0 * 1024.0);
    println!(
        "Generated {TOTAL_LINES} JSONL lines ({file_size_mb:.2} MB) in {:?}",
        gen_start.elapsed()
    );

    let storage = Storage::open_file(&db_path)?;
    let device_id = storage.local_device()?.device_id;
    let parser = ClaudeCodeParser::new();

    let ingest_start = Instant::now();
    let outcome = tail_file_incremental(&storage, &parser, &log_path, &device_id)?;
    let ingest_elapsed = ingest_start.elapsed();
    println!(
        "Cold-start 1M-line ingestion: lines={}, inserted={}, dedup={}, elapsed={:?}",
        outcome.lines_processed, outcome.events_inserted, outcome.events_deduplicated, ingest_elapsed
    );
    assert_eq!(outcome.lines_processed, TOTAL_LINES);
    assert_eq!(outcome.events_inserted, TOTAL_LINES / 2);

    // Re-scan identical file -> must insert 0
    let rescan = tail_file_incremental(&storage, &parser, &log_path, &device_id)?;
    assert_eq!(rescan.events_inserted, 0);

    // Append 1,000 lines and verify incremental resume
    {
        let mut writer = BufWriter::new(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&log_path)?,
        );
        for j in 0..APPEND_LINES {
            let idx = TOTAL_LINES + j;
            writeln!(
                writer,
                "{{\"type\":\"assistant\",\"timestamp\":{},\"sessionId\":\"bench-append\",\"uuid\":\"app-{}\",\"message\":{{\"id\":\"msg-app-{}\",\"model\":\"claude-sonnet-4-5\",\"usage\":{{\"input_tokens\":400,\"output_tokens\":100}}}}}}",
                1_760_090_000 + j as i64,
                idx,
                idx
            )?;
        }
        writer.flush()?;
    }
    let inc_start = Instant::now();
    let inc_outcome = tail_file_incremental(&storage, &parser, &log_path, &device_id)?;
    println!(
        "Incremental append ({APPEND_LINES} lines): inserted={}, elapsed={:?}",
        inc_outcome.events_inserted,
        inc_start.elapsed()
    );
    assert_eq!(inc_outcome.events_inserted, APPEND_LINES);

    // Verify rollup query latency
    let query_start = Instant::now();
    let filter = AnalyticsFilter {
        start_ms: 0,
        end_ms: i64::MAX / 2,
        provider_id: None,
        tz_offset_minutes: 480,
    };
    let overview = storage.query_overview(&filter, 1_760_100_000_000)?;
    let query_elapsed = query_start.elapsed();
    println!(
        "Rollup overview query over {} requests: elapsed={:?}, total_tokens={}",
        overview.total_requests, query_elapsed, overview.total_tokens
    );
    assert_eq!(overview.total_requests, (TOTAL_LINES / 2 + APPEND_LINES) as i64);

    Ok(())
}
