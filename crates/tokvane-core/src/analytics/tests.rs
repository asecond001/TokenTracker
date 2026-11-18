use super::*;
use crate::domain::{compute_event_id, NormalizedUsageEvent};

fn seed_sample_events(storage: &Storage) -> i64 {
    let dev = storage.local_device().expect("device").device_id;
    // 2026-11-10T10:00:00Z = 1_794_304_800_000
    let base_ts = Utc
        .with_ymd_and_hms(2026, 11, 10, 10, 0, 0)
        .single()
        .expect("valid date")
        .timestamp_millis();

    let events = vec![
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "s1", "m1", base_ts),
            device_id: dev.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "s1".to_string(),
            project_label: "app".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: base_ts,
            input_tokens: 100_000,
            output_tokens: 20_000,
            cache_read_tokens: 400_000,
            cache_write_tokens: 50_000,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(4_000),
            ttft_ms: Some(300),
            output_tps: Some(80.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        NormalizedUsageEvent {
            event_id: compute_event_id("codex", "s2", "m2", base_ts + 3_600_000),
            device_id: dev,
            provider_id: "codex".to_string(),
            session_id: "s2".to_string(),
            project_label: "cli".to_string(),
            model_id: "gpt-5-codex".to_string(),
            timestamp_ms: base_ts + 3_600_000,
            input_tokens: 50_000,
            output_tokens: 10_000,
            cache_read_tokens: 50_000,
            cache_write_tokens: 0,
            reasoning_tokens: 5_000,
            observable_only: false,
            duration_ms: Some(3_000),
            ttft_ms: None,
            output_tps: Some(50.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
    ];

    storage
        .ingest_events_with_cursor(&events, None, &[])
        .expect("ingest");
    base_ts
}

/// File-backed storage routes analytics through `SQLITE_OPEN_READ_ONLY`
/// pooled connections; any accidental write in a query path would fail here.
#[test]
fn file_backed_analytics_run_on_read_only_pool() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage = Storage::open_file(&dir.path().join("pool.db")).expect("open db");
    let base_ts = seed_sample_events(&storage);
    let now_ms = base_ts + 2 * HOUR_MS;
    let filter = AnalyticsFilter {
        start_ms: base_ts - HOUR_MS,
        end_ms: base_ts + 4 * HOUR_MS,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };

    // Writes through the read path must be rejected.
    let write_attempt = storage.with_read_conn(|conn| {
        conn.execute("DELETE FROM usage_events", [])?;
        Ok(())
    });
    assert!(write_attempt.is_err());

    let ov = storage.query_overview(&filter, now_ms).expect("overview");
    assert_eq!(ov.total_requests, 2);
    storage
        .query_trends(&TrendsQuery {
            filter: filter.clone(),
            granularity: TrendGranularity::Auto,
            dimension: StackDimension::Provider,
        })
        .expect("trends");
    storage
        .query_monthly(&MonthlyQuery {
            year: 2026,
            month: 11,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
        })
        .expect("monthly");
    storage.query_models_pricing(&filter).expect("models");
    storage.query_speed_latency(&filter).expect("speed");
    storage.query_subscriptions_with_tz(now_ms, 480).expect("subs");
    storage.query_subscription_templates(now_ms).expect("templates");

    // Event count is seeded from COUNT(*) once, then advanced by ingests.
    let tray = storage.query_tray_summary(now_ms, 480).expect("tray");
    assert_eq!(tray.total_ingested_events, 2);
    let dev = storage.local_device().expect("device").device_id;
    let extra = NormalizedUsageEvent {
        event_id: compute_event_id("codex", "s3", "m3", base_ts + 2 * HOUR_MS),
        device_id: dev,
        provider_id: "codex".to_string(),
        session_id: "s3".to_string(),
        project_label: "cli".to_string(),
        model_id: "gpt-5-codex".to_string(),
        timestamp_ms: base_ts + 2 * HOUR_MS,
        input_tokens: 1_000,
        output_tokens: 100,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms: None,
        ttft_ms: None,
        output_tps: None,
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    };
    let outcome = storage
        .ingest_events_with_cursor(&[extra.clone(), extra], None, &[])
        .expect("ingest extra");
    assert_eq!(outcome.inserted, 1);
    let tray = storage.query_tray_summary(now_ms, 480).expect("tray");
    assert_eq!(tray.total_ingested_events, 3);
    let actual: i64 = storage
        .with_read_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0))?))
        .expect("count");
    assert_eq!(actual, 3);
}

