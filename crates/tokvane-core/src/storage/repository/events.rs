//! Ingestion cursors, usage event batches, session titles and hourly/daily rollups.

use super::*;

impl Storage {
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
                        let model_upgraded = (old_model == "codex-unknown"
                            && ev.model_id != "codex-unknown"
                            && !ev.model_id.trim().is_empty())
                            || (old_pid == provider_ids::ANTIGRAVITY
                                && old_model != ev.model_id
                                && !ev.model_id.trim().is_empty());
                        let should_upgrade = new_total > old_total
                            || model_upgraded
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
                upsert_session_titles_conn(&tx, session_titles)?;
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
            self.note_events_inserted(inserted);
            Ok(IngestOutcome {
                received: events.len(),
                inserted,
                deduplicated: events.len().saturating_sub(inserted),
            })
        })
    }

    /// Advance the cached `usage_events` row count after a committed ingest batch.
    fn note_events_inserted(&self, inserted: usize) {
        if inserted == 0 {
            return;
        }
        if let Ok(mut cache) = self.event_count.lock() {
            cache.epoch = cache.epoch.wrapping_add(1);
            if let Some(count) = cache.count.as_mut() {
                *count += inserted as i64;
            }
        }
    }

    /// Total `usage_events` rows, served from an in-process cache so frequent
    /// tray refreshes avoid a full-table `COUNT(*)`.
    ///
    /// The cache is only seeded when no ingest commit happened while the count
    /// query ran (epoch check), so a stale snapshot is never persisted.
    pub(crate) fn total_event_count(&self, conn: &Connection) -> CoreResult<i64> {
        let epoch_before = {
            let cache = self
                .event_count
                .lock()
                .map_err(|_| CoreError::Validation("Event count cache poisoned".to_string()))?;
            if let Some(count) = cache.count {
                return Ok(count);
            }
            cache.epoch
        };
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0))?;
        if let Ok(mut cache) = self.event_count.lock() {
            if cache.epoch == epoch_before {
                cache.count = Some(count);
            }
        }
        Ok(count)
    }

    pub fn upsert_session_titles(&self, session_titles: &[SessionTitleRecord]) -> CoreResult<usize> {
        if session_titles.is_empty() {
            return Ok(0);
        }
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let count = upsert_session_titles_conn(&tx, session_titles)?;
            tx.commit()?;
            Ok(count)
        })
    }
}

pub(super) fn upsert_cursor_conn(conn: &Connection, cursor: &IngestionCursor) -> CoreResult<()> {
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

pub(super) fn upsert_rollup_table(
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
pub(super) fn recompute_single_rollup_bucket(
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

pub(crate) fn upsert_session_titles_conn(
    conn: &Connection,
    session_titles: &[SessionTitleRecord],
) -> CoreResult<usize> {
    if session_titles.is_empty() {
        return Ok(0);
    }
    let mut stmt = conn.prepare_cached(
        r#"
        INSERT INTO session_titles (session_key, provider_id, title, source, updated_at_ms)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(session_key) DO UPDATE SET
            title = excluded.title,
            source = excluded.source,
            updated_at_ms = excluded.updated_at_ms
        WHERE excluded.source = 'background'
           OR (session_titles.source != 'background' AND (
                (excluded.source = 'explicit' AND (
                    session_titles.source != 'explicit'
                    OR excluded.updated_at_ms >= session_titles.updated_at_ms
                ))
               OR (excluded.source = 'user_prompt' AND (
                    session_titles.source = 'derived'
                    OR (session_titles.source = 'user_prompt' AND (
                        excluded.updated_at_ms < session_titles.updated_at_ms
                        OR LENGTH(TRIM(session_titles.title)) = 0
                    ))
                ))
               OR (excluded.source = 'derived' AND session_titles.source = 'derived' AND (
                    excluded.updated_at_ms < session_titles.updated_at_ms
                    OR LENGTH(TRIM(session_titles.title)) = 0
                ))
           ))
        "#,
    )?;
    let mut affected = 0usize;
    for rec in session_titles {
        let source = match rec.source.as_str() {
            "background" => "background",
            "explicit" => "explicit",
            "user_prompt" => "user_prompt",
            _ => "derived",
        };
        let clean_title = sanitize_session_title(&rec.title).or_else(|| {
            if source == "background" {
                Some("后台会话".to_string())
            } else {
                None
            }
        });
        if let Some(title) = clean_title {
            affected += stmt.execute(params![
                rec.session_key,
                rec.provider_id,
                title,
                source,
                rec.updated_at_ms,
            ])?;
        }
    }
    Ok(affected)
}
