use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::domain::{
    AppPreferences, CollectionMode, CurrencyCode, DeviceRecord, IngestionCursor,
    ModelPricingEntry, NormalizedUsageEvent, ObservedRateLimitWindow, ProviderConfig,
    SubscriptionPlan, SubscriptionWindowRule, SyncEnvelopeV1, SyncHourlyDelta,
    SyncSubscriptionSnapshot,
};
use crate::error::{CoreError, CoreResult};
use crate::pricing::apply_pricing_to_event;
use crate::storage::migrations::run_migrations;
use crate::subscription::{evaluate_subscription_plan, WindowUsageSample};

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 86_400_000;

/// Result counts from an atomic ingestion batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IngestOutcome {
    pub received: usize,
    pub inserted: usize,
    pub deduplicated: usize,
}

#[derive(Debug, Default)]
struct RollupDelta {
    request_count: i64,
    observable_step_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    reasoning_tokens: i64,
    estimated_cost_micro_usd: i64,
    cache_saved_micro_usd: i64,
    total_duration_ms: i64,
    tps_sample_count: i64,
    tps_sum: f64,
    max_output_tps: f64,
}

impl RollupDelta {
    fn add_event(&mut self, event: &NormalizedUsageEvent) {
        self.request_count += 1;
        if event.observable_only {
            self.observable_step_count += 1;
        }
        self.input_tokens += event.input_tokens;
        self.output_tokens += event.output_tokens;
        self.cache_read_tokens += event.cache_read_tokens;
        self.cache_write_tokens += event.cache_write_tokens;
        self.reasoning_tokens += event.reasoning_tokens;
        self.estimated_cost_micro_usd += event.estimated_cost_micro_usd;
        self.cache_saved_micro_usd += event.cache_saved_micro_usd;
        if let Some(dur) = event.duration_ms {
            if dur > 0 {
                self.total_duration_ms += dur;
            }
        }
        if let Some(tps) = event.output_tps {
            if tps.is_finite() && tps > 0.0 {
                self.tps_sample_count += 1;
                self.tps_sum += tps;
                if tps > self.max_output_tps {
                    self.max_output_tps = tps;
                }
            }
        }
    }
}

/// Thread-safe SQLite repository backing all core domain operations.
#[derive(Clone)]
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
}

