use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::domain::{
    is_valid_tps_sample, provider_ids, sanitize_session_title, AppPreferences, CollectionMode,
    CurrencyCode, CustomProtocolKind, CustomProviderRule, DeclarativeMappingSpec, DeviceRecord,
    IngestionCursor, ModelPricingEntry, NormalizedUsageEvent, ObservedRateLimitWindow,
    ProviderBalanceSnapshot, ProviderConfig, QuotaUnit, RemoteSubscriptionSnapshot,
    RemoteSyncApplyResult, SessionTitleRecord, SubscriptionPlan, SubscriptionWindowRule,
    SyncConfigState, SyncEnvelopeV1, SyncHourlyDelta, SyncSubscriptionSnapshot,
    MAX_VALID_OUTPUT_TPS, MIN_VALID_TPS_DURATION_MS, MIN_VALID_TPS_OUTPUT_TOKENS,
};
use crate::error::{CoreError, CoreResult};
use crate::pricing::{
    apply_pricing_to_event, compute_catalog_hash, default_pricing_catalog, parse_exchange_rate,
    resolve_pricing,
};
use crate::storage::migrations::run_migrations;
use crate::subscription::{
    evaluate_subscription_plan, subscription_plan_templates, WindowUsageSample,
};

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

    pub(crate) fn with_conn_mut<F, T>(&self, f: F) -> CoreResult<T>
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
        self.ingest_events_with_cursor_and_titles(events, cursor, rate_limits, &[])
    }

    /// Atomically ingest events (upgrading streamed partial chunks when final usage increases),
    /// fold deltas into rollups, persist cursor, rate limits, and sanitized session titles.
    pub fn ingest_events_with_cursor_and_titles(
        &self,
        events: &[NormalizedUsageEvent],
        cursor: Option<&IngestionCursor>,
        rate_limits: &[ObservedRateLimitWindow],
        session_titles: &[SessionTitleRecord],
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
            let mut recompute_hourly_buckets: BTreeSet<(String, String, String, i64)> =
                BTreeSet::new();
            let mut recompute_daily_buckets: BTreeSet<(String, String, String, i64)> =
                BTreeSet::new();
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
                let mut select_existing_stmt = tx.prepare_cached(
                    r#"
                    SELECT device_id, provider_id, model_id, timestamp_ms,
                           input_tokens, output_tokens, cache_read_tokens,
                           cache_write_tokens, reasoning_tokens, duration_ms
                    FROM usage_events
                    WHERE event_id = ?1
                    "#,
                )?;
                let mut update_existing_stmt = tx.prepare_cached(
                    r#"
                    UPDATE usage_events
                    SET model_id = ?2,
                        timestamp_ms = ?3,
                        input_tokens = ?4,
                        output_tokens = ?5,
                        cache_read_tokens = ?6,
                        cache_write_tokens = ?7,
                        reasoning_tokens = ?8,
                        observable_only = ?9,
                        duration_ms = ?10,
                        ttft_ms = ?11,
                        output_tps = ?12,
                        estimated_cost_micro_usd = ?13,
                        cache_saved_micro_usd = ?14
                    WHERE event_id = ?1
                    "#,
                )?;

                for raw in events {
                    let mut ev = raw.clone();
                    if ev.device_id.is_empty() {
                        ev.device_id = local_dev.device_id.clone();
                    }
                    if !is_valid_tps_sample(
                        ev.output_tokens + ev.reasoning_tokens,
                        ev.duration_ms,
                        ev.output_tps,
                    ) {
                        ev.output_tps = None;
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
                    } else if let Some((
                        old_dev,
                        old_pid,
                        old_model,
                        old_ts,
                        old_in,
                        old_out,
                        old_cr,
                        old_cw,
                        old_reas,
                        old_dur,
                    )) = select_existing_stmt
                        .query_row(params![ev.event_id], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, i64>(3)?,
                                r.get::<_, i64>(4)?,
                                r.get::<_, i64>(5)?,
                                r.get::<_, i64>(6)?,
                                r.get::<_, i64>(7)?,
                                r.get::<_, i64>(8)?,
                                r.get::<_, Option<i64>>(9)?,
                            ))
                        })
                        .optional()?
                    {
                        let old_total = old_in + old_out + old_cr + old_cw + old_reas;
                        let new_total = ev.input_tokens
                            + ev.output_tokens
                            + ev.cache_read_tokens
                            + ev.cache_write_tokens
                            + ev.reasoning_tokens;
                        let should_upgrade = new_total > old_total
                            || (new_total == old_total && ev.output_tokens > old_out)
                            || (new_total == old_total
                                && ev.duration_ms.is_some()
                                && ev.duration_ms.unwrap_or(0) > old_dur.unwrap_or(0));

                        if should_upgrade {
                            if !ev.observable_only {
                                apply_pricing_to_event(&mut ev, &catalog)?;
                            }
                            update_existing_stmt.execute(params![
                                ev.event_id,
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
                            touched_providers.insert(ev.provider_id.clone(), ());

                            recompute_hourly_buckets.insert((
                                old_dev.clone(),
                                old_pid.clone(),
                                old_model.clone(),
                                old_ts.div_euclid(HOUR_MS) * HOUR_MS,
                            ));
                            recompute_hourly_buckets.insert((
                                ev.device_id.clone(),
                                ev.provider_id.clone(),
                                ev.model_id.clone(),
                                ev.timestamp_ms.div_euclid(HOUR_MS) * HOUR_MS,
                            ));
                            recompute_daily_buckets.insert((
                                old_dev,
                                old_pid,
                                old_model,
                                old_ts.div_euclid(DAY_MS) * DAY_MS,
                            ));
                            recompute_daily_buckets.insert((
                                ev.device_id.clone(),
                                ev.provider_id.clone(),
                                ev.model_id.clone(),
                                ev.timestamp_ms.div_euclid(DAY_MS) * DAY_MS,
                            ));
                        }
                    }
                }
            }

            if !hourly_deltas.is_empty() {
                upsert_rollup_table(&tx, "usage_hourly_rollups", &hourly_deltas, now_ms)?;
            }
            if !daily_deltas.is_empty() {
                upsert_rollup_table(&tx, "usage_daily_rollups", &daily_deltas, now_ms)?;
            }
            for (dev_id, pid, model_id, bucket_start) in &recompute_hourly_buckets {
                recompute_single_rollup_bucket(
                    &tx,
                    "usage_hourly_rollups",
                    HOUR_MS,
                    dev_id,
                    pid,
                    model_id,
                    *bucket_start,
                    now_ms,
                )?;
            }
            for (dev_id, pid, model_id, bucket_start) in &recompute_daily_buckets {
                recompute_single_rollup_bucket(
                    &tx,
                    "usage_daily_rollups",
                    DAY_MS,
                    dev_id,
                    pid,
                    model_id,
                    *bucket_start,
                    now_ms,
                )?;
            }

            if !session_titles.is_empty() {
                let mut title_stmt = tx.prepare_cached(
                    r#"
                    INSERT INTO session_titles (session_key, provider_id, title, source, updated_at_ms)
                    VALUES (?1, ?2, ?3, ?4, ?5)
                    ON CONFLICT(session_key) DO UPDATE SET
                        title = excluded.title,
                        source = excluded.source,
                        updated_at_ms = excluded.updated_at_ms
                    WHERE excluded.source = 'explicit'
                       OR (
                           session_titles.source = 'derived'
                           AND excluded.source = 'user_prompt'
                       )
                       OR (
                           session_titles.source = excluded.source
                           AND session_titles.source != 'explicit'
                           AND (
                               excluded.updated_at_ms < session_titles.updated_at_ms
                               OR LENGTH(TRIM(session_titles.title)) = 0
                           )
                       )
                    "#,
                )?;
                for rec in session_titles {
                    if let Some(clean_title) = sanitize_session_title(&rec.title) {
                        let source = if rec.source == "explicit" {
                            "explicit"
                        } else {
                            "derived"
                        };
                        title_stmt.execute(params![
                            rec.session_key,
                            rec.provider_id,
                            clean_title,
                            source,
                            rec.updated_at_ms,
                        ])?;
                    }
                }
            }

            if let Some(cur) = cursor {
                upsert_cursor_conn(&tx, cur)?;
                touched_providers.insert(cur.provider_id.clone(), ());
            }

            let mut rate_limit_providers: BTreeSet<String> = BTreeSet::new();
            for obs in rate_limits {
                rate_limit_providers.insert(obs.provider_id.clone());
                tx.execute(
                    r#"
                    INSERT INTO observed_rate_limits
                        (provider_id, window_name, used_percent, window_minutes, resets_at_ms, observed_at_ms, source, plan_hint)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    ON CONFLICT(provider_id, window_name) DO UPDATE SET
                        used_percent = excluded.used_percent,
                        window_minutes = excluded.window_minutes,
                        resets_at_ms = excluded.resets_at_ms,
                        observed_at_ms = excluded.observed_at_ms,
                        source = COALESCE(excluded.source, observed_rate_limits.source),
                        plan_hint = COALESCE(excluded.plan_hint, observed_rate_limits.plan_hint)
                    WHERE excluded.observed_at_ms >= observed_rate_limits.observed_at_ms
                    "#,
                    params![
                        obs.provider_id,
                        obs.window_name,
                        obs.used_percent,
                        obs.window_minutes,
                        obs.resets_at_ms,
                        obs.observed_at_ms,
                        obs.source,
                        obs.plan_hint,
                    ],
                )?;
            }

            if touched_providers.contains_key(provider_ids::ANTIGRAVITY) {
                rate_limit_providers.insert(provider_ids::ANTIGRAVITY.to_string());
            }

            for pid in rate_limit_providers {
                sync_auto_subscription_for_provider_conn(&tx, &pid, now_ms)?;
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
        self.with_conn(|conn| {
            Ok(load_observed_rate_limits(conn)?
                .into_iter()
                .filter(|o| {
                    o.source.as_deref() != Some("local_log_hint") && o.window_name != "tier_hint"
                })
                .collect())
        })
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
        self.with_conn_mut(recompute_all_costs_and_rollups_with_conn)
    }

    pub fn get_pricing_catalog_hash(&self) -> CoreResult<Option<String>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT pricing_catalog_hash FROM app_preferences WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .map_err(CoreError::from)
        })
    }

    /// Persist an automatically fetched USD->CNY exchange rate when `exchange_rate_auto` is enabled.
    /// Returns `true` when `usd_to_cny_rate` changed.
    pub fn apply_synced_exchange_rate(
        &self,
        formatted_rate: &str,
        synced_at_ms: i64,
    ) -> CoreResult<bool> {
        parse_exchange_rate(formatted_rate)?;
        self.with_conn(|conn| {
            let (current_rate, auto_enabled): (String, bool) = conn.query_row(
                "SELECT usd_to_cny_rate, exchange_rate_auto FROM app_preferences WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)),
            )?;
            if !auto_enabled {
                return Ok(false);
            }
            conn.execute(
                r#"
                UPDATE app_preferences
                SET usd_to_cny_rate = ?1,
                    last_rate_sync_at_ms = ?2,
                    updated_at_ms = ?2
                WHERE id = 1 AND exchange_rate_auto = 1
                "#,
                params![formatted_rate, synced_at_ms],
            )?;
            Ok(current_rate.trim() != formatted_rate.trim())
        })
    }

    /// Merge and upsert a remote public pricing catalog while preserving user overrides (`is_user_override = 1`)
    /// and built-in `_default_*` fallbacks. Only recomputes historical event costs & rollups when
    /// resolved pricing for models actually present in `usage_events` changes.
    /// Returns `(pricing_updated, synced_models_count, recomputed_events_count)`.
    pub fn apply_synced_pricing_catalog(
        &self,
        remote_entries: &[ModelPricingEntry],
        synced_at_ms: i64,
    ) -> CoreResult<(bool, usize, usize)> {
        let mut merged_map: BTreeMap<String, ModelPricingEntry> = BTreeMap::new();
        for item in default_pricing_catalog() {
            merged_map.insert(item.model_pattern.to_ascii_lowercase(), item);
        }
        for item in remote_entries {
            let normalized = item.model_pattern.trim().to_ascii_lowercase();
            if normalized.is_empty() || normalized.starts_with("_default_") {
                continue;
            }
            let mut sanitized = item.clone();
            sanitized.model_pattern = normalized.clone();
            sanitized.is_user_override = false;
            merged_map.insert(normalized, sanitized);
        }

        let merged: Vec<ModelPricingEntry> = merged_map.into_values().collect();
        let new_hash = compute_catalog_hash(&merged);
        let total_models = merged.len();

        self.with_conn_mut(|conn| {
            let existing_hash: Option<String> = conn.query_row(
                "SELECT pricing_catalog_hash FROM app_preferences WHERE id = 1",
                [],
                |row| row.get(0),
            )?;

            if existing_hash.as_deref() == Some(new_hash.as_str()) {
                conn.execute(
                    "UPDATE app_preferences SET last_pricing_sync_at_ms = ?1, updated_at_ms = ?1 WHERE id = 1",
                    params![synced_at_ms],
                )?;
                return Ok((false, total_models, 0));
            }

            let active_models = load_distinct_usage_models(conn)?;
            let before_catalog = load_pricing_catalog(conn)?;
            let before_rates: Vec<_> = active_models
                .iter()
                .map(|(prov, model)| resolve_pricing_rates(&before_catalog, prov, model))
                .collect();

            {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                {
                    let mut stmt = tx.prepare(
                        r#"
                        INSERT INTO model_pricing (
                            model_pattern, provider_id, input_per_m_micro_usd, output_per_m_micro_usd,
                            cache_read_per_m_micro_usd, cache_write_per_m_micro_usd,
                            reasoning_per_m_micro_usd, is_user_override, effective_from
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)
                        ON CONFLICT(model_pattern) DO UPDATE SET
                            provider_id = excluded.provider_id,
                            input_per_m_micro_usd = excluded.input_per_m_micro_usd,
                            output_per_m_micro_usd = excluded.output_per_m_micro_usd,
                            cache_read_per_m_micro_usd = excluded.cache_read_per_m_micro_usd,
                            cache_write_per_m_micro_usd = excluded.cache_write_per_m_micro_usd,
                            reasoning_per_m_micro_usd = excluded.reasoning_per_m_micro_usd,
                            effective_from = excluded.effective_from
                        WHERE model_pricing.is_user_override = 0
                        "#,
                    )?;

                    for entry in &merged {
                        stmt.execute(params![
                            entry.model_pattern,
                            entry.provider_id,
                            entry.input_per_m_micro_usd,
                            entry.output_per_m_micro_usd,
                            entry.cache_read_per_m_micro_usd,
                            entry.cache_write_per_m_micro_usd,
                            entry.reasoning_per_m_micro_usd,
                            entry.effective_from,
                        ])?;
                    }
                }

                tx.execute(
                    r#"
                    UPDATE app_preferences
                    SET last_pricing_sync_at_ms = ?1,
                        pricing_catalog_hash = ?2,
                        updated_at_ms = ?1
                    WHERE id = 1
                    "#,
                    params![synced_at_ms, new_hash],
                )?;
                tx.commit()?;
            }

            let recomputed = if !active_models.is_empty() {
                let after_catalog = load_pricing_catalog(conn)?;
                let active_changed = active_models
                    .iter()
                    .zip(before_rates)
                    .any(|((prov, model), before)| {
                        resolve_pricing_rates(&after_catalog, prov, model) != before
                    });
                if active_changed {
                    recompute_all_costs_and_rollups_with_conn(conn)?
                } else {
                    0
                }
            } else {
                0
            };

            Ok((true, total_models, recomputed))
        })
    }

    pub fn list_subscription_plans(&self) -> CoreResult<Vec<SubscriptionPlan>> {
        self.with_conn(load_subscription_plans)
    }

    pub fn upsert_subscription_plan(&self, plan: &SubscriptionPlan) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        let windows_json = serde_json::to_string(&plan.windows)?;
        let is_auto = plan.plan_id.starts_with("auto_") || plan.auto_source.is_some();
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if !is_auto {
                tx.execute(
                    r#"
                    DELETE FROM subscription_plans
                    WHERE provider_id = ?1
                      AND (plan_id LIKE 'auto_%' OR auto_source IS NOT NULL)
                      AND user_customized = 0
                    "#,
                    params![plan.provider_id],
                )?;
            }
            tx.execute(
                r#"
                INSERT INTO subscription_plans
                    (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, is_user_configured, auto_source, user_customized, user_dismissed, updated_at_ms)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, 0, ?9)
                ON CONFLICT(plan_id) DO UPDATE SET
                    provider_id = excluded.provider_id,
                    plan_name = excluded.plan_name,
                    enabled = excluded.enabled,
                    monthly_fee_micro_usd = excluded.monthly_fee_micro_usd,
                    windows_json = excluded.windows_json,
                    is_user_configured = 1,
                    auto_source = COALESCE(excluded.auto_source, subscription_plans.auto_source),
                    user_customized = 1,
                    user_dismissed = 0,
                    updated_at_ms = excluded.updated_at_ms
                "#,
                params![
                    plan.plan_id,
                    plan.provider_id,
                    plan.plan_name,
                    i64::from(plan.enabled),
                    plan.monthly_fee_micro_usd,
                    windows_json,
                    plan.auto_source,
                    i64::from(is_auto),
                    now_ms,
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn delete_subscription_plan(&self, plan_id: &str) -> CoreResult<()> {
        let trimmed = plan_id.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Validation(
                "Subscription plan_id cannot be empty".to_string(),
            ));
        }
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            let auto_source: Option<Option<String>> = conn
                .query_row(
                    "SELECT auto_source FROM subscription_plans WHERE plan_id = ?1",
                    params![trimmed],
                    |row| row.get(0),
                )
                .optional()?;

            if auto_source.flatten().is_some() || trimmed.starts_with("auto_") {
                conn.execute(
                    r#"
                    UPDATE subscription_plans
                    SET enabled = 0, is_user_configured = 0, user_dismissed = 1, updated_at_ms = ?2
                    WHERE plan_id = ?1
                    "#,
                    params![trimmed, now_ms],
                )?;
            } else {
                conn.execute(
                    "DELETE FROM subscription_plans WHERE plan_id = ?1",
                    params![trimmed],
                )?;
            }
            Ok(())
        })
    }

    pub fn reset_auto_subscription_dismissal(&self, provider_id: &str) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE subscription_plans
                SET user_dismissed = 0, updated_at_ms = ?2
                WHERE provider_id = ?1 AND (plan_id LIKE 'auto_%' OR auto_source IS NOT NULL)
                "#,
                params![provider_id, now_ms],
            )?;
            Ok(())
        })
    }

    pub fn clear_provider_api_artifacts(&self, provider_id: &str) -> CoreResult<()> {
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "DELETE FROM provider_balances WHERE provider_id = ?1",
                params![provider_id],
            )?;
            tx.execute(
                "DELETE FROM observed_rate_limits WHERE provider_id = ?1 AND source = 'official_api'",
                params![provider_id],
            )?;
            tx.execute(
                "DELETE FROM subscription_plans WHERE provider_id = ?1 AND auto_source = 'official_api' AND user_customized = 0",
                params![provider_id],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn get_preferences(&self) -> CoreResult<AppPreferences> {
        self.with_conn(load_preferences)
    }

    pub fn update_preferences(&self, prefs: &AppPreferences) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        crate::pricing::parse_exchange_rate(&prefs.usd_to_cny_rate)?;
        let bg_mode = prefs.bg_mode.trim();
        if !matches!(bg_mode, "none" | "image" | "gradient") {
            return Err(CoreError::Validation(format!(
                "Unsupported bg_mode '{}': expected none, image, or gradient",
                prefs.bg_mode
            )));
        }
        if !prefs.bg_opacity.is_finite() {
            return Err(CoreError::Validation(
                "bg_opacity must be a finite number".to_string(),
            ));
        }
        let bg_opacity = prefs.bg_opacity.clamp(0.0, 1.0);
        let bg_blur_px = prefs.bg_blur_px.clamp(0, 64);
        let bg_preset_id = if prefs.bg_preset_id.trim().is_empty() {
            "aurora_slate"
        } else {
            prefs.bg_preset_id.trim()
        };
        let bg_image_path = prefs
            .bg_image_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

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
                    bg_mode = ?9,
                    bg_image_path = ?10,
                    bg_preset_id = ?11,
                    bg_opacity = ?12,
                    bg_blur_px = ?13,
                    auto_sync_pricing = ?14,
                    exchange_rate_auto = ?15,
                    auto_check_updates = ?16,
                    floating_window_enabled = ?17,
                    updated_at_ms = ?18
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
                    bg_mode,
                    bg_image_path,
                    bg_preset_id,
                    bg_opacity,
                    bg_blur_px,
                    i64::from(prefs.auto_sync_pricing),
                    i64::from(prefs.exchange_rate_auto),
                    i64::from(prefs.auto_check_updates),
                    i64::from(prefs.floating_window_enabled),
                    now_ms,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_devices(&self) -> CoreResult<Vec<DeviceRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT device_id, device_name, platform, created_at_ms FROM devices ORDER BY created_at_ms ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(DeviceRecord {
                    device_id: row.get(0)?,
                    device_name: row.get(1)?,
                    platform: row.get(2)?,
                    created_at_ms: row.get(3)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    pub fn get_sync_config_state(&self) -> CoreResult<SyncConfigState> {
        self.with_conn(|conn| {
            conn.query_row(
                r#"
                SELECT enabled, worker_url, e2ee_enabled, e2ee_kdf_salt_hex,
                       last_push_watermark_ms, last_pull_watermark_ms,
                       last_sync_at_ms, last_sync_error, updated_at_ms
                FROM sync_state
                WHERE id = 1
                "#,
                [],
                |row| {
                    Ok(SyncConfigState {
                        enabled: row.get::<_, i64>(0)? != 0,
                        worker_url: row.get(1)?,
                        e2ee_enabled: row.get::<_, i64>(2)? != 0,
                        e2ee_kdf_salt_hex: row.get(3)?,
                        last_push_watermark_ms: row.get(4)?,
                        last_pull_watermark_ms: row.get(5)?,
                        last_sync_at_ms: row.get(6)?,
                        last_sync_error: row.get(7)?,
                        updated_at_ms: row.get(8)?,
                    })
                },
            )
            .map_err(CoreError::from)
        })
    }

    pub fn update_sync_config_state(
        &self,
        enabled: bool,
        worker_url: Option<&str>,
        e2ee_enabled: bool,
        e2ee_kdf_salt_hex: Option<&str>,
    ) -> CoreResult<SyncConfigState> {
        let now_ms = Utc::now().timestamp_millis();
        let cleaned_url = worker_url
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_end_matches('/').to_string());
        if enabled && cleaned_url.is_none() {
            return Err(CoreError::Validation(
                "Worker URL cannot be empty when cloud sync is enabled".to_string(),
            ));
        }
        let cleaned_salt = e2ee_kdf_salt_hex
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE sync_state
                SET enabled = ?1,
                    worker_url = ?2,
                    e2ee_enabled = ?3,
                    e2ee_kdf_salt_hex = COALESCE(?4, e2ee_kdf_salt_hex),
                    last_sync_error = NULL,
                    updated_at_ms = ?5
                WHERE id = 1
                "#,
                params![
                    i64::from(enabled),
                    cleaned_url,
                    i64::from(e2ee_enabled),
                    cleaned_salt,
                    now_ms,
                ],
            )?;
            Ok(())
        })?;
        self.get_sync_config_state()
    }

    pub fn record_sync_push_success(
        &self,
        push_watermark_ms: i64,
        synced_at_ms: i64,
    ) -> CoreResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE sync_state
                SET last_push_watermark_ms = MAX(last_push_watermark_ms, ?1),
                    last_sync_at_ms = ?2,
                    last_sync_error = NULL,
                    updated_at_ms = ?2
                WHERE id = 1
                "#,
                params![push_watermark_ms, synced_at_ms],
            )?;
            Ok(())
        })
    }

    pub fn record_sync_pull_success(
        &self,
        pull_watermark_ms: i64,
        synced_at_ms: i64,
    ) -> CoreResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE sync_state
                SET last_pull_watermark_ms = MAX(last_pull_watermark_ms, ?1),
                    last_sync_at_ms = ?2,
                    last_sync_error = NULL,
                    updated_at_ms = ?2
                WHERE id = 1
                "#,
                params![pull_watermark_ms, synced_at_ms],
            )?;
            Ok(())
        })
    }

    pub fn record_sync_error(&self, err_msg: Option<&str>, at_ms: i64) -> CoreResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE sync_state
                SET last_sync_error = ?1,
                    updated_at_ms = ?2
                WHERE id = 1
                "#,
                params![err_msg, at_ms],
            )?;
            Ok(())
        })
    }

    /// Idempotently merge a pulled remote `SyncEnvelopeV1` without altering local device buckets.
    pub fn apply_remote_sync_envelope_v1(
        &self,
        envelope: &SyncEnvelopeV1,
        received_at_ms: i64,
    ) -> CoreResult<RemoteSyncApplyResult> {
        if envelope.schema_version != 1 {
            return Err(CoreError::Validation(format!(
                "Unsupported SyncEnvelope schema_version {}",
                envelope.schema_version
            )));
        }
        let remote_device_id = envelope.device_id.trim();
        if remote_device_id.is_empty() {
            return Err(CoreError::Validation(
                "Remote SyncEnvelope device_id cannot be empty".to_string(),
            ));
        }

        self.with_conn_mut(|conn| {
            let local_dev = query_local_device(conn)?;
            if remote_device_id == local_dev.device_id {
                return Ok(RemoteSyncApplyResult {
                    device_id: remote_device_id.to_string(),
                    hourly_buckets_applied: 0,
                    subscription_snapshots_applied: 0,
                });
            }

            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut affected_days: BTreeSet<(String, i64)> = BTreeSet::new();
            let outcome = merge_remote_envelope_tx(
                &tx,
                &local_dev.device_id,
                envelope,
                received_at_ms,
                &mut affected_days,
            )?;
            rebuild_remote_daily_rollups_tx(&tx, &affected_days)?;
            tx.commit()?;
            Ok(outcome)
        })
    }

    pub fn list_remote_subscription_snapshots(
        &self,
    ) -> CoreResult<Vec<RemoteSubscriptionSnapshot>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT device_id, device_display_name, plan_id, window_id, provider_id,
                       plan_name, window_label, used_percent, resets_at_ms, source, updated_at_ms
                FROM remote_subscription_snapshots
                ORDER BY device_display_name ASC, provider_id ASC, window_id ASC
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(RemoteSubscriptionSnapshot {
                    device_id: row.get(0)?,
                    device_display_name: row.get(1)?,
                    plan_id: row.get(2)?,
                    window_id: row.get(3)?,
                    provider_id: row.get(4)?,
                    plan_name: row.get(5)?,
                    window_label: row.get(6)?,
                    used_percent: row.get(7)?,
                    resets_at_ms: row.get(8)?,
                    source: row.get(9)?,
                    updated_at_ms: row.get(10)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn list_provider_balances(&self) -> CoreResult<Vec<ProviderBalanceSnapshot>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT provider_id, currency, total_balance_micro, granted_balance_micro,
                       topped_up_balance_micro, daily_quota_used_micro, daily_quota_limit_micro,
                       fetched_at_ms
                FROM provider_balances
                ORDER BY provider_id ASC
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(ProviderBalanceSnapshot {
                    provider_id: row.get(0)?,
                    currency: row.get(1)?,
                    total_balance_micro: row.get(2)?,
                    granted_balance_micro: row.get(3)?,
                    topped_up_balance_micro: row.get(4)?,
                    daily_quota_used_micro: row.get(5)?,
                    daily_quota_limit_micro: row.get(6)?,
                    fetched_at_ms: row.get(7)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn get_provider_balance(
        &self,
        provider_id: &str,
    ) -> CoreResult<Option<ProviderBalanceSnapshot>> {
        self.with_conn(|conn| {
            conn.query_row(
                r#"
                SELECT provider_id, currency, total_balance_micro, granted_balance_micro,
                       topped_up_balance_micro, daily_quota_used_micro, daily_quota_limit_micro,
                       fetched_at_ms
                FROM provider_balances
                WHERE provider_id = ?1
                "#,
                params![provider_id],
                |row| {
                    Ok(ProviderBalanceSnapshot {
                        provider_id: row.get(0)?,
                        currency: row.get(1)?,
                        total_balance_micro: row.get(2)?,
                        granted_balance_micro: row.get(3)?,
                        topped_up_balance_micro: row.get(4)?,
                        daily_quota_used_micro: row.get(5)?,
                        daily_quota_limit_micro: row.get(6)?,
                        fetched_at_ms: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(CoreError::from)
        })
    }

    pub fn upsert_provider_balance(&self, balance: &ProviderBalanceSnapshot) -> CoreResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO provider_balances (
                    provider_id, currency, total_balance_micro, granted_balance_micro,
                    topped_up_balance_micro, daily_quota_used_micro, daily_quota_limit_micro,
                    fetched_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(provider_id) DO UPDATE SET
                    currency = excluded.currency,
                    total_balance_micro = excluded.total_balance_micro,
                    granted_balance_micro = excluded.granted_balance_micro,
                    topped_up_balance_micro = excluded.topped_up_balance_micro,
                    daily_quota_used_micro = excluded.daily_quota_used_micro,
                    daily_quota_limit_micro = excluded.daily_quota_limit_micro,
                    fetched_at_ms = excluded.fetched_at_ms
                "#,
                params![
                    balance.provider_id,
                    balance.currency,
                    balance.total_balance_micro,
                    balance.granted_balance_micro,
                    balance.topped_up_balance_micro,
                    balance.daily_quota_used_micro,
                    balance.daily_quota_limit_micro,
                    balance.fetched_at_ms,
                ],
            )?;
            Ok(())
        })
    }

    pub fn upsert_observed_rate_limits(
        &self,
        rate_limits: &[ObservedRateLimitWindow],
    ) -> CoreResult<()> {
        if rate_limits.is_empty() {
            return Ok(());
        }
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut rate_limit_providers: BTreeSet<String> = BTreeSet::new();
            for obs in rate_limits {
                rate_limit_providers.insert(obs.provider_id.clone());
                tx.execute(
                    r#"
                    INSERT INTO observed_rate_limits
                        (provider_id, window_name, used_percent, window_minutes, resets_at_ms, observed_at_ms, source, plan_hint)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    ON CONFLICT(provider_id, window_name) DO UPDATE SET
                        used_percent = excluded.used_percent,
                        window_minutes = excluded.window_minutes,
                        resets_at_ms = excluded.resets_at_ms,
                        observed_at_ms = excluded.observed_at_ms,
                        source = COALESCE(excluded.source, observed_rate_limits.source),
                        plan_hint = COALESCE(excluded.plan_hint, observed_rate_limits.plan_hint)
                    WHERE excluded.observed_at_ms >= observed_rate_limits.observed_at_ms
                    "#,
                    params![
                        obs.provider_id,
                        obs.window_name,
                        obs.used_percent,
                        obs.window_minutes,
                        obs.resets_at_ms,
                        obs.observed_at_ms,
                        obs.source,
                        obs.plan_hint,
                    ],
                )?;
            }

            for pid in rate_limit_providers {
                sync_auto_subscription_for_provider_conn(&tx, &pid, now_ms)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn list_custom_provider_rules(&self) -> CoreResult<Vec<CustomProviderRule>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT rule_id, display_name, enabled, protocol_kind, log_roots_json, mapping_spec_json, updated_at_ms
                FROM custom_provider_rules
                ORDER BY display_name ASC, rule_id ASC
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                let kind_str: String = row.get(3)?;
                let roots_json: String = row.get(4)?;
                let spec_json: String = row.get(5)?;
                let log_roots = serde_json::from_str::<Vec<String>>(&roots_json).unwrap_or_default();
                let mapping_spec =
                    serde_json::from_str::<DeclarativeMappingSpec>(&spec_json).unwrap_or_default();
                Ok(CustomProviderRule {
                    rule_id: row.get(0)?,
                    display_name: row.get(1)?,
                    enabled: row.get::<_, i64>(2)? != 0,
                    protocol_kind: CustomProtocolKind::parse(&kind_str),
                    log_roots,
                    mapping_spec,
                    updated_at_ms: row.get(6)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn upsert_custom_provider_rule(&self, rule: &CustomProviderRule) -> CoreResult<()> {
        let trimmed_id = rule.rule_id.trim();
        let trimmed_name = rule.display_name.trim();
        if trimmed_id.is_empty() {
            return Err(CoreError::Validation(
                "Custom provider rule_id cannot be empty".to_string(),
            ));
        }
        if trimmed_name.is_empty() {
            return Err(CoreError::Validation(
                "Custom provider display_name cannot be empty".to_string(),
            ));
        }
        let now_ms = Utc::now().timestamp_millis();
        let roots_json = serde_json::to_string(&rule.log_roots)?;
        let spec_json = serde_json::to_string(&rule.mapping_spec)?;
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO custom_provider_rules (
                    rule_id, display_name, enabled, protocol_kind, log_roots_json, mapping_spec_json, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(rule_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    enabled = excluded.enabled,
                    protocol_kind = excluded.protocol_kind,
                    log_roots_json = excluded.log_roots_json,
                    mapping_spec_json = excluded.mapping_spec_json,
                    updated_at_ms = excluded.updated_at_ms
                "#,
                params![
                    trimmed_id,
                    trimmed_name,
                    i64::from(rule.enabled),
                    rule.protocol_kind.as_str(),
                    roots_json,
                    spec_json,
                    now_ms,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_custom_provider_rule(&self, rule_id: &str) -> CoreResult<()> {
        let trimmed = rule_id.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Validation(
                "Custom provider rule_id cannot be empty".to_string(),
            ));
        }
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM custom_provider_rules WHERE rule_id = ?1",
                params![trimmed],
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
            for plan in plans.iter().filter(|p| p.enabled && p.is_user_configured) {
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

pub(crate) fn merge_remote_envelope_tx(
    tx: &rusqlite::Transaction<'_>,
    local_device_id: &str,
    envelope: &SyncEnvelopeV1,
    received_at_ms: i64,
    affected_days: &mut BTreeSet<(String, i64)>,
) -> CoreResult<RemoteSyncApplyResult> {
    if envelope.schema_version != 1 {
        return Err(CoreError::Validation(format!(
            "Unsupported SyncEnvelope schema_version {}",
            envelope.schema_version
        )));
    }
    let remote_device_id = envelope.device_id.trim();
    if remote_device_id.is_empty() {
        return Err(CoreError::Validation(
            "Remote SyncEnvelope device_id cannot be empty".to_string(),
        ));
    }
    if remote_device_id == local_device_id {
        return Ok(RemoteSyncApplyResult {
            device_id: remote_device_id.to_string(),
            hourly_buckets_applied: 0,
            subscription_snapshots_applied: 0,
        });
    }

    let display_name = if envelope.device_display_name.trim().is_empty() {
        remote_device_id.to_string()
    } else {
        envelope.device_display_name.trim().to_string()
    };

    tx.execute(
        r#"
        INSERT INTO devices (device_id, device_name, platform, created_at_ms)
        VALUES (?1, ?2, 'remote', ?3)
        ON CONFLICT(device_id) DO UPDATE SET
            device_name = excluded.device_name
        "#,
        params![remote_device_id, display_name, received_at_ms],
    )?;

    let mut hourly_applied = 0usize;
    {
        let mut hourly_stmt = tx.prepare_cached(
            r#"
            INSERT INTO usage_hourly_rollups (
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
                request_count = excluded.request_count,
                observable_step_count = excluded.observable_step_count,
                input_tokens = excluded.input_tokens,
                output_tokens = excluded.output_tokens,
                cache_read_tokens = excluded.cache_read_tokens,
                cache_write_tokens = excluded.cache_write_tokens,
                reasoning_tokens = excluded.reasoning_tokens,
                estimated_cost_micro_usd = excluded.estimated_cost_micro_usd,
                cache_saved_micro_usd = excluded.cache_saved_micro_usd,
                total_duration_ms = excluded.total_duration_ms,
                tps_sample_count = excluded.tps_sample_count,
                tps_sum = excluded.tps_sum,
                max_output_tps = excluded.max_output_tps,
                updated_at_ms = excluded.updated_at_ms
            WHERE excluded.updated_at_ms >= usage_hourly_rollups.updated_at_ms
            "#,
        )?;

        for delta in &envelope.hourly_deltas {
            let changed = hourly_stmt.execute(params![
                remote_device_id,
                delta.provider_id,
                delta.model_id,
                delta.bucket_start_ms,
                delta.request_count,
                delta.observable_step_count,
                delta.input_tokens,
                delta.output_tokens,
                delta.cache_read_tokens,
                delta.cache_write_tokens,
                delta.reasoning_tokens,
                delta.estimated_cost_micro_usd,
                delta.cache_saved_micro_usd,
                delta.total_duration_ms,
                delta.tps_sample_count,
                delta.tps_sum,
                delta.max_output_tps,
                delta.updated_at_ms,
            ])?;
            if changed > 0 {
                hourly_applied += 1;
            }
            affected_days.insert((
                remote_device_id.to_string(),
                delta.bucket_start_ms.div_euclid(DAY_MS) * DAY_MS,
            ));
        }
    }

    tx.execute(
        "DELETE FROM remote_subscription_snapshots WHERE device_id = ?1",
        params![remote_device_id],
    )?;
    let mut sub_stmt = tx.prepare_cached(
        r#"
        INSERT INTO remote_subscription_snapshots (
            device_id, device_display_name, plan_id, window_id, provider_id,
            plan_name, window_label, used_percent, resets_at_ms, source, updated_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )?;
    for snap in &envelope.subscription_snapshots {
        sub_stmt.execute(params![
            remote_device_id,
            display_name,
            snap.plan_id,
            snap.window_id,
            snap.provider_id,
            snap.plan_name,
            snap.window_label,
            snap.used_percent,
            snap.resets_at_ms,
            snap.source,
            envelope.generated_at_ms.max(received_at_ms),
        ])?;
    }

    Ok(RemoteSyncApplyResult {
        device_id: remote_device_id.to_string(),
        hourly_buckets_applied: hourly_applied,
        subscription_snapshots_applied: envelope.subscription_snapshots.len(),
    })
}

pub(crate) fn rebuild_remote_daily_rollups_tx(
    tx: &rusqlite::Transaction<'_>,
    affected_days: &BTreeSet<(String, i64)>,
) -> CoreResult<()> {
    for (remote_device_id, day_start_ms) in affected_days {
        let day_end_ms = day_start_ms.saturating_add(DAY_MS - 1);
        tx.execute(
            "DELETE FROM usage_daily_rollups WHERE device_id = ?1 AND bucket_start_ms = ?2",
            params![remote_device_id, day_start_ms],
        )?;
        tx.execute(
            r#"
            INSERT INTO usage_daily_rollups (
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
                ?2 AS bucket_start_ms,
                SUM(request_count),
                SUM(observable_step_count),
                SUM(input_tokens),
                SUM(output_tokens),
                SUM(cache_read_tokens),
                SUM(cache_write_tokens),
                SUM(reasoning_tokens),
                SUM(estimated_cost_micro_usd),
                SUM(cache_saved_micro_usd),
                SUM(total_duration_ms),
                SUM(tps_sample_count),
                SUM(tps_sum),
                MAX(max_output_tps),
                MAX(updated_at_ms)
            FROM usage_hourly_rollups
            WHERE device_id = ?1 AND bucket_start_ms >= ?2 AND bucket_start_ms <= ?3
            GROUP BY device_id, provider_id, model_id
            "#,
            params![remote_device_id, day_start_ms, day_end_ms],
        )?;
    }
    Ok(())
}

pub(crate) fn configure_connection(conn: &Connection, enable_wal: bool) -> CoreResult<()> {
    if enable_wal {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
    }
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000i64)?;
    Ok(())
}

pub(crate) fn query_local_device(conn: &Connection) -> CoreResult<DeviceRecord> {
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

type PricingRateTuple = (i64, i64, i64, i64, i64);

fn resolve_pricing_rates(
    catalog: &[ModelPricingEntry],
    provider_id: &str,
    model_id: &str,
) -> Option<PricingRateTuple> {
    resolve_pricing(catalog, provider_id, model_id).map(|entry| {
        (
            entry.input_per_m_micro_usd,
            entry.output_per_m_micro_usd,
            entry.cache_read_per_m_micro_usd,
            entry.cache_write_per_m_micro_usd,
            entry.reasoning_per_m_micro_usd,
        )
    })
}

fn load_distinct_usage_models(conn: &Connection) -> CoreResult<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT DISTINCT provider_id, model_id FROM usage_events")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn recompute_all_costs_and_rollups_with_conn(conn: &mut Connection) -> CoreResult<usize> {
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
            if ev.estimated_cost_micro_usd != old_cost || ev.cache_saved_micro_usd != old_saved {
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

    rebuild_local_rollups_conn(&tx, now_ms)?;

    tx.commit()?;
    Ok(updates.len())
}

pub(crate) fn rebuild_local_rollups_conn(conn: &Connection, updated_at_ms: i64) -> CoreResult<()> {
    conn.execute(
        "DELETE FROM usage_hourly_rollups WHERE device_id IN (SELECT DISTINCT device_id FROM usage_events)",
        [],
    )?;
    conn.execute(
        "DELETE FROM usage_daily_rollups WHERE device_id IN (SELECT DISTINCT device_id FROM usage_events)",
        [],
    )?;

    rebuild_rollup_from_events(conn, "usage_hourly_rollups", HOUR_MS, updated_at_ms)?;
    rebuild_rollup_from_events(conn, "usage_daily_rollups", DAY_MS, updated_at_ms)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn recompute_single_rollup_bucket(
    conn: &Connection,
    table_name: &str,
    bucket_ms: i64,
    device_id: &str,
    provider_id: &str,
    model_id: &str,
    bucket_start_ms: i64,
    updated_at_ms: i64,
) -> CoreResult<()> {
    let delete_sql = format!(
        "DELETE FROM {table_name} WHERE device_id = ?1 AND provider_id = ?2 AND model_id = ?3 AND bucket_start_ms = ?4"
    );
    conn.execute(
        &delete_sql,
        params![device_id, provider_id, model_id, bucket_start_ms],
    )?;

    let insert_sql = format!(
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
            ?4 AS bucket_start_ms,
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
            SUM(CASE WHEN {tps_cond} THEN 1 ELSE 0 END),
            SUM(CASE WHEN {tps_cond} THEN output_tps ELSE 0.0 END),
            MAX(CASE WHEN {tps_cond} THEN output_tps ELSE 0.0 END),
            ?6
        FROM usage_events
        WHERE device_id = ?1
          AND provider_id = ?2
          AND model_id = ?3
          AND timestamp_ms >= ?4
          AND timestamp_ms < ?5
        GROUP BY device_id, provider_id, model_id
        "#,
        tps_cond = "output_tps IS NOT NULL AND output_tps > 0.0 AND output_tps <= ?7 AND duration_ms IS NOT NULL AND duration_ms >= ?8 AND (output_tokens + reasoning_tokens) >= ?9"
    );
    conn.execute(
        &insert_sql,
        params![
            device_id,
            provider_id,
            model_id,
            bucket_start_ms,
            bucket_start_ms.saturating_add(bucket_ms),
            updated_at_ms,
            MAX_VALID_OUTPUT_TPS,
            MIN_VALID_TPS_DURATION_MS,
            MIN_VALID_TPS_OUTPUT_TOKENS,
        ],
    )?;
    Ok(())
}

pub(crate) fn rebuild_rollup_from_events(
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
            SUM(CASE WHEN {tps_cond} THEN 1 ELSE 0 END),
            SUM(CASE WHEN {tps_cond} THEN output_tps ELSE 0.0 END),
            MAX(CASE WHEN {tps_cond} THEN output_tps ELSE 0.0 END),
            ?2
        FROM usage_events
        GROUP BY device_id, provider_id, model_id, (timestamp_ms / ?1) * ?1
        "#,
        tps_cond = "output_tps IS NOT NULL AND output_tps > 0.0 AND output_tps <= ?3 AND duration_ms IS NOT NULL AND duration_ms >= ?4 AND (output_tokens + reasoning_tokens) >= ?5"
    );
    conn.execute(
        &sql,
        params![
            bucket_ms,
            updated_at_ms,
            MAX_VALID_OUTPUT_TPS,
            MIN_VALID_TPS_DURATION_MS,
            MIN_VALID_TPS_OUTPUT_TOKENS,
        ],
    )?;
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
        SELECT plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd,
               windows_json, is_user_configured, auto_source, updated_at_ms
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
            is_user_configured: row.get::<_, i64>(6)? != 0,
            auto_source: row.get(7)?,
            updated_at_ms: row.get(8)?,
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
        SELECT provider_id, window_name, used_percent, window_minutes, resets_at_ms,
               observed_at_ms, source, plan_hint
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
            source: row.get(6)?,
            plan_hint: row.get(7)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn infer_window_rule_from_observed(
    provider_id: &str,
    obs: &ObservedRateLimitWindow,
    template_windows: &[SubscriptionWindowRule],
    daily_quota_limit_micro: Option<i64>,
    balance_currency: Option<&str>,
) -> SubscriptionWindowRule {
    let name = obs.window_name.to_ascii_lowercase();
    let (window_id, default_label, duration_hours, rolling) = match obs.window_minutes {
        Some(m) if (240..=360).contains(&m) => ("5h".to_string(), "5h 滚动窗口".to_string(), 5, true),
        Some(m) if (1_200..=1_680).contains(&m) => {
            ("24h".to_string(), "24h 日额度".to_string(), 24, false)
        }
        Some(m) if (9_600..=10_560).contains(&m) => {
            ("7d".to_string(), "7d 周额度".to_string(), 168, true)
        }
        Some(m) if m >= 40_000 => ("30d".to_string(), "30d 月度限额".to_string(), 720, false),
        Some(m) => {
            let hours = (m / 60).max(1);
            (
                format!("{hours}h"),
                format!("{hours}h 窗口"),
                hours,
                hours <= 168,
            )
        }
        None if name.contains("primary") || name.contains("5h") => {
            ("5h".to_string(), "5h 滚动窗口".to_string(), 5, true)
        }
        None if name.contains("secondary") || name.contains("7d") || name.contains("week") => {
            ("7d".to_string(), "7d 周额度".to_string(), 168, true)
        }
        None if name.contains("30d") || name.contains("month") => {
            ("30d".to_string(), "30d 月度限额".to_string(), 720, false)
        }
        None if name.contains("daily") || name.contains("24h") || name.contains("day") => {
            ("24h".to_string(), "24h 日额度".to_string(), 24, false)
        }
        None => ("5h".to_string(), "5h 滚动窗口".to_string(), 5, true),
    };

    if provider_id == provider_ids::QCODE && window_id == "24h" {
        if let Some(limit) = daily_quota_limit_micro.filter(|&v| v > 0) {
            return SubscriptionWindowRule {
                window_id,
                label: default_label,
                duration_hours,
                rolling,
                unit: QuotaUnit::MicroUsd,
                limit_value: limit,
                warning_percent: 80,
                critical_percent: 95,
            };
        }
    }

    if let Some(tpl_win) = template_windows.iter().find(|w| w.window_id == window_id) {
        let mut cloned = tpl_win.clone();
        if provider_id == provider_ids::KIMI_CODE && balance_currency == Some("QUOTA") {
            cloned.label = match window_id.as_str() {
                "5h" => "5h 请求/频次窗口".to_string(),
                "7d" => "7d 周额度窗口".to_string(),
                "30d" => "30d 月度限额".to_string(),
                _ => cloned.label,
            };
        }
        return cloned;
    }

    SubscriptionWindowRule {
        window_id: window_id.clone(),
        label: default_label,
        duration_hours,
        rolling,
        unit: QuotaUnit::MicroUsd,
        limit_value: if window_id == "30d" {
            120_000_000
        } else {
            20_000_000
        },
        warning_percent: 80,
        critical_percent: 95,
    }
}

pub(crate) fn sync_auto_subscription_for_provider_conn(
    conn: &Connection,
    provider_id: &str,
    now_ms: i64,
) -> CoreResult<()> {
    let trimmed = provider_id.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    // Do not auto-seed if the user already maintains an active manually created plan for this provider.
    let has_manual_plan: Option<i64> = conn
        .query_row(
            r#"
            SELECT 1
            FROM subscription_plans
            WHERE provider_id = ?1
              AND is_user_configured = 1
              AND auto_source IS NULL
              AND plan_id NOT LIKE 'auto_%'
            LIMIT 1
            "#,
            params![trimmed],
            |row| row.get(0),
        )
        .optional()?;
    if has_manual_plan.is_some() {
        return Ok(());
    }

    let auto_plan_id = format!("auto_{trimmed}");
    let existing_flags: Option<(i64, i64)> = conn
        .query_row(
            "SELECT user_dismissed, user_customized FROM subscription_plans WHERE plan_id = ?1",
            params![auto_plan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((dismissed, customized)) = existing_flags {
        if dismissed != 0 || customized != 0 {
            return Ok(());
        }
    }

    let obs_rows: Vec<ObservedRateLimitWindow> = load_observed_rate_limits(conn)?
        .into_iter()
        .filter(|o| o.provider_id.eq_ignore_ascii_case(trimmed))
        .collect();
    if obs_rows.is_empty() {
        if trimmed == provider_ids::ANTIGRAVITY {
            let has_ag_events: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM usage_events WHERE provider_id = ?1 LIMIT 1",
                    params![provider_ids::ANTIGRAVITY],
                    |row| row.get(0),
                )
                .optional()?;
            if has_ag_events.is_none() {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    }

    let bal_info: Option<(String, Option<i64>)> = conn
        .query_row(
            "SELECT currency, daily_quota_limit_micro FROM provider_balances WHERE provider_id = ?1",
            params![trimmed],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let balance_currency = bal_info.as_ref().map(|(c, _)| c.as_str());
    let daily_quota_limit = bal_info.as_ref().and_then(|(_, l)| *l);

    let auto_source = if obs_rows
        .iter()
        .any(|o| o.source.as_deref() == Some("official_api"))
        || matches!(
            trimmed,
            provider_ids::KIMI_CODE | provider_ids::MINIMAX_CODE | provider_ids::QCODE
        ) {
        "official_api".to_string()
    } else if !obs_rows.is_empty()
        && obs_rows
            .iter()
            .any(|o| o.source.as_deref() != Some("local_log_hint"))
    {
        "official_log_snapshot".to_string()
    } else {
        "local_log".to_string()
    };

    let plan_hint = obs_rows
        .iter()
        .filter_map(|o| {
            o.plan_hint
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| (o.observed_at_ms, s.to_string()))
        })
        .max_by_key(|(ts, _)| *ts)
        .map(|(_, s)| s)
        .or_else(|| {
            if trimmed == provider_ids::ANTIGRAVITY {
                let mut stmt = conn
                    .prepare(
                        "SELECT DISTINCT LOWER(model_id) FROM usage_events WHERE provider_id = ?1",
                    )
                    .ok()?;
                let models: Vec<String> = stmt
                    .query_map(params![provider_ids::ANTIGRAVITY], |r| r.get::<_, String>(0))
                    .ok()?
                    .flatten()
                    .collect();
                if models.iter().any(|m| m.contains("ultra") && m.contains("20x")) {
                    Some("ultra_20x".to_string())
                } else if models.iter().any(|m| m.contains("ultra") && m.contains("5x")) {
                    Some("ultra_5x".to_string())
                } else if models.iter().any(|m| m.contains("ultra")) {
                    Some("ultra".to_string())
                } else if models.iter().any(|m| m.contains("enterprise")) {
                    Some("enterprise".to_string())
                } else {
                    Some("pro".to_string())
                }
            } else {
                None
            }
        });

    let hint_lower = plan_hint
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();

    let templates = subscription_plan_templates();
    let preferred_tpl = templates.iter().find(|tpl| {
        tpl.provider_id == trimmed
            && match trimmed {
                provider_ids::CODEX if hint_lower.contains("pro") => tpl.template_id == "codex_pro",
                provider_ids::CODEX if hint_lower.contains("team") => {
                    tpl.template_id == "codex_team"
                }
                provider_ids::CODEX => tpl.template_id == "codex_plus",
                provider_ids::ANTIGRAVITY
                    if hint_lower.contains("ultra")
                        && (hint_lower.contains("5x") || hint_lower.contains("99")) =>
                {
                    tpl.template_id == "google_ai_ultra_5x"
                }
                provider_ids::ANTIGRAVITY if hint_lower.contains("ultra") => {
                    tpl.template_id == "google_ai_ultra"
                }
                provider_ids::ANTIGRAVITY
                    if hint_lower.contains("enterprise") || hint_lower.contains("business") =>
                {
                    tpl.template_id == "gemini_enterprise"
                }
                provider_ids::ANTIGRAVITY if hint_lower.contains("plus") => {
                    tpl.template_id == "google_ai_plus"
                }
                provider_ids::ANTIGRAVITY if hint_lower.contains("free") => {
                    tpl.template_id == "google_ai_free"
                }
                provider_ids::ANTIGRAVITY => tpl.template_id == "google_ai_pro",
                provider_ids::KIMI_CODE
                    if matches!(
                        hint_lower.as_str(),
                        "level_max" | "vivace" | "ultra"
                    ) =>
                {
                    tpl.template_id == "kimi_code_ultra"
                }
                provider_ids::KIMI_CODE
                    if matches!(
                        hint_lower.as_str(),
                        "level_advanced" | "allegro" | "max"
                    ) =>
                {
                    tpl.template_id == "kimi_code_max"
                }
                provider_ids::KIMI_CODE
                    if matches!(
                        hint_lower.as_str(),
                        "level_basic" | "basic" | "moderato" | "plus"
                    ) =>
                {
                    tpl.template_id == "kimi_code_plus"
                }
                provider_ids::KIMI_CODE => tpl.template_id == "kimi_code_allegretto",
                _ => true,
            }
    });
    let tpl_windows: Vec<SubscriptionWindowRule> = preferred_tpl
        .map(|t| t.windows.clone())
        .unwrap_or_default();

    let real_obs_rows: Vec<&ObservedRateLimitWindow> = obs_rows
        .iter()
        .filter(|o| o.source.as_deref() != Some("local_log_hint"))
        .collect();
    let mut rules_by_id: BTreeMap<String, SubscriptionWindowRule> = BTreeMap::new();
    for obs in &real_obs_rows {
        let rule = infer_window_rule_from_observed(
            trimmed,
            obs,
            &tpl_windows,
            daily_quota_limit,
            balance_currency,
        );
        rules_by_id.entry(rule.window_id.clone()).or_insert(rule);
    }

    // Ensure Kimi Code plans always include the 30d monthly limit window alongside 5h/7d windows.
    if trimmed == provider_ids::KIMI_CODE && !rules_by_id.is_empty() {
        if let Some(monthly_rule) = tpl_windows.iter().find(|w| w.window_id == "30d") {
            rules_by_id
                .entry("30d".to_string())
                .or_insert_with(|| monthly_rule.clone());
        } else {
            rules_by_id
                .entry("30d".to_string())
                .or_insert(SubscriptionWindowRule {
                    window_id: "30d".to_string(),
                    label: "30d 月度限额".to_string(),
                    duration_hours: 720,
                    rolling: false,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 120_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                });
        }
    }

    let mut windows: Vec<SubscriptionWindowRule> = if rules_by_id.is_empty() {
        if matches!(
            trimmed,
            provider_ids::ANTIGRAVITY | provider_ids::KIMI_CODE
        ) {
            tpl_windows
        } else {
            Vec::new()
        }
    } else {
        rules_by_id.into_values().collect()
    };
    windows.sort_by_key(|w| w.duration_hours);
    if windows.is_empty() {
        return Ok(());
    }

    let (plan_name, monthly_fee_micro_usd) = match trimmed {
        provider_ids::CLAUDE_CODE => {
            if hint_lower.contains("max") && (hint_lower.contains("20") || hint_lower.contains("200")) {
                ("Claude Max ($200)".to_string(), 200_000_000)
            } else if hint_lower.contains("max") {
                ("Claude Max ($100)".to_string(), 100_000_000)
            } else if hint_lower.contains("team") {
                ("Claude Team".to_string(), 30_000_000)
            } else if hint_lower.contains("pro") {
                ("Claude Pro".to_string(), 20_000_000)
            } else {
                ("Claude Code 官方订阅".to_string(), 0)
            }
        }
        provider_ids::CODEX => {
            if hint_lower.contains("pro") {
                ("ChatGPT Pro / Codex".to_string(), 200_000_000)
            } else if hint_lower.contains("team") {
                ("ChatGPT Team / Codex".to_string(), 25_000_000)
            } else if hint_lower.contains("enterprise") {
                ("ChatGPT Enterprise / Codex".to_string(), 0)
            } else if hint_lower.contains("edu") {
                ("ChatGPT Edu / Codex".to_string(), 0)
            } else if hint_lower.contains("plus") {
                ("ChatGPT Plus / Codex".to_string(), 20_000_000)
            } else if hint_lower.contains("free") {
                ("ChatGPT Free / Codex".to_string(), 0)
            } else {
                ("Codex 官方订阅".to_string(), 0)
            }
        }
        provider_ids::ANTIGRAVITY => {
            if hint_lower.contains("ultra")
                && (hint_lower.contains("5x") || hint_lower.contains("99"))
            {
                ("Google AI Ultra 5x".to_string(), 99_990_000)
            } else if hint_lower.contains("ultra") && hint_lower.contains("249") {
                ("Google AI Ultra".to_string(), 249_990_000)
            } else if hint_lower.contains("ultra") {
                ("Google AI Ultra 20x".to_string(), 199_990_000)
            } else if hint_lower.contains("enterprise") || hint_lower.contains("business") {
                ("Gemini Enterprise / Antigravity".to_string(), 30_000_000)
            } else if hint_lower.contains("plus") {
                ("Google AI Plus".to_string(), 4_990_000)
            } else if hint_lower.contains("free") {
                ("Google AI Free / Antigravity".to_string(), 0)
            } else {
                ("Google AI Pro".to_string(), 19_990_000)
            }
        }
        provider_ids::KIMI_CODE => {
            let name = match hint_lower.as_str() {
                "level_basic" | "basic" => "Kimi Code 基础会员",
                "plus" | "moderato" => "Kimi Plus (Moderato)",
                "level_intermediate" | "allegretto" => "Kimi Code Allegretto",
                "pro" => "Kimi Pro (Allegretto)",
                "level_advanced" => "Kimi Code Moderato",
                "allegro" => "Kimi Max (Allegro)",
                "level_max" | "vivace" | "max" => "Kimi Code Vivace",
                "ultra" => "Kimi Ultra (Vivace)",
                _ => "Kimi Code 会员订阅",
            };
            (name.to_string(), 0)
        }
        provider_ids::MINIMAX_CODE => {
            if hint_lower.contains("max") {
                ("MiniMax Coding Max".to_string(), 0)
            } else if hint_lower.contains("pro") || hint_lower.contains("high") {
                ("MiniMax Coding Pro".to_string(), 0)
            } else if hint_lower.contains("plus") {
                ("MiniMax Coding Plus".to_string(), 0)
            } else if !hint_lower.is_empty() {
                let raw_hint = plan_hint.as_deref().unwrap_or("").trim();
                (format!("MiniMax Coding ({raw_hint})"), 0)
            } else {
                ("MiniMax Coding Plan".to_string(), 0)
            }
        }
        provider_ids::QCODE => {
            if hint_lower.contains("pro") || hint_lower.contains("max") {
                ("QCode Pro 订阅".to_string(), 0)
            } else if hint_lower.contains("plus") {
                ("QCode Plus 订阅".to_string(), 0)
            } else {
                ("QCode 日额度套餐".to_string(), 0)
            }
        }
        other => (format!("{other} 官方订阅"), 0),
    };

    let windows_json = serde_json::to_string(&windows)?;
    conn.execute(
        r#"
        INSERT INTO subscription_plans
            (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, is_user_configured, auto_source, user_customized, user_dismissed, updated_at_ms)
        VALUES (?1, ?2, ?3, 1, ?4, ?5, 1, ?6, 0, 0, ?7)
        ON CONFLICT(plan_id) DO UPDATE SET
            plan_name = excluded.plan_name,
            enabled = 1,
            monthly_fee_micro_usd = excluded.monthly_fee_micro_usd,
            windows_json = excluded.windows_json,
            is_user_configured = 1,
            auto_source = excluded.auto_source,
            updated_at_ms = excluded.updated_at_ms
        WHERE subscription_plans.user_customized = 0 AND subscription_plans.user_dismissed = 0
        "#,
        params![
            auto_plan_id,
            trimmed,
            plan_name,
            monthly_fee_micro_usd,
            windows_json,
            auto_source,
            now_ms,
        ],
    )?;

    Ok(())
}

pub(crate) fn load_preferences(conn: &Connection) -> CoreResult<AppPreferences> {
    conn.query_row(
        r#"
        SELECT currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup,
               otlp_enabled, otlp_port, device_display_name,
               bg_mode, bg_image_path, bg_preset_id, bg_opacity, bg_blur_px,
               auto_sync_pricing, exchange_rate_auto, last_rate_sync_at_ms, last_pricing_sync_at_ms,
               auto_check_updates, floating_window_enabled, updated_at_ms
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
                bg_mode: row.get(8)?,
                bg_image_path: row.get(9)?,
                bg_preset_id: row.get(10)?,
                bg_opacity: row.get(11)?,
                bg_blur_px: row.get(12)?,
                auto_sync_pricing: row.get::<_, i64>(13)? != 0,
                exchange_rate_auto: row.get::<_, i64>(14)? != 0,
                last_rate_sync_at_ms: row.get(15)?,
                last_pricing_sync_at_ms: row.get(16)?,
                auto_check_updates: row.get::<_, i64>(17)? != 0,
                floating_window_enabled: row.get::<_, i64>(18)? != 0,
                updated_at_ms: row.get(19)?,
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

    #[test]
    fn fresh_install_has_zero_subscriptions_and_default_bg_preferences() {
        let storage = Storage::open_in_memory().expect("in-memory db");
        let plans = storage.list_subscription_plans().expect("plans");
        assert!(plans.is_empty());

        let prefs = storage.get_preferences().expect("prefs");
        assert_eq!(prefs.bg_mode, "none");
        assert_eq!(prefs.bg_image_path, None);
        assert_eq!(prefs.bg_preset_id, "aurora_slate");
        assert!((prefs.bg_opacity - 0.18).abs() < 1e-6);
        assert_eq!(prefs.bg_blur_px, 16);
    }

    #[test]
    fn preferences_bg_roundtrip_and_subscription_delete_work() {
        let storage = Storage::open_in_memory().expect("in-memory db");
        let mut prefs = storage.get_preferences().expect("prefs");
        prefs.bg_mode = "gradient".to_string();
        prefs.bg_image_path = Some("wallpaper_123.png".to_string());
        prefs.bg_preset_id = "midnight_indigo".to_string();
        prefs.bg_opacity = 0.42;
        prefs.bg_blur_px = 24;
        storage.update_preferences(&prefs).expect("update prefs");

        let reloaded = storage.get_preferences().expect("reloaded prefs");
        assert_eq!(reloaded.bg_mode, "gradient");
        assert_eq!(
            reloaded.bg_image_path.as_deref(),
            Some("wallpaper_123.png")
        );
        assert_eq!(reloaded.bg_preset_id, "midnight_indigo");
        assert!((reloaded.bg_opacity - 0.42).abs() < 1e-6);
        assert_eq!(reloaded.bg_blur_px, 24);

        let plan = SubscriptionPlan {
            plan_id: "custom_claude".to_string(),
            provider_id: "claude_code".to_string(),
            plan_name: "My Claude Plan".to_string(),
            enabled: true,
            monthly_fee_micro_usd: 20_000_000,
            windows: vec![],
            is_user_configured: false,
            auto_source: None,
            updated_at_ms: 1_760_000_000_000,
        };
        storage.upsert_subscription_plan(&plan).expect("upsert plan");
        let list = storage.list_subscription_plans().expect("list plans");
        assert_eq!(list.len(), 1);
        assert!(list[0].is_user_configured);

        storage
            .delete_subscription_plan("custom_claude")
            .expect("delete plan");
        assert!(storage
            .list_subscription_plans()
            .expect("empty")
            .is_empty());
    }

    #[test]
    fn v1_to_v2_migration_cleans_presets_tps_and_rebuilds_rollups() {
        use crate::domain::QuotaUnit;

        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("legacy_v1.db");
        let seed_ts = 1_760_000_000_000i64;

        {
            let conn = Connection::open(&db_path).expect("open v1");
            conn.execute_batch(
                r#"
                CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);
                CREATE TABLE devices (device_id TEXT PRIMARY KEY, device_name TEXT NOT NULL, platform TEXT NOT NULL, created_at_ms INTEGER NOT NULL);
                CREATE TABLE provider_configs (provider_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL, collection_mode TEXT NOT NULL, custom_log_paths_json TEXT NOT NULL, has_credential INTEGER NOT NULL, last_sync_at_ms INTEGER, last_error TEXT, updated_at_ms INTEGER NOT NULL);
                CREATE TABLE ingestion_cursors (source_path TEXT PRIMARY KEY, provider_id TEXT NOT NULL, file_identity TEXT NOT NULL, byte_offset INTEGER NOT NULL, last_line_hash TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
                CREATE TABLE usage_events (event_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, provider_id TEXT NOT NULL, session_id TEXT NOT NULL, project_label TEXT NOT NULL, model_id TEXT NOT NULL, timestamp_ms INTEGER NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, observable_only INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER, ttft_ms INTEGER, output_tps REAL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE usage_hourly_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
                CREATE TABLE usage_daily_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
                CREATE TABLE model_pricing (model_pattern TEXT PRIMARY KEY, provider_id TEXT NOT NULL, input_per_m_micro_usd INTEGER NOT NULL, output_per_m_micro_usd INTEGER NOT NULL, cache_read_per_m_micro_usd INTEGER NOT NULL, cache_write_per_m_micro_usd INTEGER NOT NULL, reasoning_per_m_micro_usd INTEGER NOT NULL, is_user_override INTEGER NOT NULL, effective_from TEXT NOT NULL);
                CREATE TABLE subscription_plans (plan_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, plan_name TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, monthly_fee_micro_usd INTEGER NOT NULL, windows_json TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
                CREATE TABLE observed_rate_limits (provider_id TEXT NOT NULL, window_name TEXT NOT NULL, used_percent REAL NOT NULL, window_minutes INTEGER, resets_at_ms INTEGER, observed_at_ms INTEGER NOT NULL, PRIMARY KEY (provider_id, window_name));
                CREATE TABLE app_preferences (id INTEGER PRIMARY KEY CHECK (id = 1), currency TEXT NOT NULL, usd_to_cny_rate TEXT NOT NULL, theme TEXT NOT NULL, minimize_to_tray INTEGER NOT NULL, launch_at_startup INTEGER NOT NULL, otlp_enabled INTEGER NOT NULL, otlp_port INTEGER NOT NULL, device_display_name TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
                "#,
            )
            .expect("create v1 schema");

            conn.execute(
                "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (1, ?1)",
                params![seed_ts],
            )
            .expect("v1 version");
            conn.execute(
                "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES ('dev-1', 'win', 'windows', ?1)",
                params![seed_ts],
            )
            .expect("device");
            conn.execute(
                "INSERT INTO app_preferences (id, currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup, otlp_enabled, otlp_port, device_display_name, updated_at_ms) VALUES (1, 'USD', '7.25', 'dark', 1, 0, 0, 4318, 'win', ?1)",
                params![seed_ts],
            )
            .expect("prefs");

            let default_windows = serde_json::to_string(&vec![
                SubscriptionWindowRule {
                    window_id: "5h".to_string(),
                    label: "5h 滚动窗口".to_string(),
                    duration_hours: 5,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 15_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
                SubscriptionWindowRule {
                    window_id: "7d".to_string(),
                    label: "7d 周额度".to_string(),
                    duration_hours: 168,
                    rolling: true,
                    unit: QuotaUnit::MicroUsd,
                    limit_value: 75_000_000,
                    warning_percent: 80,
                    critical_percent: 95,
                },
            ])
            .expect("windows json");

            // Untouched v0.1.0 preset within 5_000 ms of device seed -> must be deleted
            conn.execute(
                "INSERT INTO subscription_plans (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, updated_at_ms) VALUES ('claude_pro', 'claude_code', 'Claude Pro ($20/mo)', 1, 20000000, ?1, ?2)",
                params![default_windows, seed_ts + 200],
            )
            .expect("untouched preset");

            // Customized v0.1.0 preset (fee altered) -> must be preserved with is_user_configured = 1
            conn.execute(
                "INSERT INTO subscription_plans (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, updated_at_ms) VALUES ('codex_plus', 'codex', 'ChatGPT Plus / Codex ($20/mo)', 1, 25000000, ?1, ?2)",
                params![default_windows, seed_ts + 200],
            )
            .expect("customized preset");

            // Valid event + fast distorted event + tiny output event
            conn.execute(
                "INSERT INTO usage_events VALUES ('ev-ok', 'dev-1', 'claude_code', 's1', 'proj', 'claude-sonnet-4-5', ?1, 100, 100, 0, 0, 0, 0, 2000, NULL, 50.0, 1000, 0)",
                params![seed_ts],
            )
            .expect("ev-ok");
            conn.execute(
                "INSERT INTO usage_events VALUES ('ev-fast', 'dev-1', 'claude_code', 's1', 'proj', 'claude-sonnet-4-5', ?1, 100, 500, 0, 0, 0, 0, 10, NULL, 50000.0, 2000, 0)",
                params![seed_ts + 1_000],
            )
            .expect("ev-fast");
            conn.execute(
                "INSERT INTO usage_events VALUES ('ev-tiny', 'dev-1', 'claude_code', 's1', 'proj', 'claude-sonnet-4-5', ?1, 100, 4, 0, 0, 0, 0, 500, NULL, 8.0, 500, 0)",
                params![seed_ts + 2_000],
            )
            .expect("ev-tiny");
        }

        let storage = Storage::open_file(&db_path).expect("migrate v1 -> v2");
        let plans = storage.list_subscription_plans().expect("plans");
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].plan_id, "codex_plus");
        assert!(plans[0].is_user_configured);

        let prefs = storage.get_preferences().expect("prefs");
        assert_eq!(prefs.bg_mode, "none");
        assert_eq!(prefs.theme, "dark");
        assert!(prefs.auto_sync_pricing);
        assert!(prefs.exchange_rate_auto);

        storage
            .with_conn(|conn| {
                let ver: i64 =
                    conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))?;
                assert_eq!(ver, crate::storage::migrations::CURRENT_SCHEMA_VERSION);

                let ok_tps: Option<f64> = conn.query_row(
                    "SELECT output_tps FROM usage_events WHERE event_id = 'ev-ok'",
                    [],
                    |r| r.get(0),
                )?;
                let fast_tps: Option<f64> = conn.query_row(
                    "SELECT output_tps FROM usage_events WHERE event_id = 'ev-fast'",
                    [],
                    |r| r.get(0),
                )?;
                let tiny_tps: Option<f64> = conn.query_row(
                    "SELECT output_tps FROM usage_events WHERE event_id = 'ev-tiny'",
                    [],
                    |r| r.get(0),
                )?;
                assert_eq!(ok_tps, Some(50.0));
                assert_eq!(fast_tps, None);
                assert_eq!(tiny_tps, None);

                let (h_cnt, h_sum, h_max): (i64, f64, f64) = conn.query_row(
                    "SELECT SUM(tps_sample_count), SUM(tps_sum), MAX(max_output_tps) FROM usage_hourly_rollups",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                assert_eq!(h_cnt, 1);
                assert!((h_sum - 50.0).abs() < 1e-6);
                assert!((h_max - 50.0).abs() < 1e-6);

                let (d_cnt, d_sum, d_max): (i64, f64, f64) = conn.query_row(
                    "SELECT SUM(tps_sample_count), SUM(tps_sum), MAX(max_output_tps) FROM usage_daily_rollups",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                assert_eq!(d_cnt, 1);
                assert!((d_sum - 50.0).abs() < 1e-6);
                assert!((d_max - 50.0).abs() < 1e-6);
                Ok(())
            })
            .expect("verify migrated state");
    }

    #[test]
    fn v2_to_v3_migration_preserves_settings_and_adds_sync_columns() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("legacy_v2.db");
        let seed_ts = 1_760_000_000_000i64;

        {
            let conn = Connection::open(&db_path).expect("open v2");
            conn.execute_batch(
                r#"
                CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);
                CREATE TABLE devices (device_id TEXT PRIMARY KEY, device_name TEXT NOT NULL, platform TEXT NOT NULL, created_at_ms INTEGER NOT NULL);
                CREATE TABLE provider_configs (provider_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL, collection_mode TEXT NOT NULL, custom_log_paths_json TEXT NOT NULL, has_credential INTEGER NOT NULL, last_sync_at_ms INTEGER, last_error TEXT, updated_at_ms INTEGER NOT NULL);
                CREATE TABLE ingestion_cursors (source_path TEXT PRIMARY KEY, provider_id TEXT NOT NULL, file_identity TEXT NOT NULL, byte_offset INTEGER NOT NULL, last_line_hash TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
                CREATE TABLE usage_events (event_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, provider_id TEXT NOT NULL, session_id TEXT NOT NULL, project_label TEXT NOT NULL, model_id TEXT NOT NULL, timestamp_ms INTEGER NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, observable_only INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER, ttft_ms INTEGER, output_tps REAL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE usage_hourly_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
                CREATE TABLE usage_daily_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
                CREATE TABLE model_pricing (model_pattern TEXT PRIMARY KEY, provider_id TEXT NOT NULL, input_per_m_micro_usd INTEGER NOT NULL, output_per_m_micro_usd INTEGER NOT NULL, cache_read_per_m_micro_usd INTEGER NOT NULL, cache_write_per_m_micro_usd INTEGER NOT NULL, reasoning_per_m_micro_usd INTEGER NOT NULL, is_user_override INTEGER NOT NULL, effective_from TEXT NOT NULL);
                CREATE TABLE subscription_plans (plan_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, plan_name TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, monthly_fee_micro_usd INTEGER NOT NULL, windows_json TEXT NOT NULL, is_user_configured INTEGER NOT NULL DEFAULT 0, updated_at_ms INTEGER NOT NULL);
                CREATE TABLE observed_rate_limits (provider_id TEXT NOT NULL, window_name TEXT NOT NULL, used_percent REAL NOT NULL, window_minutes INTEGER, resets_at_ms INTEGER, observed_at_ms INTEGER NOT NULL, PRIMARY KEY (provider_id, window_name));
                CREATE TABLE app_preferences (id INTEGER PRIMARY KEY CHECK (id = 1), currency TEXT NOT NULL, usd_to_cny_rate TEXT NOT NULL, theme TEXT NOT NULL, minimize_to_tray INTEGER NOT NULL, launch_at_startup INTEGER NOT NULL, otlp_enabled INTEGER NOT NULL, otlp_port INTEGER NOT NULL, device_display_name TEXT NOT NULL, bg_mode TEXT NOT NULL DEFAULT 'none', bg_image_path TEXT, bg_preset_id TEXT NOT NULL DEFAULT 'aurora_slate', bg_opacity REAL NOT NULL DEFAULT 0.18, bg_blur_px INTEGER NOT NULL DEFAULT 16, updated_at_ms INTEGER NOT NULL);
                "#,
            )
            .expect("create v2 schema");
            conn.execute(
                "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (1, ?1), (2, ?1)",
                params![seed_ts],
            )
            .expect("versions");
            conn.execute(
                "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES ('dev-2', 'win', 'windows', ?1)",
                params![seed_ts],
            )
            .expect("device");
            conn.execute(
                "INSERT INTO app_preferences (id, currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup, otlp_enabled, otlp_port, device_display_name, bg_mode, bg_preset_id, bg_opacity, bg_blur_px, updated_at_ms) VALUES (1, 'CNY', '7.31', 'light', 1, 0, 0, 4318, 'win', 'gradient', 'ocean', 0.22, 12, ?1)",
                params![seed_ts],
            )
            .expect("prefs");
            conn.execute(
                "INSERT INTO model_pricing VALUES ('claude-sonnet-4-5', 'claude_code', 999, 999, 0, 0, 999, 1, '2026-10-01')",
                [],
            )
            .expect("override pricing");
        }

        let storage = Storage::open_file(&db_path).expect("migrate v2 -> v3");
        let prefs = storage.get_preferences().expect("prefs");
        assert_eq!(prefs.currency, CurrencyCode::Cny);
        assert_eq!(prefs.usd_to_cny_rate, "7.31");
        assert!(prefs.auto_sync_pricing);
        assert!(prefs.exchange_rate_auto);
        assert_eq!(prefs.last_rate_sync_at_ms, None);
        assert_eq!(prefs.last_pricing_sync_at_ms, None);

        let catalog = storage.list_model_pricing().expect("catalog");
        let sonnet = catalog
            .iter()
            .find(|p| p.model_pattern == "claude-sonnet-4-5")
            .expect("sonnet");
        assert!(sonnet.is_user_override);
        assert_eq!(sonnet.input_per_m_micro_usd, 999);
    }

    #[test]
    fn synced_rate_respects_manual_lock_and_preserves_timestamps_across_pref_saves() {
        let storage = Storage::open_in_memory().expect("db");

        // Auto mode updates rate & timestamp
        let changed = storage
            .apply_synced_exchange_rate("7.1852", 1_760_100_000_000)
            .expect("apply rate");
        assert!(changed);
        let mut prefs = storage.get_preferences().expect("prefs");
        assert_eq!(prefs.usd_to_cny_rate, "7.1852");
        assert_eq!(prefs.last_rate_sync_at_ms, Some(1_760_100_000_000));

        // Lock manual rate -> saving UI preferences must not wipe sync timestamp
        prefs.exchange_rate_auto = false;
        prefs.usd_to_cny_rate = "7.35".to_string();
        prefs.last_rate_sync_at_ms = None;
        storage.update_preferences(&prefs).expect("save manual prefs");

        let reloaded = storage.get_preferences().expect("reloaded");
        assert!(!reloaded.exchange_rate_auto);
        assert_eq!(reloaded.usd_to_cny_rate, "7.35");
        assert_eq!(reloaded.last_rate_sync_at_ms, Some(1_760_100_000_000));

        // Subsequent synced rate must be ignored when locked
        let locked_changed = storage
            .apply_synced_exchange_rate("7.10", 1_760_200_000_000)
            .expect("locked noop");
        assert!(!locked_changed);
        let final_prefs = storage.get_preferences().expect("final prefs");
        assert_eq!(final_prefs.usd_to_cny_rate, "7.35");
        assert_eq!(final_prefs.last_rate_sync_at_ms, Some(1_760_100_000_000));
    }

    #[test]
    fn synced_pricing_preserves_user_overrides_and_conditionally_recomputes() {
        let storage = Storage::open_in_memory().expect("db");
        let dev = storage.local_device().expect("device").device_id;
        let ts = 1_760_000_000_000i64;

        // Add user override on claude-sonnet-4-5 ($2 / $10) and ingest two events:
        // one on overridden claude-sonnet-4-5, one on built-in gpt-5-codex
        storage
            .upsert_model_pricing(&ModelPricingEntry {
                model_pattern: "claude-sonnet-4-5".to_string(),
                provider_id: "claude_code".to_string(),
                input_per_m_micro_usd: 2_000_000,
                output_per_m_micro_usd: 10_000_000,
                cache_read_per_m_micro_usd: 200_000,
                cache_write_per_m_micro_usd: 2_500_000,
                reasoning_per_m_micro_usd: 10_000_000,
                is_user_override: true,
                effective_from: "2026-11-01".to_string(),
            })
            .expect("override");

        let events = vec![
            NormalizedUsageEvent {
                event_id: compute_event_id("claude_code", "s1", "m1", ts),
                device_id: dev.clone(),
                provider_id: "claude_code".to_string(),
                session_id: "s1".to_string(),
                project_label: "app".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                timestamp_ms: ts,
                input_tokens: 1_000_000,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: None,
                ttft_ms: None,
                output_tps: None,
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            },
            NormalizedUsageEvent {
                event_id: compute_event_id("codex", "s2", "m2", ts),
                device_id: dev,
                provider_id: "codex".to_string(),
                session_id: "s2".to_string(),
                project_label: "cli".to_string(),
                model_id: "gpt-5-codex".to_string(),
                timestamp_ms: ts,
                input_tokens: 1_000_000,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
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

        // Sync catalog changing ONLY unused gemini-3-flash & trying to change overridden sonnet
        let first_remote = vec![
            ModelPricingEntry {
                model_pattern: "claude-sonnet-4-5".to_string(),
                provider_id: "claude_code".to_string(),
                input_per_m_micro_usd: 99_000_000,
                output_per_m_micro_usd: 99_000_000,
                cache_read_per_m_micro_usd: 0,
                cache_write_per_m_micro_usd: 0,
                reasoning_per_m_micro_usd: 99_000_000,
                is_user_override: false,
                effective_from: "2026-11-16".to_string(),
            },
            ModelPricingEntry {
                model_pattern: "gemini-3-flash".to_string(),
                provider_id: "antigravity".to_string(),
                input_per_m_micro_usd: 450_000,
                output_per_m_micro_usd: 2_800_000,
                cache_read_per_m_micro_usd: 45_000,
                cache_write_per_m_micro_usd: 0,
                reasoning_per_m_micro_usd: 2_800_000,
                is_user_override: false,
                effective_from: "2026-11-16".to_string(),
            },
        ];

        let (updated_1, _, recomputed_1) = storage
            .apply_synced_pricing_catalog(&first_remote, ts + 10_000)
            .expect("first sync");
        assert!(updated_1);
        assert_eq!(recomputed_1, 0); // Neither active model's effective pricing changed!

        // Identical second sync short-circuits via hash without updating catalog
        let (updated_2, _, recomputed_2) = storage
            .apply_synced_pricing_catalog(&first_remote, ts + 20_000)
            .expect("second sync");
        assert!(!updated_2);
        assert_eq!(recomputed_2, 0);

        // Now change non-overridden active model gpt-5-codex price -> triggers recompute for 1 event!
        let second_remote = vec![ModelPricingEntry {
            model_pattern: "gpt-5-codex".to_string(),
            provider_id: "codex".to_string(),
            input_per_m_micro_usd: 2_500_000,
            output_per_m_micro_usd: 15_000_000,
            cache_read_per_m_micro_usd: 250_000,
            cache_write_per_m_micro_usd: 0,
            reasoning_per_m_micro_usd: 15_000_000,
            is_user_override: false,
            effective_from: "2026-11-16".to_string(),
        }];
        let (updated_3, _, recomputed_3) = storage
            .apply_synced_pricing_catalog(&second_remote, ts + 30_000)
            .expect("third sync");
        assert!(updated_3);
        assert_eq!(recomputed_3, 1);

        let catalog = storage.list_model_pricing().expect("catalog");
        let sonnet = catalog
            .iter()
            .find(|p| p.model_pattern == "claude-sonnet-4-5")
            .expect("sonnet");
        assert!(sonnet.is_user_override);
        assert_eq!(sonnet.input_per_m_micro_usd, 2_000_000);
    }

    #[test]
    fn v3_to_v4_migration_preserves_data_and_adds_new_tables() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("legacy_v3.db");
        let seed_ts = 1_760_000_000_000i64;

        {
            let conn = Connection::open(&db_path).expect("open v3");
            conn.execute_batch(
                r#"
                CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);
                CREATE TABLE devices (device_id TEXT PRIMARY KEY, device_name TEXT NOT NULL, platform TEXT NOT NULL, created_at_ms INTEGER NOT NULL);
                CREATE TABLE provider_configs (provider_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL, collection_mode TEXT NOT NULL, custom_log_paths_json TEXT NOT NULL, has_credential INTEGER NOT NULL, last_sync_at_ms INTEGER, last_error TEXT, updated_at_ms INTEGER NOT NULL);
                CREATE TABLE ingestion_cursors (source_path TEXT PRIMARY KEY, provider_id TEXT NOT NULL, file_identity TEXT NOT NULL, byte_offset INTEGER NOT NULL, last_line_hash TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
                CREATE TABLE usage_events (event_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, provider_id TEXT NOT NULL, session_id TEXT NOT NULL, project_label TEXT NOT NULL, model_id TEXT NOT NULL, timestamp_ms INTEGER NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, observable_only INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER, ttft_ms INTEGER, output_tps REAL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE usage_hourly_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
                CREATE TABLE usage_daily_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
                CREATE TABLE model_pricing (model_pattern TEXT PRIMARY KEY, provider_id TEXT NOT NULL, input_per_m_micro_usd INTEGER NOT NULL, output_per_m_micro_usd INTEGER NOT NULL, cache_read_per_m_micro_usd INTEGER NOT NULL, cache_write_per_m_micro_usd INTEGER NOT NULL, reasoning_per_m_micro_usd INTEGER NOT NULL, is_user_override INTEGER NOT NULL, effective_from TEXT NOT NULL);
                CREATE TABLE subscription_plans (plan_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, plan_name TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, monthly_fee_micro_usd INTEGER NOT NULL, windows_json TEXT NOT NULL, is_user_configured INTEGER NOT NULL DEFAULT 0, updated_at_ms INTEGER NOT NULL);
                CREATE TABLE observed_rate_limits (provider_id TEXT PRIMARY KEY, window_name TEXT NOT NULL, used_percent REAL NOT NULL, window_minutes INTEGER, resets_at_ms INTEGER, observed_at_ms INTEGER NOT NULL);
                DROP TABLE IF EXISTS observed_rate_limits;
                CREATE TABLE observed_rate_limits (provider_id TEXT NOT NULL, window_name TEXT NOT NULL, used_percent REAL NOT NULL, window_minutes INTEGER, resets_at_ms INTEGER, observed_at_ms INTEGER NOT NULL, PRIMARY KEY (provider_id, window_name));
                CREATE TABLE app_preferences (id INTEGER PRIMARY KEY CHECK (id = 1), currency TEXT NOT NULL, usd_to_cny_rate TEXT NOT NULL, theme TEXT NOT NULL, minimize_to_tray INTEGER NOT NULL, launch_at_startup INTEGER NOT NULL, otlp_enabled INTEGER NOT NULL, otlp_port INTEGER NOT NULL, device_display_name TEXT NOT NULL, bg_mode TEXT NOT NULL DEFAULT 'none', bg_image_path TEXT, bg_preset_id TEXT NOT NULL DEFAULT 'aurora_slate', bg_opacity REAL NOT NULL DEFAULT 0.18, bg_blur_px INTEGER NOT NULL DEFAULT 16, auto_sync_pricing INTEGER NOT NULL DEFAULT 1, exchange_rate_auto INTEGER NOT NULL DEFAULT 1, last_rate_sync_at_ms INTEGER, last_pricing_sync_at_ms INTEGER, pricing_catalog_hash TEXT, updated_at_ms INTEGER NOT NULL);
                CREATE TABLE remote_sync_meta (meta_key TEXT PRIMARY KEY, meta_value TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
                "#,
            )
            .expect("v3 schema");

            conn.execute(
                "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (3, ?1)",
                params![seed_ts],
            )
            .expect("v3 version");
            conn.execute(
                "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES ('dev-v3', 'ws-v3', 'windows', ?1)",
                params![seed_ts],
            )
            .expect("dev");
            conn.execute(
                "INSERT INTO app_preferences (id, currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup, otlp_enabled, otlp_port, device_display_name, updated_at_ms) VALUES (1, 'CNY', '7.20', 'light', 1, 1, 1, 4318, 'ws-v3', ?1)",
                params![seed_ts],
            )
            .expect("prefs");
            conn.execute(
                "INSERT INTO model_pricing VALUES ('deepseek-chat', 'deepseek', 270000, 1100000, 70000, 0, 1100000, 0, '2026-01-01')",
                [],
            )
            .expect("ds pricing");
        }

        let storage = Storage::open_file(&db_path).expect("migrate v3 -> v4");
        let prefs = storage.get_preferences().expect("prefs");
        assert!(prefs.auto_check_updates);
        assert_eq!(prefs.device_display_name, "ws-v3");

        let configs = storage.list_provider_configs().expect("providers");
        assert_eq!(configs.len(), 8);

        let pricing = storage.list_model_pricing().expect("pricing");
        let ds = pricing
            .iter()
            .find(|p| p.model_pattern == "deepseek-chat")
            .expect("ds migrated");
        assert_eq!(ds.provider_id, "deepseek_harness");

        let sync_cfg = storage.get_sync_config_state().expect("sync state");
        assert!(!sync_cfg.enabled);
        assert!(storage
            .list_provider_balances()
            .expect("balances")
            .is_empty());
        assert!(storage
            .list_custom_provider_rules()
            .expect("custom rules")
            .is_empty());
    }

    #[test]
    fn apply_remote_sync_envelope_is_idempotent_and_multi_device_safe() {
        use crate::domain::{SyncEnvelopeV1, SyncHourlyDelta, SyncSubscriptionSnapshot};

        let storage = Storage::open_in_memory().expect("db");
        let ts = 1_760_000_000_000i64;
        let hour = ts.div_euclid(HOUR_MS) * HOUR_MS;
        let env = SyncEnvelopeV1 {
            schema_version: 1,
            device_id: "macbook-air".to_string(),
            device_display_name: "MacBook Air".to_string(),
            generated_at_ms: ts + 10_000,
            since_watermark_ms: 0,
            hourly_deltas: vec![SyncHourlyDelta {
                provider_id: "kimi_code".to_string(),
                model_id: "kimi-k2-0905-preview".to_string(),
                bucket_start_ms: hour,
                request_count: 4,
                observable_step_count: 0,
                input_tokens: 10_000,
                output_tokens: 2_000,
                cache_read_tokens: 3_000,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                estimated_cost_micro_usd: 150_000,
                cache_saved_micro_usd: 5_000,
                total_duration_ms: 4_000,
                tps_sample_count: 4,
                tps_sum: 200.0,
                max_output_tps: 65.0,
                updated_at_ms: ts + 10_000,
            }],
            subscription_snapshots: vec![SyncSubscriptionSnapshot {
                plan_id: "kimi-pro".to_string(),
                provider_id: "kimi_code".to_string(),
                plan_name: "Kimi Pro".to_string(),
                window_id: "5h".to_string(),
                window_label: "5h 窗口".to_string(),
                used_percent: 33.5,
                resets_at_ms: Some(ts + 3_600_000),
                source: "estimated".to_string(),
            }],
        };

        let r1 = storage
            .apply_remote_sync_envelope_v1(&env, ts + 11_000)
            .expect("first apply");
        assert_eq!(r1.hourly_buckets_applied, 1);
        assert_eq!(r1.subscription_snapshots_applied, 1);

        // Replay identical envelope must stay idempotent and never double-count daily rollups
        let r2 = storage
            .apply_remote_sync_envelope_v1(&env, ts + 12_000)
            .expect("replay apply");
        assert_eq!(r2.hourly_buckets_applied, 0);

        storage
            .with_conn(|conn| {
                let (h_tok, h_req): (i64, i64) = conn.query_row(
                    "SELECT SUM(input_tokens + output_tokens + cache_read_tokens), SUM(request_count) FROM usage_hourly_rollups WHERE device_id = 'macbook-air'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                let (d_tok, d_req): (i64, i64) = conn.query_row(
                    "SELECT SUM(input_tokens + output_tokens + cache_read_tokens), SUM(request_count) FROM usage_daily_rollups WHERE device_id = 'macbook-air'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                assert_eq!((h_tok, h_req), (15_000, 4));
                assert_eq!((d_tok, d_req), (15_000, 4));
                Ok(())
            })
            .expect("verify rollup totals");

        let devices = storage.list_devices().expect("devices");
        assert!(devices.iter().any(|d| d.device_id == "macbook-air"));
        let remote_snaps = storage
            .list_remote_subscription_snapshots()
            .expect("remote snaps");
        assert_eq!(remote_snaps.len(), 1);
        assert_eq!(remote_snaps[0].device_display_name, "MacBook Air");
    }

    #[test]
    fn auto_subscription_discovery_respects_user_edits_dismissal_and_resets() {
        let storage = Storage::open_in_memory().expect("db");
        let now_ms = Utc::now().timestamp_millis();

        // 1. Fresh DB starts with zero subscriptions (Bug 4 preserves zero-assumption for non-observable providers).
        assert!(storage
            .query_subscriptions(now_ms)
            .expect("initial subs")
            .is_empty());

        // 2. Observed rate-limit from official API creates auto_kimi_code in "我的订阅配置".
        storage
            .upsert_observed_rate_limits(&[
                ObservedRateLimitWindow {
                    provider_id: provider_ids::KIMI_CODE.to_string(),
                    window_name: "5h".to_string(),
                    used_percent: 25.0,
                    window_minutes: Some(300),
                    resets_at_ms: Some(now_ms + 3_600_000),
                    observed_at_ms: now_ms,
                    source: Some("official_api".to_string()),
                    plan_hint: Some("LEVEL_INTERMEDIATE".to_string()),
                },
                ObservedRateLimitWindow {
                    provider_id: provider_ids::KIMI_CODE.to_string(),
                    window_name: "7d".to_string(),
                    used_percent: 40.0,
                    window_minutes: Some(10_080),
                    resets_at_ms: Some(now_ms + 3 * DAY_MS),
                    observed_at_ms: now_ms,
                    source: Some("official_api".to_string()),
                    plan_hint: Some("LEVEL_INTERMEDIATE".to_string()),
                },
            ])
            .expect("upsert kimi observed");

        let subs = storage.query_subscriptions(now_ms).expect("subs after api");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].plan.plan_id, "auto_kimi_code");
        assert_eq!(subs[0].plan.auto_source.as_deref(), Some("official_api"));
        assert_eq!(subs[0].plan.plan_name, "Kimi Code Allegretto");
        assert_eq!(subs[0].windows.len(), 3);
        assert_eq!(subs[0].windows[0].source, "official_api");
        assert!((subs[0].windows[0].used_percent - 25.0).abs() < 0.01);
        assert_eq!(subs[0].windows[2].window_id, "30d");
        assert_eq!(subs[0].windows[2].label, "30d 月度限额");

        // 3. Deleting an auto-discovered plan soft-dismisses it so future polls do not resurrect it.
        storage
            .delete_subscription_plan("auto_kimi_code")
            .expect("dismiss auto");
        assert!(storage
            .query_subscriptions(now_ms)
            .expect("after dismiss")
            .is_empty());

        storage
            .upsert_observed_rate_limits(&[ObservedRateLimitWindow {
                provider_id: provider_ids::KIMI_CODE.to_string(),
                window_name: "5h".to_string(),
                used_percent: 30.0,
                window_minutes: Some(300),
                resets_at_ms: Some(now_ms + 3_600_000),
                observed_at_ms: now_ms + 5_000,
                source: Some("official_api".to_string()),
                plan_hint: Some("LEVEL_INTERMEDIATE".to_string()),
            }])
            .expect("poll while dismissed");
        assert!(storage
            .query_subscriptions(now_ms + 5_000)
            .expect("remains dismissed")
            .is_empty());

        // 4. Re-configuring API key resets dismissal and re-discovers on next fetch.
        storage
            .reset_auto_subscription_dismissal(provider_ids::KIMI_CODE)
            .expect("reset dismissal");
        storage
            .upsert_observed_rate_limits(&[ObservedRateLimitWindow {
                provider_id: provider_ids::KIMI_CODE.to_string(),
                window_name: "5h".to_string(),
                used_percent: 35.0,
                window_minutes: Some(300),
                resets_at_ms: Some(now_ms + 3_600_000),
                observed_at_ms: now_ms + 10_000,
                source: Some("official_api".to_string()),
                plan_hint: Some("LEVEL_MAX".to_string()),
            }])
            .expect("poll after reset");
        let revived = storage
            .query_subscriptions(now_ms + 10_000)
            .expect("revived");
        assert_eq!(revived.len(), 1);
        assert_eq!(revived[0].plan.plan_name, "Kimi Code Vivace");

        // 5. User customization preserves custom plan_name while windows keep refreshing from API.
        let mut custom = revived[0].plan.clone();
        custom.plan_name = "我的 Kimi 专享套餐".to_string();
        custom.monthly_fee_micro_usd = 7_000_000;
        storage
            .upsert_subscription_plan(&custom)
            .expect("customize auto");
        storage
            .upsert_observed_rate_limits(&[ObservedRateLimitWindow {
                provider_id: provider_ids::KIMI_CODE.to_string(),
                window_name: "5h".to_string(),
                used_percent: 50.0,
                window_minutes: Some(300),
                resets_at_ms: Some(now_ms + 3_600_000),
                observed_at_ms: now_ms + 20_000,
                source: Some("official_api".to_string()),
                plan_hint: Some("LEVEL_MAX".to_string()),
            }])
            .expect("poll after customize");
        let after_edit = storage
            .query_subscriptions(now_ms + 20_000)
            .expect("after edit");
        assert_eq!(after_edit[0].plan.plan_name, "我的 Kimi 专享套餐");
        assert_eq!(after_edit[0].plan.monthly_fee_micro_usd, 7_000_000);
        assert!((after_edit[0].windows[0].used_percent - 50.0).abs() < 0.01);
    }

    #[test]
    fn v6_migration_cleans_tps_preserves_remote_rollups_and_handles_stream_upgrades() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("v6.sqlite");
        let storage = Storage::open_file(&db_path).expect("open v6");
        let dev = storage.local_device().expect("device").device_id;

        // Preferences persist floating_window_enabled
        let mut prefs = storage.get_preferences().expect("prefs");
        assert!(!prefs.floating_window_enabled);
        prefs.floating_window_enabled = true;
        storage.update_preferences(&prefs).expect("save prefs");
        assert!(storage.get_preferences().expect("reloaded").floating_window_enabled);

        // Insert a remote pure-rollup device that has no rows in usage_events
        let bucket = 1_794_816_000_000i64;
        storage
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES ('remote-mac', 'Mac', 'macos', 100)",
                    [],
                )?;
                conn.execute(
                    r#"
                    INSERT INTO usage_hourly_rollups (
                        device_id, provider_id, model_id, bucket_start_ms,
                        request_count, observable_step_count, input_tokens, output_tokens,
                        cache_read_tokens, cache_write_tokens, reasoning_tokens,
                        estimated_cost_micro_usd, cache_saved_micro_usd,
                        total_duration_ms, tps_sample_count, tps_sum, max_output_tps, updated_at_ms
                    ) VALUES ('remote-mac', 'claude_code', 'claude-sonnet-4-5', ?1, 2, 0, 1000, 200, 0, 0, 0, 5000, 0, 4000, 2, 100.0, 55.0, 100)
                    "#,
                    params![bucket],
                )?;
                Ok(())
            })
            .expect("seed remote");

        // Ingest a partial streaming chunk, then an upgraded final chunk + session title
        let partial = NormalizedUsageEvent {
            event_id: "claude_code:s1:msg1".to_string(),
            device_id: dev.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "s1".to_string(),
            project_label: "TokenTracker".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: bucket + 1_000,
            input_tokens: 500,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(400),
            ttft_ms: None,
            output_tps: Some(600.0), // invalid outlier (>450 + <500ms), must be dropped
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        };
        let title_derived = SessionTitleRecord {
            session_key: "claude_code:s1".to_string(),
            provider_id: "claude_code".to_string(),
            title: "<environment_context><cwd>/tmp</cwd></environment_context>Fix TPS spike"
                .to_string(),
            source: "derived".to_string(),
            updated_at_ms: bucket + 1_000,
        };
        storage
            .ingest_events_with_cursor_and_titles(&[partial], None, &[], &[title_derived])
            .expect("partial");

        let final_chunk = NormalizedUsageEvent {
            event_id: "claude_code:s1:msg1".to_string(),
            device_id: dev.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "s1".to_string(),
            project_label: "TokenTracker".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: bucket + 3_000,
            input_tokens: 500,
            output_tokens: 180,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(3_000),
            ttft_ms: None,
            output_tps: Some(60.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        };
        let title_explicit = SessionTitleRecord {
            session_key: "claude_code:s1".to_string(),
            provider_id: "claude_code".to_string(),
            title: "Final Explicit Session Title".to_string(),
            source: "explicit".to_string(),
            updated_at_ms: bucket + 3_000,
        };
        storage
            .ingest_events_with_cursor_and_titles(&[final_chunk], None, &[], &[title_explicit])
            .expect("final");

        storage.recompute_all_costs_and_rollups().expect("recompute");

        storage
            .with_conn(|conn| {
                let (out_tok, tps): (i64, Option<f64>) = conn.query_row(
                    "SELECT output_tokens, output_tps FROM usage_events WHERE event_id = 'claude_code:s1:msg1'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                assert_eq!(out_tok, 180);
                assert_eq!(tps, Some(60.0));

                let local_reqs: i64 = conn.query_row(
                    "SELECT request_count FROM usage_hourly_rollups WHERE device_id = ?1",
                    params![dev],
                    |r| r.get(0),
                )?;
                assert_eq!(local_reqs, 1);

                // Remote rollup must still exist intact!
                let remote_reqs: i64 = conn.query_row(
                    "SELECT request_count FROM usage_hourly_rollups WHERE device_id = 'remote-mac'",
                    [],
                    |r| r.get(0),
                )?;
                assert_eq!(remote_reqs, 2);

                let stored_title: String = conn.query_row(
                    "SELECT title FROM session_titles WHERE session_key = 'claude_code:s1'",
                    [],
                    |r| r.get(0),
                )?;
                assert_eq!(stored_title, "Final Explicit Session Title");
                Ok(())
            })
            .expect("verify");
    }

    #[test]
    fn antigravity_auto_subscription_monitors_5h_and_7d_rolling_windows() {
        let storage = Storage::open_in_memory().expect("db");
        let dev = storage.local_device().expect("device").device_id;
        let now_ms = 1_794_820_000_000i64;

        let ev = NormalizedUsageEvent {
            event_id: "antigravity:conv-1:step-1".to_string(),
            device_id: dev,
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            session_id: "conv-1".to_string(),
            project_label: "antigravity".to_string(),
            model_id: "Orchestrator (Carbon)".to_string(),
            timestamp_ms: now_ms - 60_000,
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: Some(4_000),
            ttft_ms: Some(600),
            output_tps: Some(25.0),
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        };
        let hint_obs = ObservedRateLimitWindow {
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            window_name: "tier_hint".to_string(),
            used_percent: 0.0,
            window_minutes: None,
            resets_at_ms: None,
            observed_at_ms: now_ms - 60_000,
            source: Some("local_log_hint".to_string()),
            plan_hint: Some("ultra_5x".to_string()),
        };

        storage
            .ingest_events_with_cursor(&[ev], None, &[hint_obs])
            .expect("ingest antigravity");

        // Internal tier_hint rows must NOT leak into list_observed_rate_limits()
        assert!(storage
            .list_observed_rate_limits()
            .expect("observed")
            .is_empty());

        let subs = storage.query_subscriptions(now_ms).expect("subs");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].plan.plan_id, "auto_antigravity");
        assert_eq!(subs[0].plan.plan_name, "Google AI Ultra 5x");
        assert_eq!(subs[0].plan.monthly_fee_micro_usd, 99_990_000);
        assert_eq!(subs[0].plan.auto_source.as_deref(), Some("local_log"));
        assert_eq!(subs[0].windows.len(), 2);
        // 1M input ($2.00) + 100k output ($1.20) = $3.20 used out of $100 5h window = 3.2%
        assert_eq!(subs[0].windows[0].used_value, 3_200_000);
        assert!((subs[0].windows[0].used_percent - 3.2).abs() < 0.01);
    }
}
