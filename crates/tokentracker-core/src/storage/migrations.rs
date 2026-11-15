use chrono::Utc;
use rusqlite::{params, Connection};

use crate::domain::{provider_ids, AppPreferences, CollectionMode};
use crate::error::CoreResult;
use crate::pricing::default_pricing_catalog;
use crate::subscription::default_subscription_plans;

const SCHEMA_V1_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS devices (
    device_id TEXT PRIMARY KEY,
    device_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS provider_configs (
    provider_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL,
    collection_mode TEXT NOT NULL,
    custom_log_paths_json TEXT NOT NULL,
    has_credential INTEGER NOT NULL,
    last_sync_at_ms INTEGER,
    last_error TEXT,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS ingestion_cursors (
    source_path TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    file_identity TEXT NOT NULL,
    byte_offset INTEGER NOT NULL,
    last_line_hash TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS usage_events (
    event_id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    project_label TEXT NOT NULL,
    model_id TEXT NOT NULL,
    timestamp_ms INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    reasoning_tokens INTEGER NOT NULL,
    observable_only INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER,
    ttft_ms INTEGER,
    output_tps REAL,
    estimated_cost_micro_usd INTEGER NOT NULL,
    cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (device_id) REFERENCES devices(device_id)
);

CREATE INDEX IF NOT EXISTS idx_usage_events_ts ON usage_events(timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_usage_events_provider_ts ON usage_events(provider_id, timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_usage_events_model_ts ON usage_events(model_id, timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_usage_events_device_ts ON usage_events(device_id, timestamp_ms);

CREATE TABLE IF NOT EXISTS usage_hourly_rollups (
    device_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    bucket_start_ms INTEGER NOT NULL,
    request_count INTEGER NOT NULL,
    observable_step_count INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    reasoning_tokens INTEGER NOT NULL,
    estimated_cost_micro_usd INTEGER NOT NULL,
    cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0,
    total_duration_ms INTEGER NOT NULL,
    tps_sample_count INTEGER NOT NULL,
    tps_sum REAL NOT NULL,
    max_output_tps REAL NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms)
);

CREATE INDEX IF NOT EXISTS idx_hourly_rollups_bucket ON usage_hourly_rollups(bucket_start_ms);
CREATE INDEX IF NOT EXISTS idx_hourly_rollups_updated ON usage_hourly_rollups(updated_at_ms);

CREATE TABLE IF NOT EXISTS usage_daily_rollups (
    device_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    bucket_start_ms INTEGER NOT NULL,
    request_count INTEGER NOT NULL,
    observable_step_count INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    reasoning_tokens INTEGER NOT NULL,
    estimated_cost_micro_usd INTEGER NOT NULL,
    cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0,
    total_duration_ms INTEGER NOT NULL,
    tps_sample_count INTEGER NOT NULL,
    tps_sum REAL NOT NULL,
    max_output_tps REAL NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms)
);

CREATE INDEX IF NOT EXISTS idx_daily_rollups_bucket ON usage_daily_rollups(bucket_start_ms);

CREATE TABLE IF NOT EXISTS model_pricing (
    model_pattern TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    input_per_m_micro_usd INTEGER NOT NULL,
    output_per_m_micro_usd INTEGER NOT NULL,
    cache_read_per_m_micro_usd INTEGER NOT NULL,
    cache_write_per_m_micro_usd INTEGER NOT NULL,
    reasoning_per_m_micro_usd INTEGER NOT NULL,
    is_user_override INTEGER NOT NULL,
    effective_from TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS subscription_plans (
    plan_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    plan_name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    monthly_fee_micro_usd INTEGER NOT NULL,
    windows_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS observed_rate_limits (
    provider_id TEXT NOT NULL,
    window_name TEXT NOT NULL,
    used_percent REAL NOT NULL,
    window_minutes INTEGER,
    resets_at_ms INTEGER,
    observed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (provider_id, window_name)
);

CREATE TABLE IF NOT EXISTS app_preferences (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    currency TEXT NOT NULL,
    usd_to_cny_rate TEXT NOT NULL,
    theme TEXT NOT NULL,
    minimize_to_tray INTEGER NOT NULL,
    launch_at_startup INTEGER NOT NULL,
    otlp_enabled INTEGER NOT NULL,
    otlp_port INTEGER NOT NULL,
    device_display_name TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
"#;

/// Run database schema migrations and seed required baseline defaults idempotently.
pub fn run_migrations(conn: &mut Connection) -> CoreResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);",
    )?;

    let current_version: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;

    let now_ms = Utc::now().timestamp_millis();

    if current_version < 1 {
        let tx = conn.transaction()?;
        tx.execute_batch(SCHEMA_V1_SQL)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
            params![1i64, now_ms],
        )?;
        tx.commit()?;
    }

    seed_defaults(conn, now_ms)?;
    Ok(())
}

fn seed_defaults(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    // 1. Ensure local device exists
    let device_count: i64 = tx.query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))?;
    if device_count == 0 {
        let device_id = uuid::Uuid::new_v4().to_string();
        let platform = std::env::consts::OS.to_string();
        let default_name = format!("{platform}-workstation");
        tx.execute(
            "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![device_id, default_name, platform, now_ms],
        )?;
    }

    // 2. Ensure default provider configs exist
    for provider_id in provider_ids::ALL {
        tx.execute(
            r#"
            INSERT OR IGNORE INTO provider_configs
                (provider_id, enabled, collection_mode, custom_log_paths_json, has_credential, last_sync_at_ms, last_error, updated_at_ms)
            VALUES (?1, 1, ?2, '[]', 0, NULL, NULL, ?3)
            "#,
            params![provider_id, CollectionMode::LocalLog.as_str(), now_ms],
        )?;
    }

    // 3. Seed built-in pricing catalog (preserves user overrides)
    for item in default_pricing_catalog() {
        tx.execute(
            r#"
            INSERT OR IGNORE INTO model_pricing
                (model_pattern, provider_id, input_per_m_micro_usd, output_per_m_micro_usd, cache_read_per_m_micro_usd, cache_write_per_m_micro_usd, reasoning_per_m_micro_usd, is_user_override, effective_from)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)
            "#,
            params![
                item.model_pattern,
                item.provider_id,
                item.input_per_m_micro_usd,
                item.output_per_m_micro_usd,
                item.cache_read_per_m_micro_usd,
                item.cache_write_per_m_micro_usd,
                item.reasoning_per_m_micro_usd,
                item.effective_from,
            ],
        )?;
    }

    // 4. Seed default subscription plans if none exist
    for plan in default_subscription_plans() {
        let windows_json = serde_json::to_string(&plan.windows)?;
        tx.execute(
            r#"
            INSERT OR IGNORE INTO subscription_plans
                (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, updated_at_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                plan.plan_id,
                plan.provider_id,
                plan.plan_name,
                i64::from(plan.enabled),
                plan.monthly_fee_micro_usd,
                windows_json,
                now_ms,
            ],
        )?;
    }

    // 5. Seed default preferences row (id = 1)
    let prefs = AppPreferences::default();
    tx.execute(
        r#"
        INSERT OR IGNORE INTO app_preferences
            (id, currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup, otlp_enabled, otlp_port, device_display_name, updated_at_ms)
        VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            prefs.currency.as_str(),
            prefs.usd_to_cny_rate,
            prefs.theme,
            i64::from(prefs.minimize_to_tray),
            i64::from(prefs.launch_at_startup),
            i64::from(prefs.otlp_enabled),
            i64::from(prefs.otlp_port),
            prefs.device_display_name,
            now_ms,
        ],
    )?;

    tx.commit()?;
    Ok(())
}