impl Storage {
    /// Open (or create) a file-backed SQLite database with WAL mode and run migrations.
    pub fn open_file(path: &Path) -> CoreResult<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CoreError::Validation(format!(
                        "Failed to create database directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }
        }
        let mut conn = Connection::open(path)?;
        configure_connection(&conn, true)?;
        run_migrations(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an isolated in-memory SQLite database for tests and benchmarks.
    pub fn open_in_memory() -> CoreResult<Self> {
        let mut conn = Connection::open_in_memory()?;
        configure_connection(&conn, false)?;
        run_migrations(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub(crate) fn with_conn<F, T>(&self, f: F) -> CoreResult<T>
    where
        F: FnOnce(&Connection) -> CoreResult<T>,
    {
        let guard = self
            .conn
            .lock()
            .map_err(|_| CoreError::Validation("SQLite mutex poisoned".to_string()))?;
        f(&guard)
    }

    fn with_conn_mut<F, T>(&self, f: F) -> CoreResult<T>
    where
        F: FnOnce(&mut Connection) -> CoreResult<T>,
    {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| CoreError::Validation("SQLite mutex poisoned".to_string()))?;
        f(&mut guard)
    }

    pub fn local_device(&self) -> CoreResult<DeviceRecord> {
        self.with_conn(query_local_device)
    }

    pub fn get_cursor(&self, source_path: &str) -> CoreResult<Option<IngestionCursor>> {
        self.with_conn(|conn| {
            conn.query_row(
                r#"
                SELECT source_path, provider_id, file_identity, byte_offset, last_line_hash, updated_at_ms
                FROM ingestion_cursors
                WHERE source_path = ?1
                "#,
                params![source_path],
                |row| {
                    Ok(IngestionCursor {
                        source_path: row.get(0)?,
                        provider_id: row.get(1)?,
                        file_identity: row.get(2)?,
                        byte_offset: row.get::<_, i64>(3)?.max(0) as u64,
                        last_line_hash: row.get(4)?,
                        updated_at_ms: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(CoreError::from)
        })
    }

    pub fn list_cursors(&self) -> CoreResult<Vec<IngestionCursor>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT source_path, provider_id, file_identity, byte_offset, last_line_hash, updated_at_ms
                FROM ingestion_cursors
                ORDER BY updated_at_ms DESC
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(IngestionCursor {
                    source_path: row.get(0)?,
                    provider_id: row.get(1)?,
                    file_identity: row.get(2)?,
                    byte_offset: row.get::<_, i64>(3)?.max(0) as u64,
                    last_line_hash: row.get(4)?,
                    updated_at_ms: row.get(5)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn upsert_cursor(&self, cursor: &IngestionCursor) -> CoreResult<()> {
        self.with_conn(|conn| upsert_cursor_conn(conn, cursor))
    }

    /// Atomically insert deduplicated usage events, fold newly-inserted rows into
    /// hourly & daily rollups, optionally persist an ingestion cursor, and update
    /// any freshly observed official rate limit snapshots.
    pub fn ingest_events_with_cursor(
        &self,
        events: &[NormalizedUsageEvent],
        cursor: Option<&IngestionCursor>,
        rate_limits: &[ObservedRateLimitWindow],
    ) -> CoreResult<IngestOutcome> {
        self.with_conn_mut(|conn| {
            let now_ms = Utc::now().timestamp_millis();
            let catalog = load_pricing_catalog(conn)?;
            let local_dev = query_local_device(conn)?;

            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut inserted = 0usize;
            let mut hourly_deltas: HashMap<(String, String, String, i64), RollupDelta> =
                HashMap::new();
            let mut daily_deltas: HashMap<(String, String, String, i64), RollupDelta> =
                HashMap::new();
            let mut touched_providers: HashMap<String, ()> = HashMap::new();

            {
                let mut insert_stmt = tx.prepare_cached(
                    r#"
                    INSERT OR IGNORE INTO usage_events (
                        event_id, device_id, provider_id, session_id, project_label,
                        model_id, timestamp_ms, input_tokens, output_tokens,
                        cache_read_tokens, cache_write_tokens, reasoning_tokens,
                        observable_only, duration_ms, ttft_ms, output_tps,
                        estimated_cost_micro_usd, cache_saved_micro_usd
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5,
                        ?6, ?7, ?8, ?9,
                        ?10, ?11, ?12,
                        ?13, ?14, ?15, ?16,
                        ?17, ?18
                    )
                    "#,
                )?;

                for raw in events {
                    let mut ev = raw.clone();
                    if ev.device_id.is_empty() {
                        ev.device_id = local_dev.device_id.clone();
                    }
                    if ev.estimated_cost_micro_usd == 0 && !ev.observable_only {
                        apply_pricing_to_event(&mut ev, &catalog)?;
                    }

                    let changed = insert_stmt.execute(params![
                        ev.event_id,
                        ev.device_id,
                        ev.provider_id,
                        ev.session_id,
                        ev.project_label,
                        ev.model_id,
                        ev.timestamp_ms,
                        ev.input_tokens,
                        ev.output_tokens,
                        ev.cache_read_tokens,
                        ev.cache_write_tokens,
                        ev.reasoning_tokens,
                        i64::from(ev.observable_only),
                        ev.duration_ms,
                        ev.ttft_ms,
                        ev.output_tps,
                        ev.estimated_cost_micro_usd,
                        ev.cache_saved_micro_usd,
                    ])?;

                    if changed == 1 {
                        inserted += 1;
                        touched_providers.insert(ev.provider_id.clone(), ());

                        let hour_bucket = ev.timestamp_ms.div_euclid(HOUR_MS) * HOUR_MS;
                        hourly_deltas
                            .entry((
                                ev.device_id.clone(),
                                ev.provider_id.clone(),
                                ev.model_id.clone(),
                                hour_bucket,
                            ))
                            .or_default()
                            .add_event(&ev);

                        let day_bucket = ev.timestamp_ms.div_euclid(DAY_MS) * DAY_MS;
                        daily_deltas
                            .entry((
                                ev.device_id.clone(),
                                ev.provider_id.clone(),
                                ev.model_id.clone(),
                                day_bucket,
                            ))
                            .or_default()
                            .add_event(&ev);
                    }
                }
            }

            if !hourly_deltas.is_empty() {
                upsert_rollup_table(&tx, "usage_hourly_rollups", &hourly_deltas, now_ms)?;
            }
            if !daily_deltas.is_empty() {
                upsert_rollup_table(&tx, "usage_daily_rollups", &daily_deltas, now_ms)?;
            }

            if let Some(cur) = cursor {
                upsert_cursor_conn(&tx, cur)?;
                touched_providers.insert(cur.provider_id.clone(), ());
            }

            for obs in rate_limits {
                tx.execute(
                    r#"
                    INSERT INTO observed_rate_limits
                        (provider_id, window_name, used_percent, window_minutes, resets_at_ms, observed_at_ms)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    ON CONFLICT(provider_id, window_name) DO UPDATE SET
                        used_percent = excluded.used_percent,
                        window_minutes = excluded.window_minutes,
                        resets_at_ms = excluded.resets_at_ms,
                        observed_at_ms = excluded.observed_at_ms
                    WHERE excluded.observed_at_ms >= observed_rate_limits.observed_at_ms
                    "#,
                    params![
                        obs.provider_id,
                        obs.window_name,
                        obs.used_percent,
                        obs.window_minutes,
                        obs.resets_at_ms,
                        obs.observed_at_ms,
                    ],
                )?;
            }

            for provider_id in touched_providers.keys() {
                tx.execute(
                    r#"
                    UPDATE provider_configs
                    SET last_sync_at_ms = ?2, last_error = NULL, updated_at_ms = ?2
                    WHERE provider_id = ?1
                    "#,
                    params![provider_id, now_ms],
                )?;
            }

            tx.commit()?;
            Ok(IngestOutcome {
                received: events.len(),
                inserted,
                deduplicated: events.len().saturating_sub(inserted),
            })
        })
    }

    pub fn list_observed_rate_limits(&self) -> CoreResult<Vec<ObservedRateLimitWindow>> {
        self.with_conn(load_observed_rate_limits)
    }

    pub fn list_provider_configs(&self) -> CoreResult<Vec<ProviderConfig>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT provider_id, enabled, collection_mode, custom_log_paths_json,
                       has_credential, last_sync_at_ms, last_error, updated_at_ms
                FROM provider_configs
                ORDER BY provider_id ASC
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                let mode_str: String = row.get(2)?;
                let paths_json: String = row.get(3)?;
                let paths = serde_json::from_str::<Vec<String>>(&paths_json).unwrap_or_default();
                Ok(ProviderConfig {
                    provider_id: row.get(0)?,
                    enabled: row.get::<_, i64>(1)? != 0,
                    collection_mode: CollectionMode::parse(&mode_str),
                    custom_log_paths: paths,
                    has_credential: row.get::<_, i64>(4)? != 0,
                    last_sync_at_ms: row.get(5)?,
                    last_error: row.get(6)?,
                    updated_at_ms: row.get(7)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn upsert_provider_config(&self, config: &ProviderConfig) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        let paths_json = serde_json::to_string(&config.custom_log_paths)?;
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO provider_configs
                    (provider_id, enabled, collection_mode, custom_log_paths_json, has_credential, last_sync_at_ms, last_error, updated_at_ms)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(provider_id) DO UPDATE SET
                    enabled = excluded.enabled,
                    collection_mode = excluded.collection_mode,
                    custom_log_paths_json = excluded.custom_log_paths_json,
                    has_credential = excluded.has_credential,
                    last_sync_at_ms = COALESCE(excluded.last_sync_at_ms, provider_configs.last_sync_at_ms),
                    last_error = excluded.last_error,
                    updated_at_ms = excluded.updated_at_ms
                "#,
                params![
                    config.provider_id,
                    i64::from(config.enabled),
                    config.collection_mode.as_str(),
                    paths_json,
                    i64::from(config.has_credential),
                    config.last_sync_at_ms,
                    config.last_error,
                    now_ms,
                ],
            )?;
            Ok(())
        })
    }

    pub fn set_provider_sync_status(
        &self,
        provider_id: &str,
        last_sync_at_ms: Option<i64>,
        last_error: Option<&str>,
    ) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE provider_configs
                SET last_sync_at_ms = COALESCE(?2, last_sync_at_ms),
                    last_error = ?3,
                    updated_at_ms = ?4
                WHERE provider_id = ?1
                "#,
                params![provider_id, last_sync_at_ms, last_error, now_ms],
            )?;
            Ok(())
        })
    }

    pub fn set_provider_has_credential(
        &self,
        provider_id: &str,
        has_credential: bool,
    ) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE provider_configs
                SET has_credential = ?2, updated_at_ms = ?3
                WHERE provider_id = ?1
                "#,
                params![provider_id, i64::from(has_credential), now_ms],
            )?;
            Ok(())
        })
    }

    pub fn list_model_pricing(&self) -> CoreResult<Vec<ModelPricingEntry>> {
        self.with_conn(load_pricing_catalog)
    }

    pub fn upsert_model_pricing(&self, entry: &ModelPricingEntry) -> CoreResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO model_pricing (
                    model_pattern, provider_id, input_per_m_micro_usd, output_per_m_micro_usd,
                    cache_read_per_m_micro_usd, cache_write_per_m_micro_usd,
                    reasoning_per_m_micro_usd, is_user_override, effective_from
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(model_pattern) DO UPDATE SET
                    provider_id = excluded.provider_id,
                    input_per_m_micro_usd = excluded.input_per_m_micro_usd,
                    output_per_m_micro_usd = excluded.output_per_m_micro_usd,
                    cache_read_per_m_micro_usd = excluded.cache_read_per_m_micro_usd,
                    cache_write_per_m_micro_usd = excluded.cache_write_per_m_micro_usd,
                    reasoning_per_m_micro_usd = excluded.reasoning_per_m_micro_usd,
                    is_user_override = excluded.is_user_override,
                    effective_from = excluded.effective_from
                "#,
                params![
                    entry.model_pattern,
                    entry.provider_id,
                    entry.input_per_m_micro_usd,
                    entry.output_per_m_micro_usd,
                    entry.cache_read_per_m_micro_usd,
                    entry.cache_write_per_m_micro_usd,
                    entry.reasoning_per_m_micro_usd,
                    i64::from(entry.is_user_override),
                    entry.effective_from,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_model_pricing_override(&self, model_pattern: &str) -> CoreResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM model_pricing WHERE model_pattern = ?1 AND is_user_override = 1",
                params![model_pattern],
            )?;
            Ok(())
        })
    }

    /// Recompute historical costs for all usage events under the latest pricing table
    /// and rebuild hourly/daily rollups atomically.
    pub fn recompute_all_costs_and_rollups(&self) -> CoreResult<usize> {
        self.with_conn_mut(|conn| {
            let now_ms = Utc::now().timestamp_millis();
            let catalog = load_pricing_catalog(conn)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

            let mut updates: Vec<(String, i64, i64)> = Vec::new();
            {
                let mut stmt = tx.prepare(
                    r#"
                    SELECT event_id, provider_id, model_id,
                           input_tokens, output_tokens, cache_read_tokens,
                           cache_write_tokens, reasoning_tokens, observable_only,
                           estimated_cost_micro_usd, cache_saved_micro_usd
                    FROM usage_events
                    "#,
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)? != 0,
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                    ))
                })?;

                for row in rows {
                    let (
                        event_id,
                        provider_id,
                        model_id,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        reasoning_tokens,
                        observable_only,
                        old_cost,
                        old_saved,
                    ) = row?;

                    let mut ev = NormalizedUsageEvent {
                        event_id: event_id.clone(),
                        device_id: String::new(),
                        provider_id,
                        session_id: String::new(),
                        project_label: String::new(),
                        model_id,
                        timestamp_ms: 0,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        reasoning_tokens,
                        observable_only,
                        duration_ms: None,
                        ttft_ms: None,
                        output_tps: None,
                        estimated_cost_micro_usd: 0,
                        cache_saved_micro_usd: 0,
                    };
                    apply_pricing_to_event(&mut ev, &catalog)?;
                    if ev.estimated_cost_micro_usd != old_cost
                        || ev.cache_saved_micro_usd != old_saved
                    {
                        updates.push((
                            event_id,
                            ev.estimated_cost_micro_usd,
                            ev.cache_saved_micro_usd,
                        ));
                    }
                }
            }

            {
                let mut upd_stmt = tx.prepare(
                    r#"
                    UPDATE usage_events
                    SET estimated_cost_micro_usd = ?2, cache_saved_micro_usd = ?3
                    WHERE event_id = ?1
                    "#,
                )?;
                for (event_id, new_cost, new_saved) in &updates {
                    upd_stmt.execute(params![event_id, new_cost, new_saved])?;
                }
            }

            tx.execute("DELETE FROM usage_hourly_rollups", [])?;
            tx.execute("DELETE FROM usage_daily_rollups", [])?;

            rebuild_rollup_from_events(&tx, "usage_hourly_rollups", HOUR_MS, now_ms)?;
            rebuild_rollup_from_events(&tx, "usage_daily_rollups", DAY_MS, now_ms)?;

            tx.commit()?;
            Ok(updates.len())
        })
    }

    pub fn list_subscription_plans(&self) -> CoreResult<Vec<SubscriptionPlan>> {
        self.with_conn(load_subscription_plans)
    }

    pub fn upsert_subscription_plan(&self, plan: &SubscriptionPlan) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        let windows_json = serde_json::to_string(&plan.windows)?;
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO subscription_plans
                    (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, updated_at_ms)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(plan_id) DO UPDATE SET
                    provider_id = excluded.provider_id,
                    plan_name = excluded.plan_name,
                    enabled = excluded.enabled,
                    monthly_fee_micro_usd = excluded.monthly_fee_micro_usd,
                    windows_json = excluded.windows_json,
                    updated_at_ms = excluded.updated_at_ms
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
            Ok(())
        })
    }

    pub fn get_preferences(&self) -> CoreResult<AppPreferences> {
        self.with_conn(load_preferences)
    }

    pub fn update_preferences(&self, prefs: &AppPreferences) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        crate::pricing::parse_exchange_rate(&prefs.usd_to_cny_rate)?;
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE app_preferences
                SET currency = ?1,
                    usd_to_cny_rate = ?2,
                    theme = ?3,
                    minimize_to_tray = ?4,
                    launch_at_startup = ?5,
                    otlp_enabled = ?6,
                    otlp_port = ?7,
                    device_display_name = ?8,
                    updated_at_ms = ?9
                WHERE id = 1
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
            Ok(())
        })
    }

    /// Build a Phase 2 ready `SyncEnvelopeV1` containing only desensitized hourly deltas
    /// and subscription window percentages updated since `since_watermark_ms`.
    pub fn build_sync_envelope_v1(
        &self,
        since_watermark_ms: i64,
        now_ms: i64,
    ) -> CoreResult<SyncEnvelopeV1> {
        self.with_conn(|conn| {
            let device = query_local_device(conn)?;
            let prefs = load_preferences(conn)?;

            let mut stmt = conn.prepare(
                r#"
                SELECT provider_id, model_id, bucket_start_ms, request_count,
                       observable_step_count, input_tokens, output_tokens,
                       cache_read_tokens, cache_write_tokens, reasoning_tokens,
                       estimated_cost_micro_usd, cache_saved_micro_usd,
                       total_duration_ms, tps_sample_count, tps_sum, max_output_tps,
                       updated_at_ms
                FROM usage_hourly_rollups
                WHERE device_id = ?1 AND updated_at_ms > ?2
                ORDER BY bucket_start_ms ASC
                "#,
            )?;

            let rows = stmt.query_map(params![device.device_id, since_watermark_ms], |row| {
                Ok(SyncHourlyDelta {
                    provider_id: row.get(0)?,
                    model_id: row.get(1)?,
                    bucket_start_ms: row.get(2)?,
                    request_count: row.get(3)?,
                    observable_step_count: row.get(4)?,
                    input_tokens: row.get(5)?,
                    output_tokens: row.get(6)?,
                    cache_read_tokens: row.get(7)?,
                    cache_write_tokens: row.get(8)?,
                    reasoning_tokens: row.get(9)?,
                    estimated_cost_micro_usd: row.get(10)?,
                    cache_saved_micro_usd: row.get(11)?,
                    total_duration_ms: row.get(12)?,
                    tps_sample_count: row.get(13)?,
                    tps_sum: row.get(14)?,
                    max_output_tps: row.get(15)?,
                    updated_at_ms: row.get(16)?,
                })
            })?;

            let mut hourly_deltas = Vec::new();
            for row in rows {
                hourly_deltas.push(row?);
            }

            let plans = load_subscription_plans(conn)?;
            let observed = load_observed_rate_limits(conn)?;
            let samples_by_provider = load_window_samples_map(conn, now_ms)?;

            let mut subscription_snapshots = Vec::new();
            for plan in plans.iter().filter(|p| p.enabled) {
                let samples = samples_by_provider
                    .get(&plan.provider_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let eval = evaluate_subscription_plan(plan, samples, &observed, now_ms);
                for win in eval.windows {
                    subscription_snapshots.push(SyncSubscriptionSnapshot {
                        plan_id: plan.plan_id.clone(),
                        provider_id: plan.provider_id.clone(),
                        plan_name: plan.plan_name.clone(),
                        window_id: win.window_id,
                        window_label: win.label,
                        used_percent: win.used_percent,
                        resets_at_ms: win.resets_at_ms,
                        source: win.source,
                    });
                }
            }

            Ok(SyncEnvelopeV1 {
                schema_version: 1,
                device_id: device.device_id,
                device_display_name: prefs.device_display_name,
                generated_at_ms: now_ms,
                since_watermark_ms,
                hourly_deltas,
                subscription_snapshots,
            })
        })
    }
}

