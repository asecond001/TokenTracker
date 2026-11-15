use std::fs::OpenOptions;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use tempfile::tempdir;
use tokentracker_collectors::otlp::{
    ingest_otlp_logs_payload, ingest_otlp_metrics_payload, start_loopback_otlp_server,
    validate_loopback_addr,
};
use tokentracker_collectors::providers::antigravity::AntigravityParser;
use tokentracker_collectors::providers::claude_code::{read_claude_stats_cache, ClaudeCodeParser};
use tokentracker_collectors::providers::codex::CodexParser;
use tokentracker_collectors::watcher::{discover_files_for_parser, tail_file_incremental};
use tokentracker_core::analytics::AnalyticsFilter;
use tokentracker_core::domain::provider_ids;
use tokentracker_core::Storage;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

#[test]
fn replays_claude_codex_and_antigravity_fixtures_accurately() {
    let storage = Storage::open_in_memory().expect("in-memory db");
    let dev_id = storage.local_device().expect("device").device_id;
    let fx = fixtures_dir();

    // 1. Claude Code
    let cc_parser = ClaudeCodeParser::new();
    let cc_file = fx.join("claude_code").join("project_alpha").join("sess-cc-001.jsonl");
    let cc_out = tail_file_incremental(&storage, &cc_parser, &cc_file, &dev_id).expect("cc tail");
    assert_eq!(cc_out.events_inserted, 2);

    let stats_cache =
        read_claude_stats_cache(&fx.join("claude_code").join("stats-cache.json"))
            .expect("read stats")
            .expect("some stats");
    assert_eq!(stats_cache.total_sessions, 12);
    assert_eq!(stats_cache.total_messages, 480);

    // 2. Codex
    let codex_parser = CodexParser::new();
    let codex_file = fx
        .join("codex")
        .join("2026")
        .join("11")
        .join("15")
        .join("rollout-2026-11-15T10-05-00-sess-codex-001.jsonl");
    let codex_out =
        tail_file_incremental(&storage, &codex_parser, &codex_file, &dev_id).expect("codex tail");
    assert_eq!(codex_out.events_inserted, 2);

    let rate_limits = storage.list_observed_rate_limits().expect("rate limits");
    assert_eq!(rate_limits.len(), 2);
    assert!(rate_limits
        .iter()
        .any(|r| r.window_name == "primary" && (r.used_percent - 35.5).abs() < 0.01));

    // 3. Antigravity (must skip transcript_full.jsonl)
    let ag_parser = AntigravityParser::new();
    let ag_root = fx.join("antigravity");
    let ag_cfg = tokentracker_core::domain::ProviderConfig {
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        enabled: true,
        collection_mode: tokentracker_core::domain::CollectionMode::LocalLog,
        custom_log_paths: vec![ag_root.to_string_lossy().to_string()],
        has_credential: false,
        last_sync_at_ms: None,
        last_error: None,
        updated_at_ms: 0,
    };
    let ag_files = discover_files_for_parser(&ag_parser, Some(&ag_cfg));
    assert!(ag_files.iter().any(|p| p.ends_with("transcript.jsonl")));
    assert!(!ag_files.iter().any(|p| p.ends_with("transcript_full.jsonl")));

    let ag_file = ag_root
        .join("conv-001")
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl");
    let ag_out =
        tail_file_incremental(&storage, &ag_parser, &ag_file, &dev_id).expect("ag tail");
    assert_eq!(ag_out.events_inserted, 2);

    let filter = AnalyticsFilter {
        start_ms: 0,
        end_ms: i64::MAX / 2,
        provider_id: None,
        tz_offset_minutes: 480,
    };
    let ov = storage.query_overview(&filter, 1_800_000_000_000).expect("overview");
    assert_eq!(ov.total_requests, 6);
    assert_eq!(ov.observable_steps, 1);

    // Re-running all 3 files is strictly idempotent
    let cc_again = tail_file_incremental(&storage, &cc_parser, &cc_file, &dev_id).expect("re-tail");
    assert_eq!(cc_again.events_inserted, 0);
}