#[test]
fn overview_monthly_models_and_speed_queries_align() {
    let storage = Storage::open_in_memory().expect("db");
    let base_ts = seed_sample_events(&storage);

    let filter = AnalyticsFilter {
        start_ms: base_ts - HOUR_MS,
        end_ms: base_ts + 4 * HOUR_MS,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };
    let ov = storage
        .query_overview(&filter, base_ts + 2 * HOUR_MS)
        .expect("overview");
    assert_eq!(ov.total_requests, 2);
    assert_eq!(ov.total_tokens, 685_000);
    assert!(ov.total_cost_micro_usd > 0);
    assert_eq!(ov.provider_series.len(), 2);
    assert_eq!(
        ov.provider_series[0].points.len(),
        ov.series.len()
    );

    let monthly = storage
        .query_monthly(&MonthlyQuery {
            year: 2026,
            month: 11,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
        })
        .expect("monthly");
    assert_eq!(monthly.month_total_tokens, 685_000);
    assert_eq!(monthly.api_equivalent_cost_micro_usd, ov.total_cost_micro_usd);
    // Zero user-configured plans => zero fee & None ROI
    assert_eq!(monthly.subscription_monthly_fee_micro_usd, 0);
    assert!(monthly.roi_multiplier.is_none());
    let csv = export_monthly_csv(&monthly);
    assert!(csv.contains("2026-11-10"));

    let models = storage.query_models_pricing(&filter).expect("models");
    assert_eq!(models.rows.len(), 2);
    assert_eq!(models.total_cost_micro_usd, ov.total_cost_micro_usd);

    let speed = storage.query_speed_latency(&filter).expect("speed");
    assert_eq!(speed.providers.len(), 2);
    assert_eq!(speed.models.len(), 2);
}