fn configure_connection(conn: &Connection, enable_wal: bool) -> CoreResult<()> {
    if enable_wal {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
    }
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000i64)?;
    Ok(())
}

fn query_local_device(conn: &Connection) -> CoreResult<DeviceRecord> {
    conn.query_row(
        "SELECT device_id, device_name, platform, created_at_ms FROM devices ORDER BY created_at_ms ASC LIMIT 1",
        [],
        |row| {
            Ok(DeviceRecord {
                device_id: row.get(0)?,
                device_name: row.get(1)?,
                platform: row.get(2)?,
                created_at_ms: row.get(3)?,
            })
        },
    )
    .map_err(CoreError::from)
}

fn upsert_cursor_conn(conn: &Connection, cursor: &IngestionCursor) -> CoreResult<()> {
    conn.execute(
        r#"
        INSERT INTO ingestion_cursors
            (source_path, provider_id, file_identity, byte_offset, last_line_hash, updated_at_ms)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(source_path) DO UPDATE SET
            provider_id = excluded.provider_id,
            file_identity = excluded.file_identity,
            byte_offset = excluded.byte_offset,
            last_line_hash = excluded.last_line_hash,
            updated_at_ms = excluded.updated_at_ms
        "#,
        params![
            cursor.source_path,
            cursor.provider_id,
            cursor.file_identity,
            cursor.byte_offset as i64,
            cursor.last_line_hash,
            cursor.updated_at_ms,
        ],
    )?;
    Ok(())
}