#[test]
fn incremental_tailer_handles_partial_lines_concurrent_writers_truncation_and_rotation() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("tail-test.db");
    let storage = Storage::open_file(&db_path).expect("db");
    let dev_id = storage.local_device().expect("device").device_id;

    let log_path = dir.path().join("sess-live.jsonl");
    let parser = ClaudeCodeParser::new();

    // Hold a concurrent write handle open while tailing (verifies Windows shared read lock compatibility)
    let mut writer = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("open writer");

    let line1 = r#"{"type":"assistant","timestamp":"2026-11-15T12:00:00Z","sessionId":"s-live","uuid":"u1","message":{"id":"m1","model":"claude-sonnet-4-5","usage":{"input_tokens":1000,"output_tokens":200}}}"#;
    writeln!(writer, "{line1}").expect("write line1");
    // Write an incomplete line without '\n'
    let partial_prefix = r#"{"type":"assistant","timestamp":"2026-11-15T12:00:05Z","sessionId":"s-live","uuid":"u2","message":{"id":"m2""#;
    write!(writer, "{partial_prefix}").expect("write partial");
    writer.flush().expect("flush");

    let out1 = tail_file_incremental(&storage, &parser, &log_path, &dev_id).expect("tail 1");
    assert_eq!(out1.events_inserted, 1);

    // Finish writing line2 while writer remains open
    let line2_rest = r#","model":"claude-sonnet-4-5","usage":{"input_tokens":500,"output_tokens":100}}}"#;
    writeln!(writer, "{line2_rest}").expect("finish line2");
    writer.flush().expect("flush2");

    let out2 = tail_file_incremental(&storage, &parser, &log_path, &dev_id).expect("tail 2");
    assert_eq!(out2.events_inserted, 1);
    drop(writer);

    // Simulate truncation + rewrite with old line1 + new line3 (must NOT double count line1!)
    {
        let mut trunc = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&log_path)
            .expect("truncate");
        writeln!(trunc, "{line1}").expect("rewrite 1");
        let line3 = r#"{"type":"assistant","timestamp":"2026-11-15T12:01:00Z","sessionId":"s-live","uuid":"u3","message":{"id":"m3","model":"claude-sonnet-4-5","usage":{"input_tokens":200,"output_tokens":50}}}"#;
        writeln!(trunc, "{line3}").expect("write 3");
        trunc.flush().expect("flush3");
    }

    let out3 = tail_file_incremental(&storage, &parser, &log_path, &dev_id).expect("tail 3");
    assert!(out3.reset_detected);
    assert_eq!(out3.events_inserted, 1);
    assert_eq!(out3.events_deduplicated, 1);

    // Verify exact total across all steps = 3 requests, 2050 tokens
    let filter = AnalyticsFilter {
        start_ms: 0,
        end_ms: i64::MAX / 2,
        provider_id: None,
        tz_offset_minutes: 480,
    };
    let ov = storage.query_overview(&filter, 1_800_000_000_000).expect("ov");
    assert_eq!(ov.total_requests, 3);
    assert_eq!(ov.total_tokens, 2_050);
}

#[tokio::test]
async fn loopback_otlp_server_receives_and_persists_metrics_and_logs() {
    // 1. Strictly enforce loopback-only binding at validation boundary
    assert!(validate_loopback_addr(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 4318).is_err());
    assert!(validate_loopback_addr(IpAddr::V4(Ipv4Addr::LOCALHOST), 4318).is_ok());

    // 2. Bind real 127.0.0.1 loopback TCP listener and verify clean graceful shutdown
    let storage = Storage::open_in_memory().expect("db");
    let handle = start_loopback_otlp_server(storage.clone(), 0)
        .await
        .expect("start loopback otlp");
    assert!(handle.port > 0);

    // 3. Ingest OTLP/JSON logs payload and verify idempotent deduplication
    let log_payload = serde_json::json!({
        "resourceLogs": [{
            "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "claude-code" } }] },
            "scopeLogs": [{
                "logRecords": [{
                    "timeUnixNano": "1760000000000000000",
                    "attributes": [
                        { "key": "event.name", "value": { "stringValue": "claude_code.api_request" } },
                        { "key": "model", "value": { "stringValue": "claude-sonnet-4-5" } },
                        { "key": "session.id", "value": { "stringValue": "otlp-e2e" } },
                        { "key": "input_tokens", "value": { "intValue": "2500" } },
                        { "key": "output_tokens", "value": { "intValue": "500" } },
                        { "key": "duration_ms", "value": { "intValue": "2500" } }
                    ]
                }]
            }]
        }]
    });

    let log_out = ingest_otlp_logs_payload(&storage, &log_payload).expect("ingest otlp logs");
    assert_eq!(log_out.received, 1);
    assert_eq!(log_out.inserted, 1);
    assert_eq!(log_out.deduplicated, 0);

    let log_replay = ingest_otlp_logs_payload(&storage, &log_payload).expect("replay otlp logs");
    assert_eq!(log_replay.inserted, 0);
    assert_eq!(log_replay.deduplicated, 1);

    // 4. Ingest OTLP/JSON metrics payload (`claude_code.token.usage`)
    let metrics_payload = serde_json::json!({
        "resourceMetrics": [{
            "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "claude-code" } }] },
            "scopeMetrics": [{
                "metrics": [{
                    "name": "claude_code.token.usage",
                    "sum": {
                        "dataPoints": [
                            {
                                "timeUnixNano": "1760000060000000000",
                                "asInt": "1200",
                                "attributes": [
                                    { "key": "model", "value": { "stringValue": "claude-sonnet-4-5" } },
                                    { "key": "session.id", "value": { "stringValue": "otlp-metric-s1" } },
                                    { "key": "type", "value": { "stringValue": "input" } }
                                ]
                            },
                            {
                                "timeUnixNano": "1760000060000000000",
                                "asInt": "300",
                                "attributes": [
                                    { "key": "model", "value": { "stringValue": "claude-sonnet-4-5" } },
                                    { "key": "session.id", "value": { "stringValue": "otlp-metric-s1" } },
                                    { "key": "type", "value": { "stringValue": "output" } }
                                ]
                            }
                        ]
                    }
                }]
            }]
        }]
    });

    let metrics_out =
        ingest_otlp_metrics_payload(&storage, &metrics_payload).expect("ingest otlp metrics");
    assert_eq!(metrics_out.received, 1);
    assert_eq!(metrics_out.inserted, 1);

    // 5. Verify total persisted tokens (2500+500 + 1200+300 = 4500) across 2 requests
    let filter = AnalyticsFilter {
        start_ms: 0,
        end_ms: i64::MAX / 2,
        provider_id: Some(provider_ids::CLAUDE_CODE.to_string()),
        tz_offset_minutes: 480,
    };
    let ov = storage
        .query_overview(&filter, 1_800_000_000_000)
        .expect("overview");
    assert_eq!(ov.total_requests, 2);
    assert_eq!(ov.total_tokens, 4_500);

    handle.stop().await;
}