#[test]
fn all_time_trends_auto_observed_fallback_and_speed_outliers() {
    let storage = Storage::open_in_memory().expect("db");
    let base_ts = seed_sample_events(&storage);
    let dev = storage.local_device().expect("device").device_id;

    // Add an unphysical TPS outlier (> 800) and a short-duration step (< 300ms) + a valid observed limit
    let outliers = vec![
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "s-out", "m-fast", base_ts + 120_000),
            device_id: dev.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "s-out".to_string(),
            project_label: "app".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: base_ts + 120_000,
            input_tokens: 1_000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(150),
            ttft_ms: None,
            output_tps: Some(3_333.3),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        NormalizedUsageEvent {
            event_id: compute_event_id("codex", "s-old", "m-old", base_ts - 5 * DAY_MS),
            device_id: dev,
            provider_id: "codex".to_string(),
            session_id: "s-old".to_string(),
            project_label: "cli".to_string(),
            model_id: "gpt-5-codex".to_string(),
            timestamp_ms: base_ts - 5 * DAY_MS,
            input_tokens: 10_000,
            output_tokens: 5_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(2_000),
            ttft_ms: Some(200),
            output_tps: Some(60.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
    ];
    let observed = vec![ObservedRateLimitWindow {
        provider_id: "codex".to_string(),
        window_name: "primary".to_string(),
        used_percent: 42.5,
        window_minutes: Some(300),
        resets_at_ms: Some(base_ts + 4 * HOUR_MS),
        observed_at_ms: base_ts,
        source: Some("local_log".to_string()),
        plan_hint: Some("plus".to_string()),
    }];
    storage
        .ingest_events_with_cursor(&outliers, None, &observed)
        .expect("ingest outliers");

    let all_time_filter = AnalyticsFilter {
        start_ms: base_ts,
        end_ms: base_ts + 2 * HOUR_MS,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: true,
    };
    let ov = storage
        .query_overview(&all_time_filter, base_ts + HOUR_MS)
        .expect("all_time overview");
    assert!(ov.tokens_delta_pct.is_none());
    assert!(ov.cost_delta_pct.is_none());
    assert!(ov.cache_delta_pct.is_none());
    assert!(ov.tps_delta_pct.is_none());
    let codex_row = ov
        .providers
        .iter()
        .find(|p| p.provider_id == "codex")
        .expect("codex row");
    assert_eq!(codex_row.active_window_label.as_deref(), Some("5h 滚动窗口"));
    assert_eq!(codex_row.active_window_used_percent, Some(42.5));

    let trends = storage
        .query_trends(&TrendsQuery {
            filter: all_time_filter.clone(),
            granularity: TrendGranularity::Auto,
            dimension: StackDimension::Provider,
        })
        .expect("all_time trends");
    assert_eq!(trends.effective_granularity, TrendGranularity::Daily);
    assert!(trends.buckets.len() >= 5);
    assert!(trends.comparisons.iter().all(|c| c.delta_pct.is_none() && c.previous_tokens == 0));

    let speed = storage
        .query_speed_latency(&all_time_filter)
        .expect("speed filtered");
    assert!(speed.providers.iter().all(|p| p.max_tps <= 450.0));
    assert!(speed.models.iter().all(|m| m.max_tps <= 450.0));
}

#[test]
fn heatmap_365d_aligns_with_daily_ledger_and_includes_observable_only() {
    let storage = Storage::open_in_memory().expect("db");
    let dev = storage.local_device().expect("device").device_id;

    // 2026-11-15T23:30:00Z:
    //   - UTC+8 (+480): 2026-11-16 07:30 (+8) -> must appear in 2026-11-16
    //   - UTC-5 (-300): 2026-11-15 18:30 (-5) -> must appear in 2026-11-15
    let cross_boundary_ts = Utc
        .with_ymd_and_hms(2026, 11, 15, 23, 30, 0)
        .single()
        .expect("ts")
        .timestamp_millis();

    // Observable-only Antigravity event on 2026-11-12T10:00:00Z with 0 tokens
    let obs_ts = Utc
        .with_ymd_and_hms(2026, 11, 12, 10, 0, 0)
        .single()
        .expect("ts")
        .timestamp_millis();

    let events = vec![
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "s-tz", "m1", cross_boundary_ts),
            device_id: dev.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "s-tz".to_string(),
            project_label: "app".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: cross_boundary_ts,
            input_tokens: 12_000,
            output_tokens: 3_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(1_000),
            ttft_ms: None,
            output_tps: Some(30.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        NormalizedUsageEvent {
            event_id: compute_event_id("antigravity", "s-obs", "m2", obs_ts),
            device_id: dev,
            provider_id: "antigravity".to_string(),
            session_id: "s-obs".to_string(),
            project_label: "agt".to_string(),
            model_id: "antigravity-default".to_string(),
            timestamp_ms: obs_ts,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: true,
            duration_ms: None,
            ttft_ms: None,
            output_tps: None,
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
    ];
    storage
        .ingest_events_with_cursor(&events, None, &[])
        .expect("ingest");

    // Verify UTC+8
    let m_plus8 = storage
        .query_monthly(&MonthlyQuery {
            year: 2026,
            month: 11,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
        })
        .expect("monthly +8");
    assert_eq!(m_plus8.daily_ledger.len(), 2);
    assert_eq!(m_plus8.heatmap_365d.len(), 2);
    for ledger in &m_plus8.daily_ledger {
        let cell = m_plus8
            .heatmap_365d
            .iter()
            .find(|c| c.date == ledger.date)
            .expect("matching heatmap date");
        assert_eq!(cell.total_tokens, ledger.total_tokens);
        assert_eq!(cell.requests, ledger.requests);
    }
    assert!(m_plus8.heatmap_365d.iter().any(|c| c.date == "2026-11-16"));
    let obs_cell = m_plus8
        .heatmap_365d
        .iter()
        .find(|c| c.date == "2026-11-12")
        .expect("observable-only day present in heatmap");
    assert_eq!(obs_cell.total_tokens, 0);
    assert_eq!(obs_cell.requests, 1);

    // Verify UTC-5
    let m_minus5 = storage
        .query_monthly(&MonthlyQuery {
            year: 2026,
            month: 11,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: -300,
        })
        .expect("monthly -5");
    assert!(m_minus5
        .heatmap_365d
        .iter()
        .any(|c| c.date == "2026-11-15" && c.total_tokens == 15_000));
}

#[test]
fn overview_and_trends_exclude_zero_token_series() {
    let storage = Storage::open_in_memory().expect("db");
    let dev = storage.local_device().expect("device").device_id;
    let ts = Utc
        .with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .single()
        .expect("ts")
        .timestamp_millis();

    // Ingest claude_code with real tokens (0 reasoning, 0 cache_write) + antigravity observable_only (0 tokens)
    let events = vec![
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "s1", "m1", ts),
            device_id: dev.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "s1".to_string(),
            project_label: "app".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: ts,
            input_tokens: 1_000,
            output_tokens: 200,
            cache_read_tokens: 500,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(1_000),
            ttft_ms: None,
            output_tps: Some(20.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        NormalizedUsageEvent {
            event_id: compute_event_id("antigravity", "s2", "m2", ts + 1_000),
            device_id: dev,
            provider_id: "antigravity".to_string(),
            session_id: "s2".to_string(),
            project_label: "agt".to_string(),
            model_id: "antigravity-default".to_string(),
            timestamp_ms: ts + 1_000,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: true,
            duration_ms: None,
            ttft_ms: None,
            output_tps: None,
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
    ];
    storage
        .ingest_events_with_cursor(&events, None, &[])
        .expect("ingest");

    let filter = AnalyticsFilter {
        start_ms: ts - HOUR_MS,
        end_ms: ts + HOUR_MS,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };
    let ov = storage.query_overview(&filter, ts + HOUR_MS).expect("overview");
    assert_eq!(ov.provider_series.len(), 1);
    assert_eq!(ov.provider_series[0].key, "claude_code");

    let prov_trends = storage
        .query_trends(&TrendsQuery {
            filter: filter.clone(),
            granularity: TrendGranularity::Hourly,
            dimension: StackDimension::Provider,
        })
        .expect("provider trends");
    assert_eq!(prov_trends.series.len(), 1);
    assert_eq!(prov_trends.series[0].key, "claude_code");

    let tok_trends = storage
        .query_trends(&TrendsQuery {
            filter,
            granularity: TrendGranularity::Hourly,
            dimension: StackDimension::TokenType,
        })
        .expect("token type trends");
    assert!(tok_trends.series.iter().all(|s| s.total_tokens > 0));
    assert!(tok_trends.series.iter().all(|s| s.key != "reasoning"));
}

#[test]
fn active_sessions_and_shares_classify_active_vs_recent_and_sanitize_labels() {
    let storage = Storage::open_in_memory().expect("db");
    let dev = storage.local_device().expect("device").device_id;
    let now_ms = Utc
        .with_ymd_and_hms(2026, 11, 16, 12, 0, 0)
        .single()
        .expect("now")
        .timestamp_millis();

    let events = vec![
        // Active (45s ago) + earlier step 2h ago in same session
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "sess-alpha", "m1", now_ms - 7_200_000),
            device_id: dev.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "sess-alpha".to_string(),
            project_label: "C:\\Users\\alice\\repos\\my-secret-app".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: now_ms - 7_200_000,
            input_tokens: 1_000,
            output_tokens: 200,
            cache_read_tokens: 400,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(1_500),
            ttft_ms: None,
            output_tps: Some(45.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "sess-alpha", "m2", now_ms - 45_000),
            device_id: dev.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "sess-alpha".to_string(),
            project_label: "C:\\Users\\alice\\repos\\my-secret-app\\".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: now_ms - 45_000,
            input_tokens: 2_000,
            output_tokens: 300,
            cache_read_tokens: 600,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(1_000),
            ttft_ms: None,
            output_tps: Some(82.5),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        // Recent (10 min ago)
        NormalizedUsageEvent {
            event_id: compute_event_id("kimi_code", "sess-beta", "m1", now_ms - 600_000),
            device_id: dev.clone(),
            provider_id: "kimi_code".to_string(),
            session_id: "sess-beta".to_string(),
            project_label: "/home/dev/worktree/backend-api".to_string(),
            model_id: "kimi-k2-0905-preview".to_string(),
            timestamp_ms: now_ms - 600_000,
            input_tokens: 500,
            output_tokens: 150,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(2_000),
            ttft_ms: None,
            output_tps: Some(55.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        // Too old (20 min ago) -> excluded from active_sessions
        NormalizedUsageEvent {
            event_id: compute_event_id("codex", "sess-gamma", "m1", now_ms - 1_200_000),
            device_id: dev,
            provider_id: "codex".to_string(),
            session_id: "sess-gamma".to_string(),
            project_label: "old-project".to_string(),
            model_id: "gpt-5-codex".to_string(),
            timestamp_ms: now_ms - 1_200_000,
            input_tokens: 300,
            output_tokens: 100,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(1_000),
            ttft_ms: None,
            output_tps: Some(60.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
    ];
    storage
        .ingest_events_with_cursor(&events, None, &[])
        .expect("ingest");

    let ov = storage
        .query_overview(
            &AnalyticsFilter {
                start_ms: now_ms - 6 * HOUR_MS,
                end_ms: now_ms,
                provider_id: None,
                device_id: None,
                tz_offset_minutes: 480,
                all_time: false,
            },
            now_ms,
        )
        .expect("overview");

    assert_eq!(ov.active_sessions.len(), 2);
    let s0 = &ov.active_sessions[0];
    assert_eq!(s0.provider_id, "claude_code");
    assert_eq!(s0.activity_state, "active");
    assert_eq!(s0.project_label, "my-secret-app");
    assert!(s0.session_key.starts_with('#') && s0.session_key.len() == 7);
    assert_ne!(s0.session_key, "sess-alpha");
    assert_eq!(s0.request_count, 2);
    assert_eq!(s0.total_tokens, 4_500);
    assert_eq!(s0.latest_output_tps, Some(82.5));

    let s1 = &ov.active_sessions[1];
    assert_eq!(s1.provider_id, "kimi_code");
    assert_eq!(s1.activity_state, "recent");
    assert_eq!(s1.project_label, "backend-api");

    assert_eq!(ov.provider_shares.len(), 3);
    let pct_sum: f64 = ov
        .provider_shares
        .iter()
        .map(|p| p.token_share_percent)
        .sum();
    assert!((pct_sum - 100.0).abs() < 1e-6);
    assert_eq!(ov.top_model_shares.len(), 3);

    let tray = storage.query_tray_summary(now_ms, 480).expect("tray");
    assert_eq!(tray.active_sessions.len(), 2);
    assert_eq!(tray.provider_shares.len(), 3);
}

#[test]
fn multi_device_filter_isolates_overview_trends_monthly_and_models() {
    use crate::domain::{SyncEnvelopeV1, SyncHourlyDelta};

    let storage = Storage::open_in_memory().expect("db");
    let base_ts = seed_sample_events(&storage);
    let bucket = base_ts.div_euclid(HOUR_MS) * HOUR_MS;
    let remote_envelope = SyncEnvelopeV1 {
        schema_version: 1,
        device_id: "remote-mbp".to_string(),
        device_display_name: "MacBook Pro".to_string(),
        generated_at_ms: base_ts + 5_000,
        since_watermark_ms: 0,
        hourly_deltas: vec![SyncHourlyDelta {
            provider_id: "deepseek_harness".to_string(),
            model_id: "deepseek-reasoner".to_string(),
            bucket_start_ms: bucket,
            request_count: 3,
            observable_step_count: 0,
            input_tokens: 20_000,
            output_tokens: 10_000,
            cache_read_tokens: 5_000,
            cache_write_tokens: 0,
            reasoning_tokens: 15_000,
            estimated_cost_micro_usd: 420_000,
            cache_saved_micro_usd: 10_000,
            total_duration_ms: 9_000,
            tps_sample_count: 3,
            tps_sum: 150.0,
            max_output_tps: 60.0,
            updated_at_ms: base_ts + 5_000,
        }],
        subscription_snapshots: Vec::new(),
    };
    storage
        .apply_remote_sync_envelope_v1(&remote_envelope, base_ts + 6_000)
        .expect("apply remote");

    // All devices = local (685k) + remote (50k) = 735k
    let all_filter = AnalyticsFilter {
        start_ms: base_ts - HOUR_MS,
        end_ms: base_ts + 4 * HOUR_MS,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };
    let all_ov = storage
        .query_overview(&all_filter, base_ts + 2 * HOUR_MS)
        .expect("all overview");
    assert_eq!(all_ov.total_tokens, 735_000);

    // Single remote device = 50k tokens
    let remote_filter = AnalyticsFilter {
        device_id: Some("remote-mbp".to_string()),
        ..all_filter.clone()
    };
    let remote_ov = storage
        .query_overview(&remote_filter, base_ts + 2 * HOUR_MS)
        .expect("remote overview");
    assert_eq!(remote_ov.total_tokens, 50_000);
    assert_eq!(remote_ov.total_cost_micro_usd, 420_000);
    assert_eq!(remote_ov.active_sessions.len(), 0);

    let remote_trends = storage
        .query_trends(&TrendsQuery {
            filter: remote_filter.clone(),
            granularity: TrendGranularity::Hourly,
            dimension: StackDimension::Provider,
        })
        .expect("remote trends");
    assert_eq!(remote_trends.series.len(), 1);
    assert_eq!(remote_trends.series[0].key, "deepseek_harness");

    let remote_monthly = storage
        .query_monthly(&MonthlyQuery {
            year: 2026,
            month: 11,
            provider_id: None,
            device_id: Some("remote-mbp".to_string()),
            tz_offset_minutes: 480,
        })
        .expect("remote monthly");
    assert_eq!(remote_monthly.month_total_tokens, 50_000);

    let remote_models = storage
        .query_models_pricing(&remote_filter)
        .expect("remote models");
    assert_eq!(remote_models.rows.len(), 1);
    assert_eq!(remote_models.rows[0].model_id, "deepseek-reasoner");

    // Pure-rollup remote device must also support speed latency, latest TPS, and 15m trend fallback.
    let remote_speed = storage
        .query_speed_latency(&remote_filter)
        .expect("remote speed latency");
    assert!(remote_speed.from_hourly_rollups);
    assert_eq!(remote_speed.providers.len(), 1);
    assert_eq!(remote_speed.providers[0].sample_count, 3);
    assert!((remote_speed.providers[0].mean_tps - 50.0).abs() < 1e-6);
    assert!((remote_speed.providers[0].max_tps - 60.0).abs() < 1e-6);
    assert_eq!(remote_speed.providers[0].avg_ttft_ms, None);
    assert_eq!(remote_speed.models.len(), 1);
    assert_eq!(remote_speed.models[0].model_id, "deepseek-reasoner");

    assert!(remote_ov.latest_output_tps.is_some());
    assert!((remote_ov.latest_output_tps.expect("latest tps") - 50.0).abs() < 1e-6);

    let remote_15m_trends = storage
        .query_trends(&TrendsQuery {
            filter: remote_filter,
            granularity: TrendGranularity::FifteenMinutes,
            dimension: StackDimension::Provider,
        })
        .expect("remote 15m fallback trends");
    assert_eq!(remote_15m_trends.series.len(), 1);
    assert_eq!(remote_15m_trends.series[0].key, "deepseek_harness");

    // Empty query should report from_hourly_rollups = false cleanly.
    let empty_speed = storage
        .query_speed_latency(&AnalyticsFilter {
            device_id: Some("non-existent-device".to_string()),
            ..all_filter
        })
        .expect("empty speed latency");
    assert!(!empty_speed.from_hourly_rollups);
    assert!(empty_speed.providers.is_empty());
    assert!(empty_speed.models.is_empty());
}

#[test]
fn overview_today_shares_remain_fixed_across_time_ranges() {
    let storage = Storage::open_in_memory().expect("in-memory");
    let dev_id = storage.local_device().expect("local device").device_id;
    let now_ms = 1_794_800_000_000i64; // 2026-11-15 in UTC+8
    let (today_start_ms, _) = local_day_bounds_ms(now_ms, 480);
    let today_ts = today_start_ms + HOUR_MS;
    let yesterday_ts = today_start_ms - 4 * HOUR_MS;

    let events = vec![
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "s-today", "m-today", today_ts),
            device_id: dev_id.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "s-today".to_string(),
            project_label: "proj".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: today_ts,
            input_tokens: 1_000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(1_000),
            ttft_ms: None,
            output_tps: Some(50.0),
            estimated_cost_micro_usd: 15_000,
            cache_saved_micro_usd: 0,
        },
        NormalizedUsageEvent {
            event_id: compute_event_id("codex", "s-yesterday", "m-yday", yesterday_ts),
            device_id: dev_id,
            provider_id: "codex".to_string(),
            session_id: "s-yesterday".to_string(),
            project_label: "proj".to_string(),
            model_id: "gpt-5.3-codex".to_string(),
            timestamp_ms: yesterday_ts,
            input_tokens: 9_000,
            output_tokens: 1_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(2_000),
            ttft_ms: None,
            output_tps: Some(50.0),
            estimated_cost_micro_usd: 90_000,
            cache_saved_micro_usd: 0,
        },
    ];
    storage
        .ingest_events_with_cursor(&events, None, &[])
        .expect("ingest");

    let today_filter = AnalyticsFilter {
        start_ms: today_start_ms,
        end_ms: now_ms,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };
    let range_30d_filter = AnalyticsFilter {
        start_ms: now_ms - 30 * 24 * HOUR_MS,
        end_ms: now_ms,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };

    let ov_today = storage.query_overview(&today_filter, now_ms).expect("today");
    let ov_30d = storage.query_overview(&range_30d_filter, now_ms).expect("30d");

    // Trend totals change between today vs 30d
    assert_eq!(ov_today.total_tokens, 1_500);
    assert_eq!(ov_30d.total_tokens, 11_500);

    // Today live pulse share cards stay pinned to today regardless of filter range
    assert_eq!(ov_today.provider_shares.len(), 1);
    assert_eq!(ov_today.provider_shares[0].key, "claude_code");
    assert_eq!(ov_today.provider_shares[0].tokens, 1_500);
    assert_eq!(ov_30d.provider_shares, ov_today.provider_shares);
    assert_eq!(ov_30d.top_model_shares, ov_today.top_model_shares);
    assert_eq!(ov_30d.top_model_shares.len(), 1);
    assert_eq!(ov_30d.top_model_shares[0].key, "claude-sonnet-4-5");
}

#[test]
fn overview_series_zero_fills_gaps_and_active_sessions_resolve_titles() {
    let storage = Storage::open_in_memory().expect("in-memory");
    let dev_id = storage.local_device().expect("device").device_id;
    let now_ms = 1_794_830_400_000i64; // aligned hour boundary in Nov 2026
    let h0 = now_ms - 3 * HOUR_MS;
    let h3 = now_ms - 10_000;

    let events = vec![
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "sess-uuid-1", "m1", h0),
            device_id: dev_id.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "sess-uuid-1".to_string(),
            project_label: "TokenVane".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: h0,
            input_tokens: 1_000,
            output_tokens: 120,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(2_000),
            ttft_ms: None,
            output_tps: Some(60.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "sess-uuid-1", "m2", h3),
            device_id: dev_id,
            provider_id: "claude_code".to_string(),
            session_id: "sess-uuid-1".to_string(),
            project_label: "TokenVane".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: h3,
            input_tokens: 2_000,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(2_500),
            ttft_ms: None,
            output_tps: Some(80.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
    ];
    let titles = vec![crate::domain::SessionTitleRecord {
        session_key: "claude_code:sess-uuid-1".to_string(),
        provider_id: "claude_code".to_string(),
        title: "Refactor Chart Stacking".to_string(),
        source: "explicit".to_string(),
        updated_at_ms: h3,
    }];
    storage
        .ingest_events_with_cursor_and_titles(&events, None, &[], &titles)
        .expect("ingest");

    let filter = AnalyticsFilter {
        start_ms: h0,
        end_ms: now_ms,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };
    let ov = storage.query_overview(&filter, now_ms).expect("overview");
    // Continuous hourly buckets from h0 through now_ms: 4 buckets with zeros in middle gaps
    assert_eq!(ov.series.len(), 4);
    assert_eq!(ov.series[1].input_tokens, 0);
    assert_eq!(ov.series[1].output_tokens, 0);
    assert_eq!(ov.provider_series[0].points.len(), ov.series.len());
    assert_eq!(ov.active_sessions.len(), 1);
    assert_eq!(
        ov.active_sessions[0].session_title.as_deref(),
        Some("Refactor Chart Stacking")
    );
}

#[test]
fn active_sessions_and_live_tps_exclude_background_guardian_review_sessions() {
    let storage = Storage::open_in_memory().expect("in-memory");
    let dev_id = storage.local_device().expect("device").device_id;
    let now_ms = 1_794_830_400_000i64;

    let events = vec![
        NormalizedUsageEvent {
            event_id: compute_event_id("codex", "sess-user-real", "step-1", now_ms - 15_000),
            device_id: dev_id.clone(),
            provider_id: "codex".to_string(),
            session_id: "sess-user-real".to_string(),
            project_label: "TokenVane".to_string(),
            model_id: "gpt-5.4".to_string(),
            timestamp_ms: now_ms - 15_000,
            input_tokens: 2_000,
            output_tokens: 300,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(3_000),
            ttft_ms: None,
            output_tps: Some(100.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        // Background Guardian auto-review event (newer timestamp, high token rate)
        NormalizedUsageEvent {
            event_id: compute_event_id("codex", "sess-guardian-bg", "step-bg", now_ms - 2_000),
            device_id: dev_id,
            provider_id: "codex".to_string(),
            session_id: "sess-guardian-bg".to_string(),
            project_label: "TokenVane".to_string(),
            model_id: "codex-auto-review".to_string(),
            timestamp_ms: now_ms - 2_000,
            input_tokens: 1_500,
            output_tokens: 250,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(1_000),
            ttft_ms: None,
            output_tps: Some(250.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
    ];
    let titles = vec![
        crate::domain::SessionTitleRecord {
            session_key: "codex:sess-user-real".to_string(),
            provider_id: "codex".to_string(),
            title: "Implement v0.2.6 features".to_string(),
            source: "user_prompt".to_string(),
            updated_at_ms: now_ms - 15_000,
        },
        crate::domain::SessionTitleRecord {
            session_key: "codex:sess-guardian-bg".to_string(),
            provider_id: "codex".to_string(),
            title: "Guardian review".to_string(),
            source: "background".to_string(),
            updated_at_ms: now_ms - 2_000,
        },
    ];
    storage
        .ingest_events_with_cursor_and_titles(&events, None, &[], &titles)
        .expect("ingest");

    let ov = storage
        .query_overview(
            &AnalyticsFilter {
                start_ms: now_ms - HOUR_MS,
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
    assert_eq!(
        ov.active_sessions[0].session_title.as_deref(),
        Some("Implement v0.2.6 features")
    );
    assert_eq!(ov.active_sessions[0].model_id, "gpt-5.4");
    // Live TPS fallback & active speed must reflect user session (100 tok/s), NOT 250 tok/s auto-review.
    assert_eq!(ov.latest_output_tps, Some(100.0));
    assert!(!ov.live_model_speeds.iter().any(|s| s.model_id == "codex-auto-review"));
}

#[test]
fn trends_daily_buckets_and_comparisons_align_to_local_timezone() {
    let storage = Storage::open_in_memory().expect("in-memory");
    let dev_id = storage.local_device().expect("device").device_id;
    let offset_plus8 = local_offset(480);

    // Window: 2026-11-11 00:00:00.000 +08:00 through 2026-11-17 23:59:59.999 +08:00 (7 calendar days)
    let start_ms = NaiveDate::from_ymd_opt(2026, 11, 11)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .and_then(|ndt| offset_plus8.from_local_datetime(&ndt).single())
        .unwrap()
        .timestamp_millis();
    let end_ms = NaiveDate::from_ymd_opt(2026, 11, 17)
        .unwrap()
        .and_hms_milli_opt(23, 59, 59, 999)
        .and_then(|ndt| offset_plus8.from_local_datetime(&ndt).single())
        .unwrap()
        .timestamp_millis();

    // Event 1: 2026-11-11 01:00 +08:00 (= 2026-11-10 17:00 UTC) -> MUST land in 2026-11-11 +08:00
    let ts_nov11_early = start_ms + HOUR_MS;
    // Event 2: 2026-11-17 03:36 +08:00 -> MUST land in 2026-11-17 +08:00
    let ts_nov17 = start_ms + 6 * DAY_MS + 3 * HOUR_MS + 36 * 60_000;
    // Event 3: 2026-11-10 23:30 +08:00 -> MUST belong ONLY to previous 7-day period (2026-11-04..2026-11-10)
    let ts_nov10_late = start_ms - 30 * 60_000;

    let events = vec![
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "s-11", "m1", ts_nov11_early),
            device_id: dev_id.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "s-11".to_string(),
            project_label: "proj".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: ts_nov11_early,
            input_tokens: 1_000,
            output_tokens: 500,
            cache_read_tokens: 3_000,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(1_000),
            ttft_ms: None,
            output_tps: Some(50.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "s-17", "m2", ts_nov17),
            device_id: dev_id.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "s-17".to_string(),
            project_label: "proj".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: ts_nov17,
            input_tokens: 2_000,
            output_tokens: 1_000,
            cache_read_tokens: 2_000,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(1_500),
            ttft_ms: None,
            output_tps: Some(60.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "s-10", "m3", ts_nov10_late),
            device_id: dev_id,
            provider_id: "claude_code".to_string(),
            session_id: "s-10".to_string(),
            project_label: "proj".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: ts_nov10_late,
            input_tokens: 1_500,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(1_000),
            ttft_ms: None,
            output_tps: Some(40.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
    ];
    storage
        .ingest_events_with_cursor(&events, None, &[])
        .expect("ingest");

    let filter = AnalyticsFilter {
        start_ms,
        end_ms,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };

    let trends = storage
        .query_trends(&TrendsQuery {
            filter: filter.clone(),
            granularity: TrendGranularity::Auto,
            dimension: StackDimension::Provider,
        })
        .expect("trends");

    assert_eq!(trends.effective_granularity, TrendGranularity::Daily);
    assert_eq!(trends.buckets.len(), 7);
    assert_eq!(trends.buckets[0], start_ms);
    assert_eq!(trends.buckets[6], start_ms + 6 * DAY_MS);

    let claude = &trends.series[0];
    assert_eq!(claude.points[0].tokens, 4_500);
    // 3000 / (1000 + 3000) = 0.75
    assert!((claude.points[0].cache_hit_ratio - 0.75).abs() < 1e-6);
    assert_eq!(claude.points[6].tokens, 5_000);
    // 2000 / (2000 + 2000) = 0.50
    assert!((claude.points[6].cache_hit_ratio - 0.50).abs() < 1e-6);

    let cmp = &trends.comparisons[0];
    assert_eq!(cmp.current_tokens, 9_500);
    assert_eq!(cmp.previous_tokens, 2_000);
    // Overall cache hit ratio: 5000 / (3000 + 5000) = 0.625
    assert!((cmp.cache_hit_ratio - 0.625).abs() < 1e-6);
    assert!(cmp.cost_delta_pct.is_some());

    let ov = storage.query_overview(&filter, ts_nov17).expect("overview");
    assert_eq!(ov.today_hourly_pulse.len(), 24);
    assert_eq!(ov.today_requests, 1);
    assert_eq!(ov.today_tokens, 5_000);
    assert!((ov.today_cache_hit_ratio - 0.50).abs() < 1e-6);
    assert_eq!(ov.today_hourly_pulse[3].input_tokens, 2_000);

    let monthly = storage
        .query_monthly(&MonthlyQuery {
            year: 2026,
            month: 11,
            provider_id: None,
            device_id: None,
            tz_offset_minutes: 480,
        })
        .expect("monthly");
    assert_eq!(monthly.provider_shares.len(), 1);
    assert_eq!(monthly.provider_shares[0].key, "claude_code");
    assert_eq!(monthly.model_shares.len(), 1);
    assert_eq!(monthly.model_shares[0].key, "claude-sonnet-4-5");
}

#[test]
fn auto_granularity_uses_fifteen_minute_buckets_for_short_drag_ranges() {
    assert_eq!(
        resolve_trend_granularity(TrendGranularity::Auto, 90 * 60_000),
        TrendGranularity::FifteenMinutes
    );
    assert_eq!(
        resolve_trend_granularity(TrendGranularity::Auto, 6 * HOUR_MS),
        TrendGranularity::FifteenMinutes
    );
    assert_eq!(
        resolve_trend_granularity(TrendGranularity::Auto, 6 * HOUR_MS + 1),
        TrendGranularity::Hourly
    );
    assert_eq!(
        resolve_trend_granularity(TrendGranularity::Auto, 30 * DAY_MS),
        TrendGranularity::Daily
    );
    assert_eq!(
        resolve_trend_granularity(TrendGranularity::Hourly, 60_000),
        TrendGranularity::Hourly
    );
}
