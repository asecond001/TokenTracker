use std::collections::{BTreeMap, HashMap};
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
use crate::pricing::{
    apply_pricing_to_event, compute_catalog_hash, default_pricing_catalog, parse_exchange_rate,
    resolve_pricing,
};
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
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO subscription_plans
                    (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, is_user_configured, updated_at_ms)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)
                ON CONFLICT(plan_id) DO UPDATE SET
                    provider_id = excluded.provider_id,
                    plan_name = excluded.plan_name,
                    enabled = excluded.enabled,
                    monthly_fee_micro_usd = excluded.monthly_fee_micro_usd,
                    windows_json = excluded.windows_json,
                    is_user_configured = 1,
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

    pub fn delete_subscription_plan(&self, plan_id: &str) -> CoreResult<()> {
        let trimmed = plan_id.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Validation(
                "Subscription plan_id cannot be empty".to_string(),
            ));
        }
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM subscription_plans WHERE plan_id = ?1",
                params![trimmed],
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
                    updated_at_ms = ?16
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

    tx.execute("DELETE FROM usage_hourly_rollups", [])?;
    tx.execute("DELETE FROM usage_daily_rollups", [])?;

    rebuild_rollup_from_events(&tx, "usage_hourly_rollups", HOUR_MS, now_ms)?;
    rebuild_rollup_from_events(&tx, "usage_daily_rollups", DAY_MS, now_ms)?;

    tx.commit()?;
    Ok(updates.len())
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
        SELECT plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd,
               windows_json, is_user_configured, updated_at_ms
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
            updated_at_ms: row.get(7)?,
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
               otlp_enabled, otlp_port, device_display_name,
               bg_mode, bg_image_path, bg_preset_id, bg_opacity, bg_blur_px,
               auto_sync_pricing, exchange_rate_auto, last_rate_sync_at_ms, last_pricing_sync_at_ms,
               updated_at_ms
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
                updated_at_ms: row.get(17)?,
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
                assert_eq!(ver, 3);

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
}