#[test]
fn background_watcher_spawns_from_non_tokio_main_thread_without_panic() {
    use std::sync::mpsc;
    use std::time::Duration;
    use tokentracker_collectors::CollectorCoordinator;

    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("watcher-boot.db");
    let storage = Storage::open_file(&db_path).expect("db");

    // Disable default log roots and point Claude Code at a controlled fixture copy
    let cc_dir = dir.path().join("cc_logs");
    std::fs::create_dir_all(&cc_dir).expect("mkdir");
    let cc_log = cc_dir.join("sess-boot.jsonl");
    std::fs::write(
        &cc_log,
        concat!(
            r#"{"type":"assistant","timestamp":"2026-11-15T12:00:00Z","sessionId":"s-boot","uuid":"u-boot-1","message":{"id":"m-boot-1","model":"claude-sonnet-4-5","usage":{"input_tokens":120,"output_tokens":30}}}"#,
            "\n"
        ),
    )
    .expect("write log");

    for pid in [
        provider_ids::CLAUDE_CODE,
        provider_ids::CODEX,
        provider_ids::ANTIGRAVITY,
    ] {
        let mut cfg = tokentracker_core::domain::ProviderConfig {
            provider_id: pid.to_string(),
            enabled: pid == provider_ids::CLAUDE_CODE,
            collection_mode: tokentracker_core::domain::CollectionMode::LocalLog,
            custom_log_paths: if pid == provider_ids::CLAUDE_CODE {
                vec![cc_log.to_string_lossy().to_string()]
            } else {
                Vec::new()
            },
            has_credential: false,
            last_sync_at_ms: None,
            last_error: None,
            updated_at_ms: 0,
        };
        if pid == provider_ids::CLAUDE_CODE {
            cfg.enabled = true;
        }
        storage.upsert_provider_config(&cfg).expect("upsert cfg");
    }

    let coord = CollectorCoordinator::new(storage.clone());
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let rt_handle = rt.handle().clone();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (scan_done_tx, scan_done_rx) = mpsc::channel::<usize>();

    // Simulate Tauri `.setup(...)`: plain OS thread with NO entered Tokio runtime context.
    let spawn_thread = std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        rt_handle.spawn(coord.run_background_watcher_with_hook(shutdown_rx, move |report| {
            let _ = scan_done_tx.send(report.events_inserted);
        }))
    });

    let watcher_task = spawn_thread
        .join()
        .expect("spawning watcher from non-Tokio thread must not panic");

    let inserted = scan_done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("initial startup scan should complete");
    assert!(inserted >= 1);

    shutdown_tx.send(true).expect("signal shutdown");
    rt.block_on(watcher_task)
        .expect("watcher task should exit cleanly");
}