fn upsert_rollup_table(
    conn: &Connection,
    table_name: &str,
    deltas: &HashMap<(String, String, String, i64), RollupDelta>,
    updated_at_ms: i64,
) -> CoreResult<()> {
    let sql = format!(
        r#"
        INSERT INTO {table_name} (
            device_id, provider_id, model_id, bucket_start_ms,
            request_count, observable_step_count, input_tokens, output_tokens,
            cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_micro_usd, cache_saved_micro_usd,
            total_duration_ms, tps_sample_count, tps_sum, max_output_tps, updated_at_ms
        ) VALUES (
            ?1, ?2, ?3, ?4,
            ?5, ?6, ?7, ?8,
            ?9, ?10, ?11,
            ?12, ?13,
            ?14, ?15, ?16, ?17, ?18
        )
        ON CONFLICT(device_id, provider_id, model_id, bucket_start_ms) DO UPDATE SET
            request_count = {table_name}.request_count + excluded.request_count,
            observable_step_count = {table_name}.observable_step_count + excluded.observable_step_count,
            input_tokens = {table_name}.input_tokens + excluded.input_tokens,
            output_tokens = {table_name}.output_tokens + excluded.output_tokens,
            cache_read_tokens = {table_name}.cache_read_tokens + excluded.cache_read_tokens,
            cache_write_tokens = {table_name}.cache_write_tokens + excluded.cache_write_tokens,
            reasoning_tokens = {table_name}.reasoning_tokens + excluded.reasoning_tokens,
            estimated_cost_micro_usd = {table_name}.estimated_cost_micro_usd + excluded.estimated_cost_micro_usd,
            cache_saved_micro_usd = {table_name}.cache_saved_micro_usd + excluded.cache_saved_micro_usd,
            total_duration_ms = {table_name}.total_duration_ms + excluded.total_duration_ms,
            tps_sample_count = {table_name}.tps_sample_count + excluded.tps_sample_count,
            tps_sum = {table_name}.tps_sum + excluded.tps_sum,
            max_output_tps = MAX({table_name}.max_output_tps, excluded.max_output_tps),
            updated_at_ms = excluded.updated_at_ms
        "#
    );

    let mut stmt = conn.prepare_cached(&sql)?;
    for ((device_id, provider_id, model_id, bucket_start_ms), d) in deltas {
        stmt.execute(params![
            device_id,
            provider_id,
            model_id,
            bucket_start_ms,
            d.request_count,
            d.observable_step_count,
            d.input_tokens,
            d.output_tokens,
            d.cache_read_tokens,
            d.cache_write_tokens,
            d.reasoning_tokens,
            d.estimated_cost_micro_usd,
            d.cache_saved_micro_usd,
            d.total_duration_ms,
            d.tps_sample_count,
            d.tps_sum,
            d.max_output_tps,
            updated_at_ms,
        ])?;
    }
    Ok(())
}

