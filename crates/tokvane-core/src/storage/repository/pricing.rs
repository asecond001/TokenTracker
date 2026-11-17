//! Model pricing catalog, exchange rate sync and cost recomputation.

use super::*;

impl Storage {
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
}

pub(super) type PricingRateTuple = (i64, i64, i64, i64, i64);

pub(super) fn resolve_pricing_rates(
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

pub(super) fn load_distinct_usage_models(conn: &Connection) -> CoreResult<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT DISTINCT provider_id, model_id FROM usage_events")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(super) fn recompute_all_costs_and_rollups_with_conn(conn: &mut Connection) -> CoreResult<usize> {
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
