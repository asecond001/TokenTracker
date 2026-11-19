//! Cloud sync configuration, envelope building and remote merge.

use super::*;

impl Storage {
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
                    payg_snapshots_applied: 0,
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
                       plan_name, window_label, used_percent, resets_at_ms, source,
                       monthly_fee_micro_usd, updated_at_ms
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
                    monthly_fee_micro_usd: row.get(10)?,
                    updated_at_ms: row.get(11)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn list_remote_payg_snapshots(&self) -> CoreResult<Vec<RemotePaygSnapshot>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT device_id, device_display_name, account_id, provider_id, account_name,
                       enabled, currency, total_balance_micro, granted_balance_micro,
                       topped_up_balance_micro, remaining_manual_balance_micro_usd,
                       monthly_budget_micro_usd, today_cost_micro_usd, month_cost_micro_usd,
                       fetched_at_ms, updated_at_ms
                FROM remote_payg_snapshots
                ORDER BY device_display_name ASC, provider_id ASC, account_name ASC
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(RemotePaygSnapshot {
                    device_id: row.get(0)?,
                    device_display_name: row.get(1)?,
                    account_id: row.get(2)?,
                    provider_id: row.get(3)?,
                    account_name: row.get(4)?,
                    enabled: row.get::<_, i64>(5)? != 0,
                    currency: row.get(6)?,
                    total_balance_micro: row.get(7)?,
                    granted_balance_micro: row.get(8)?,
                    topped_up_balance_micro: row.get(9)?,
                    remaining_manual_balance_micro_usd: row.get(10)?,
                    monthly_budget_micro_usd: row.get(11)?,
                    today_cost_micro_usd: row.get(12)?,
                    month_cost_micro_usd: row.get(13)?,
                    fetched_at_ms: row.get(14)?,
                    updated_at_ms: row.get(15)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    /// Build a Phase 2 ready `SyncEnvelopeV1` containing only desensitized hourly deltas,
    /// subscription window percentages/fees, and PAYG balance snapshots updated since `since_watermark_ms`.
    pub fn build_sync_envelope_v1(
        &self,
        since_watermark_ms: i64,
        now_ms: i64,
    ) -> CoreResult<SyncEnvelopeV1> {
        let payg_statuses = self.query_payg_accounts(now_ms, 0)?;
        let payg_snapshots: Vec<SyncPaygSnapshot> = payg_statuses
            .into_iter()
            .map(|st| SyncPaygSnapshot {
                account_id: st.account.account_id,
                provider_id: st.account.provider_id,
                account_name: st.account.account_name,
                enabled: st.account.enabled,
                currency: st.official_balance.as_ref().map(|b| b.currency.clone()),
                total_balance_micro: st.official_balance.as_ref().and_then(|b| b.total_balance_micro),
                granted_balance_micro: st.official_balance.as_ref().and_then(|b| b.granted_balance_micro),
                topped_up_balance_micro: st
                    .official_balance
                    .as_ref()
                    .and_then(|b| b.topped_up_balance_micro),
                remaining_manual_balance_micro_usd: st.remaining_manual_balance_micro_usd,
                monthly_budget_micro_usd: st.account.monthly_budget_micro_usd,
                today_cost_micro_usd: st.today_cost_micro_usd,
                month_cost_micro_usd: st.month_cost_micro_usd,
                fetched_at_ms: st.official_balance.as_ref().map(|b| b.fetched_at_ms),
                updated_at_ms: st.account.updated_at_ms,
            })
            .collect();

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

            let mut configured_providers: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut subscription_snapshots = Vec::new();
            for plan in plans.iter().filter(|p| p.enabled && p.is_user_configured) {
                configured_providers.insert(plan.provider_id.to_ascii_lowercase());
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
                        monthly_fee_micro_usd: plan.monthly_fee_micro_usd,
                    });
                }
            }

            // Also include unbound official observed quota windows not covered by a configured plan.
            let mut sorted_observed: Vec<&ObservedRateLimitWindow> = observed
                .iter()
                .filter(|obs| {
                    obs.source.as_deref() != Some("local_log_hint")
                        && obs.window_name != "tier_hint"
                        && !configured_providers.contains(&obs.provider_id.to_ascii_lowercase())
                })
                .collect();
            sorted_observed.sort_by_key(|a| std::cmp::Reverse(a.observed_at_ms));