fn rebuild_rollup_from_events(
    conn: &Connection,
    table_name: &str,
    bucket_ms: i64,
    updated_at_ms: i64,
) -> CoreResult<()> {
    // Integer floor division for positive Unix timestamps: (timestamp_ms / bucket_ms) * bucket_ms.
    let sql = format!(
        r#"
        INSERT INTO {table_name} (
            device_id, provider_id, model_id, bucket_start_ms,
            request_count, observable_step_count, input_tokens, output_tokens,
            cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_micro_usd, cache_saved_micro_usd,
            total_duration_ms, tps_sample_count, tps_sum, max_output_tps, updated_at_ms
        )
        SELECT
            device_id,
            provider_id,
            model_id,
            (timestamp_ms / ?1) * ?1 AS bucket_start_ms,
            COUNT(*) AS request_count,
            SUM(observable_only) AS observable_step_count,
            SUM(input_tokens),
            SUM(output_tokens),
            SUM(cache_read_tokens),
            SUM(cache_write_tokens),
            SUM(reasoning_tokens),
            SUM(estimated_cost_micro_usd),
            SUM(cache_saved_micro_usd),
            SUM(COALESCE(CASE WHEN duration_ms > 0 THEN duration_ms ELSE 0 END, 0)),
            SUM(CASE WHEN output_tps IS NOT NULL AND output_tps > 0 THEN 1 ELSE 0 END),
            SUM(COALESCE(CASE WHEN output_tps > 0 THEN output_tps ELSE 0.0 END, 0.0)),
            MAX(COALESCE(CASE WHEN output_tps > 0 THEN output_tps ELSE 0.0 END, 0.0)),
            ?2
        FROM usage_events
        GROUP BY device_id, provider_id, model_id, (timestamp_ms / ?1) * ?1
        "#
    );
    conn.execute(&sql, params![bucket_ms, updated_at_ms])?;
    Ok(())
}

pub(crate) fn load_pricing_catalog(conn: &Connection) -> CoreResult<Vec<ModelPricingEntry>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT model_pattern, provider_id, input_per_m_micro_usd, output_per_m_micro_usd,
               cache_read_per_m_micro_usd, cache_write_per_m_micro_usd,
               reasoning_per_m_micro_usd, is_user_override, effective_from
        FROM model_pricing
        ORDER BY is_user_override DESC, provider_id ASC, model_pattern ASC
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ModelPricingEntry {
            model_pattern: row.get(0)?,
            provider_id: row.get(1)?,
            input_per_m_micro_usd: row.get(2)?,
            output_per_m_micro_usd: row.get(3)?,
            cache_read_per_m_micro_usd: row.get(4)?,
            cache_write_per_m_micro_usd: row.get(5)?,
            reasoning_per_m_micro_usd: row.get(6)?,
            is_user_override: row.get::<_, i64>(7)? != 0,
            effective_from: row.get(8)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(crate) fn load_subscription_plans(conn: &Connection) -> CoreResult<Vec<SubscriptionPlan>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, updated_at_ms
        FROM subscription_plans
        ORDER BY provider_id ASC, monthly_fee_micro_usd ASC
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        let windows_json: String = row.get(5)?;
        let windows =
            serde_json::from_str::<Vec<SubscriptionWindowRule>>(&windows_json).unwrap_or_default();
        Ok(SubscriptionPlan {
            plan_id: row.get(0)?,
            provider_id: row.get(1)?,
            plan_name: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            monthly_fee_micro_usd: row.get(4)?,
            windows,
            updated_at_ms: row.get(6)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(crate) fn load_observed_rate_limits(
    conn: &Connection,
) -> CoreResult<Vec<ObservedRateLimitWindow>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT provider_id, window_name, used_percent, window_minutes, resets_at_ms, observed_at_ms
        FROM observed_rate_limits
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ObservedRateLimitWindow {
            provider_id: row.get(0)?,
            window_name: row.get(1)?,
            used_percent: row.get(2)?,
            window_minutes: row.get(3)?,
            resets_at_ms: row.get(4)?,
            observed_at_ms: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(crate) fn load_preferences(conn: &Connection) -> CoreResult<AppPreferences> {
    conn.query_row(
        r#"
        SELECT currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup,
               otlp_enabled, otlp_port, device_display_name, updated_at_ms
        FROM app_preferences
        WHERE id = 1
        "#,
        [],
        |row| {
            let cur_str: String = row.get(0)?;
            Ok(AppPreferences {
                currency: CurrencyCode::parse(&cur_str),
                usd_to_cny_rate: row.get(1)?,
                theme: row.get(2)?,
                minimize_to_tray: row.get::<_, i64>(3)? != 0,
                launch_at_startup: row.get::<_, i64>(4)? != 0,
                otlp_enabled: row.get::<_, i64>(5)? != 0,
                otlp_port: row.get::<_, i64>(6)?.clamp(1, 65535) as u16,
                device_display_name: row.get(7)?,
                updated_at_ms: row.get(8)?,
            })
        },
    )
    .map_err(CoreError::from)
}

pub(crate) fn load_window_samples_map(
    conn: &Connection,
    now_ms: i64,
) -> CoreResult<HashMap<String, Vec<WindowUsageSample>>> {
    // Load up to 31 days of event samples for evaluating 5h / 7d / 30d windows accurately.
    let lookback_start = now_ms.saturating_sub(31 * DAY_MS);
    let mut stmt = conn.prepare(
        r#"
        SELECT provider_id, timestamp_ms, estimated_cost_micro_usd,
               (input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens) AS total_tokens
        FROM usage_events
        WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2
        "#,
    )?;
    let rows = stmt.query_map(params![lookback_start, now_ms], |row| {
        Ok((
            row.get::<_, String>(0)?,
            WindowUsageSample {
                timestamp_ms: row.get(1)?,
                cost_micro_usd: row.get(2)?,
                total_tokens: row.get(3)?,
            },
        ))
    })?;
    let mut map: HashMap<String, Vec<WindowUsageSample>> = HashMap::new();
    for row in rows {
        let (provider_id, sample) = row?;
        map.entry(provider_id).or_default().push(sample);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::compute_event_id;
    use tempfile::tempdir;

    #[test]
    fn wal_mode_and_device_bootstrap_work_on_file_db() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("tokentracker.db");
        let storage = Storage::open_file(&db_path).expect("open db");

        let device = storage.local_device().expect("device");
        assert!(!device.device_id.is_empty());

        storage
            .with_conn(|conn| {
                let mode: String =
                    conn.query_row("PRAGMA journal_mode;", [], |row| row.get(0))?;
                assert_eq!(mode.to_ascii_lowercase(), "wal");
                Ok(())
            })
            .expect("wal check");
    }

    #[test]
    fn ingestion_is_strictly_idempotent_across_duplicate_replays() {
        let storage = Storage::open_in_memory().expect("in-memory db");
        let device = storage.local_device().expect("device");
        let ts = 1_760_000_000_000i64;

        let ev = NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "sess-1", "msg-1", ts),
            device_id: device.device_id.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "sess-1".to_string(),
            project_label: "TokenTracker".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: ts,
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(2_000),
            ttft_ms: Some(350),
            output_tps: Some(250.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        };

        let first = storage
            .ingest_events_with_cursor(std::slice::from_ref(&ev), None, &[])
            .expect("first ingest");
        assert_eq!(first.inserted, 1);
        assert_eq!(first.deduplicated, 0);

        let second = storage
            .ingest_events_with_cursor(&[ev], None, &[])
            .expect("second ingest");
        assert_eq!(second.inserted, 0);
        assert_eq!(second.deduplicated, 1);

        storage
            .with_conn(|conn| {
                let (req_count, cost): (i64, i64) = conn.query_row(
                    "SELECT SUM(request_count), SUM(estimated_cost_micro_usd) FROM usage_hourly_rollups",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                assert_eq!(req_count, 1);
                // 1M input ($3) + 0.5M output ($7.5) = $10.50 = 10_500_000 micro-USD
                assert_eq!(cost, 10_500_000);
                Ok(())
            })
            .expect("rollup counts verified");
    }

    #[test]
    fn sync_envelope_v1_contains_only_desensitized_aggregates() {
        let storage = Storage::open_in_memory().expect("in-memory db");
        let device = storage.local_device().expect("device");
        let ts = 1_760_000_000_000i64;
        let ev = NormalizedUsageEvent {
            event_id: compute_event_id("codex", "secret-session", "step-1", ts),
            device_id: device.device_id,
            provider_id: "codex".to_string(),
            session_id: "secret-session".to_string(),
            project_label: "secret-project".to_string(),
            model_id: "gpt-5-codex".to_string(),
            timestamp_ms: ts,
            input_tokens: 2000,
            output_tokens: 500,
            cache_read_tokens: 1000,
            cache_write_tokens: 0,
            reasoning_tokens: 100,
            observable_only: false,
            duration_ms: Some(1500),
            ttft_ms: None,
            output_tps: Some(40.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        };
        storage
            .ingest_events_with_cursor(&[ev], None, &[])
            .expect("ingest");

        let env = storage
            .build_sync_envelope_v1(0, ts + 60_000)
            .expect("build envelope");
        assert_eq!(env.schema_version, 1);
        assert_eq!(env.hourly_deltas.len(), 1);
        let serialized = serde_json::to_string(&env).expect("serialize");
        assert!(!serialized.contains("secret-session"));
        assert!(!serialized.contains("secret-project"));
    }
}