            let mut emitted_unbound: std::collections::HashSet<(String, String)> =
                std::collections::HashSet::new();
            for obs in sorted_observed {
                let rule = infer_window_rule_from_observed(
                    &obs.provider_id,
                    obs,
                    &[],
                    None,
                    None,
                );
                let win = crate::subscription::evaluate_window(
                    &obs.provider_id,
                    &rule,
                    &[],
                    &observed,
                    now_ms,
                );
                if win.source == "local_estimation" {
                    continue;
                }
                let dedup_key = (
                    obs.provider_id.to_ascii_lowercase(),
                    win.window_id.to_ascii_lowercase(),
                );
                if !emitted_unbound.insert(dedup_key) {
                    continue;
                }
                let plan_name = obs
                    .plan_hint
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| {
                        format!("{} 官方限额", provider_ids::display_name(&obs.provider_id))
                    });
                subscription_snapshots.push(SyncSubscriptionSnapshot {
                    plan_id: format!("observed:{}", obs.provider_id),
                    provider_id: obs.provider_id.clone(),
                    plan_name,
                    window_id: win.window_id,
                    window_label: win.label,
                    used_percent: win.used_percent,
                    resets_at_ms: win.resets_at_ms,
                    source: win.source,
                    monthly_fee_micro_usd: 0,
                });
            }

            Ok(SyncEnvelopeV1 {
                schema_version: 1,
                device_id: device.device_id,
                device_display_name: prefs.device_display_name,
                generated_at_ms: now_ms,
                since_watermark_ms,
                hourly_deltas,
                subscription_snapshots,
                payg_snapshots,
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
            payg_snapshots_applied: 0,
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
            WHERE excluded.updated_at_ms > usage_hourly_rollups.updated_at_ms
               OR (excluded.updated_at_ms = usage_hourly_rollups.updated_at_ms
                   AND (excluded.request_count != usage_hourly_rollups.request_count
                        OR excluded.input_tokens != usage_hourly_rollups.input_tokens
                        OR excluded.output_tokens != usage_hourly_rollups.output_tokens
                        OR excluded.estimated_cost_micro_usd != usage_hourly_rollups.estimated_cost_micro_usd))
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
                affected_days.insert((
                    remote_device_id.to_string(),
                    delta.bucket_start_ms.div_euclid(DAY_MS) * DAY_MS,
                ));
            }
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
            plan_name, window_label, used_percent, resets_at_ms, source,
            monthly_fee_micro_usd, updated_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
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
            snap.monthly_fee_micro_usd,
            envelope.generated_at_ms.max(received_at_ms),
        ])?;
    }

    tx.execute(
        "DELETE FROM remote_payg_snapshots WHERE device_id = ?1",
        params![remote_device_id],
    )?;
    let mut payg_stmt = tx.prepare_cached(
        r#"
        INSERT INTO remote_payg_snapshots (
            device_id, device_display_name, account_id, provider_id, account_name,
            enabled, currency, total_balance_micro, granted_balance_micro,
            topped_up_balance_micro, remaining_manual_balance_micro_usd,
            monthly_budget_micro_usd, today_cost_micro_usd, month_cost_micro_usd,
            fetched_at_ms, updated_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        "#,
    )?;
    for payg in &envelope.payg_snapshots {
        payg_stmt.execute(params![
            remote_device_id,
            display_name,
            payg.account_id,
            payg.provider_id,
            payg.account_name,
            i64::from(payg.enabled),
            payg.currency,
            payg.total_balance_micro,
            payg.granted_balance_micro,
            payg.topped_up_balance_micro,
            payg.remaining_manual_balance_micro_usd,
            payg.monthly_budget_micro_usd,
            payg.today_cost_micro_usd,
            payg.month_cost_micro_usd,
            payg.fetched_at_ms,
            payg.updated_at_ms.max(envelope.generated_at_ms).max(received_at_ms),
        ])?;
    }

    Ok(RemoteSyncApplyResult {
        device_id: remote_device_id.to_string(),
        hourly_buckets_applied: hourly_applied,
        subscription_snapshots_applied: envelope.subscription_snapshots.len(),
        payg_snapshots_applied: envelope.payg_snapshots.len(),
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
