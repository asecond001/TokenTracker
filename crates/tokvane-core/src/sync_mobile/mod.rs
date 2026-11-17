use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::analytics::{local_day_bounds_ms, query_latest_tps, query_provider_shares};
use crate::domain::{
    provider_ids, AppPreferences, CurrencyCode, EncryptedSyncEnvelopeV1,
    RemoteSubscriptionSnapshot, SyncConfigState, SyncEnvelopeV1,
};
use crate::error::{CoreError, CoreResult};
use crate::pricing::remote_sync::{format_exchange_rate, parse_remote_exchange_rate_json};
use crate::storage::repository::{
    merge_remote_envelope_tx, query_local_device, rebuild_remote_daily_rollups_tx, DAY_MS,
    HOUR_MS,
};
use crate::storage::Storage;
use crate::sync_crypto::{clear_key_cache, decrypt_sync_envelope_v1_cached};

/// Wire representation of one record returned by Cloudflare Worker `GET /v1/sync/pull`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPullItemV1 {
    pub mode: String,
    pub seq_ms: i64,
    pub device_id: String,
    #[serde(default)]
    pub envelope: Option<SyncEnvelopeV1>,
    #[serde(default)]
    pub encrypted_envelope: Option<EncryptedSyncEnvelopeV1>,
}

/// Wire response returned by Cloudflare Worker `GET /v1/sync/pull`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPullResponseV1 {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub server_seq_ms: i64,
    #[serde(default)]
    pub next_since_ms: i64,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub items: Vec<SyncPullItemV1>,
}

/// Summary outcome returned after a single-transaction mobile batch pull merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncPullIngestReport {
    pub items_received: usize,
    pub envelopes_applied: usize,
    pub hourly_buckets_applied: usize,
    pub subscription_snapshots_applied: usize,
    pub next_since_ms: i64,
    pub server_seq_ms: i64,
    pub has_more: bool,
    pub applied_device_ids: Vec<String>,
}

/// Activity summary for one synced desktop/laptop peer device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncedDevicePulse {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub last_activity_ms: Option<i64>,
    pub today_tokens: i64,
    pub today_cost_micro_usd: i64,
    pub total_tokens: i64,
    pub total_cost_micro_usd: i64,
}

/// Initial state payload loaded by the mobile UI shell on launch and after sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobileBootstrapState {
    pub schema_version: u32,
    pub viewer_device_id: String,
    pub preferences: AppPreferences,
    pub sync_config: SyncConfigState,
    pub synced_devices: Vec<SyncedDevicePulse>,
    pub remote_subscriptions: Vec<RemoteSubscriptionSnapshot>,
}

/// Partial preferences patch supported from the iOS settings screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MobilePreferencesPatch {
    pub currency: Option<CurrencyCode>,
    pub usd_to_cny_rate: Option<String>,
    pub theme: Option<String>,
    pub exchange_rate_auto: Option<bool>,
}

/// Compact provider breakdown item inside `MobileWidgetSnapshot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetProviderShare {
    pub provider_id: String,
    pub display_name: String,
    pub tokens: i64,
    pub cost_micro_usd: i64,
    pub share_percent: f64,
}

/// Compact subscription rolling-window item inside `MobileWidgetSnapshot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetQuotaWindowItem {
    pub device_id: String,
    pub device_display_name: String,
    pub plan_id: String,
    pub provider_id: String,
    pub plan_name: String,
    pub window_id: String,
    pub window_label: String,
    pub used_percent: f64,
    pub resets_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

/// Sub-8KB atomic dashboard snapshot shared between the iOS app and `TokenVaneWidgets`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobileWidgetSnapshot {
    pub schema_version: u32,
    pub generated_at_ms: i64,
    pub currency: CurrencyCode,
    pub usd_to_cny_rate: String,
    pub today_tokens: i64,
    pub today_cost_micro_usd: i64,
    pub today_requests: i64,
    pub month_tokens: i64,
    pub month_cost_micro_usd: i64,
    pub active_device_count: i64,
    pub latest_output_tps: Option<f64>,
    pub last_sync_at_ms: Option<i64>,
    pub top_providers: Vec<WidgetProviderShare>,
    pub primary_quotas: Vec<WidgetQuotaWindowItem>,
}

#[derive(Debug, Default)]
struct RollupTotals {
    tokens: i64,
    cost_micro_usd: i64,
    requests: i64,
}

fn query_hourly_totals(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    device_id: Option<&str>,
) -> CoreResult<RollupTotals> {
    let aligned_start = start_ms.div_euclid(HOUR_MS) * HOUR_MS;
    if let Some(dev_id) = device_id.map(str::trim).filter(|s| !s.is_empty()) {
        conn.query_row(
            r#"
            SELECT
                COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens), 0),
                COALESCE(SUM(estimated_cost_micro_usd), 0),
                COALESCE(SUM(request_count), 0)
            FROM usage_hourly_rollups
            WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2 AND device_id = ?3
            "#,
            params![aligned_start, end_ms, dev_id],
            |r| {
                Ok(RollupTotals {
                    tokens: r.get(0)?,
                    cost_micro_usd: r.get(1)?,
                    requests: r.get(2)?,
                })
            },
        )
        .map_err(CoreError::from)
    } else {
        conn.query_row(
            r#"
            SELECT
                COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens), 0),
                COALESCE(SUM(estimated_cost_micro_usd), 0),
                COALESCE(SUM(request_count), 0)
            FROM usage_hourly_rollups
            WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2
            "#,
            params![aligned_start, end_ms],
            |r| {
                Ok(RollupTotals {
                    tokens: r.get(0)?,
                    cost_micro_usd: r.get(1)?,
                    requests: r.get(2)?,
                })
            },
        )
        .map_err(CoreError::from)
    }
}

fn decode_pull_envelopes(
    items: &[SyncPullItemV1],
    e2ee_passphrase: Option<&str>,
) -> CoreResult<Vec<SyncEnvelopeV1>> {
    let mut envelopes = Vec::with_capacity(items.len());
    let mut key_cache: HashMap<String, [u8; 32]> = HashMap::new();
    let passphrase_clean = e2ee_passphrase.map(str::trim).filter(|s| !s.is_empty());

    let res: CoreResult<()> = (|| {
        for item in items {
            let env = match item.mode.trim() {
                "plaintext" => item.envelope.clone().ok_or_else(|| {
                    CoreError::Validation(format!(
                        "Plaintext sync pull item for device '{}' missing envelope",
                        item.device_id
                    ))
                })?,
                "e2ee" => {
                    let encrypted = item.encrypted_envelope.as_ref().ok_or_else(|| {
                        CoreError::Validation(format!(
                            "E2EE sync pull item for device '{}' missing encrypted_envelope",
                            item.device_id
                        ))
                    })?;
                    let pass = passphrase_clean.ok_or_else(|| {
                        CoreError::Validation(
                            "E2EE sync envelope received but no passphrase is configured"
                                .to_string(),
                        )
                    })?;
                    decrypt_sync_envelope_v1_cached(encrypted, pass, &mut key_cache)?
                }
                other => {
                    return Err(CoreError::Validation(format!(
                        "Unsupported sync pull item mode '{other}'"
                    )));
                }
            };
            if !item.device_id.trim().is_empty()
                && env.device_id.trim() != item.device_id.trim()
            {
                return Err(CoreError::Validation(format!(
                    "Sync pull device_id '{}' does not match envelope device_id '{}'",
                    item.device_id, env.device_id
                )));
            }
            envelopes.push(env);
        }
        Ok(())
    })();

    clear_key_cache(&mut key_cache);
    res?;
    Ok(envelopes)
}

impl Storage {
    /// Open (or initialize) a mobile SQLite database at `path` and set the viewer device label.
    pub fn open_mobile_file(path: &Path, local_device_name: &str) -> CoreResult<Self> {
        let storage = Self::open_file(path)?;
        let cleaned_name = local_device_name.trim();
        if !cleaned_name.is_empty() {
            storage.with_conn(|conn| {
                let local_dev = query_local_device(conn)?;
                conn.execute(
                    "UPDATE devices SET device_name = ?1 WHERE device_id = ?2",
                    params![cleaned_name, local_dev.device_id],
                )?;
                Ok(())
            })?;
        }
        Ok(storage)
    }

    /// Load mobile bootstrap configuration, device pulse metadata, and synced subscriptions.
    pub fn get_mobile_bootstrap(
        &self,
        now_ms: i64,
        tz_offset_minutes: i32,
    ) -> CoreResult<MobileBootstrapState> {
        let preferences = self.get_preferences()?;
        let sync_config = self.get_sync_config_state()?;
        let remote_subscriptions = self.list_remote_subscription_snapshots()?;
        let (today_start_ms, _) = local_day_bounds_ms(now_ms, tz_offset_minutes);
        let today_aligned_start = today_start_ms.div_euclid(HOUR_MS) * HOUR_MS;

        let (viewer_device_id, synced_devices) = self.with_conn(|conn| {
            let local_dev = query_local_device(conn)?;
            let mut stmt = conn.prepare(
                r#"
                SELECT
                    d.device_id,
                    d.device_name,
                    d.platform,
                    (
                        SELECT MAX(ts) FROM (
                            SELECT MAX(updated_at_ms) AS ts FROM usage_hourly_rollups u WHERE u.device_id = d.device_id
                            UNION ALL
                            SELECT MAX(updated_at_ms) AS ts FROM remote_subscription_snapshots s WHERE s.device_id = d.device_id
                        )
                    ) AS last_activity_ms,
                    COALESCE((
                        SELECT SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens)
                        FROM usage_hourly_rollups u
                        WHERE u.device_id = d.device_id AND u.bucket_start_ms >= ?2 AND u.bucket_start_ms <= ?3
                    ), 0) AS today_tokens,
                    COALESCE((
                        SELECT SUM(estimated_cost_micro_usd)
                        FROM usage_hourly_rollups u
                        WHERE u.device_id = d.device_id AND u.bucket_start_ms >= ?2 AND u.bucket_start_ms <= ?3
                    ), 0) AS today_cost_micro_usd,
                    COALESCE((
                        SELECT SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens)
                        FROM usage_hourly_rollups u
                        WHERE u.device_id = d.device_id
                    ), 0) AS total_tokens,
                    COALESCE((
                        SELECT SUM(estimated_cost_micro_usd)
                        FROM usage_hourly_rollups u
                        WHERE u.device_id = d.device_id
                    ), 0) AS total_cost_micro_usd
                FROM devices d
                WHERE d.device_id != ?1
                ORDER BY COALESCE(last_activity_ms, d.created_at_ms) DESC, d.device_name ASC
                "#,
            )?;
            let mut pulses = Vec::new();
            let rows = stmt.query_map(
                params![local_dev.device_id, today_aligned_start, now_ms],
                |row| {
                    Ok(SyncedDevicePulse {
                        device_id: row.get(0)?,
                        device_name: row.get(1)?,
                        platform: row.get(2)?,
                        last_activity_ms: row.get(3)?,
                        today_tokens: row.get(4)?,
                        today_cost_micro_usd: row.get(5)?,
                        total_tokens: row.get(6)?,
                        total_cost_micro_usd: row.get(7)?,
                    })
                },
            )?;
            for row in rows {
                pulses.push(row?);
            }
            Ok((local_dev.device_id, pulses))
        })?;

        Ok(MobileBootstrapState {
            schema_version: 1,
            viewer_device_id,
            preferences,
            sync_config,
            synced_devices,
            remote_subscriptions,
        })
    }

    /// Decrypt and merge a `GET /v1/sync/pull` JSON payload inside a single immediate transaction.
    pub fn ingest_sync_pull_payload(
        &self,
        raw_json: &str,
        e2ee_passphrase: Option<&str>,
        now_ms: i64,
    ) -> CoreResult<SyncPullIngestReport> {
        let pull: SyncPullResponseV1 = serde_json::from_str(raw_json)?;
        if !pull.ok {
            return Err(CoreError::Validation(
                pull.error
                    .unwrap_or_else(|| "Sync pull response reported ok=false".to_string()),
            ));
        }

        let envelopes = decode_pull_envelopes(&pull.items, e2ee_passphrase)?;
        let items_received = pull.items.len();

        self.with_conn_mut(|conn| {
            let local_dev = query_local_device(conn)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut affected_days: BTreeSet<(String, i64)> = BTreeSet::new();
            let mut envelopes_applied = 0usize;
            let mut hourly_buckets_applied = 0usize;
            let mut subscription_snapshots_applied = 0usize;
            let mut applied_devices: BTreeMap<String, ()> = BTreeMap::new();

            for envelope in &envelopes {
                let outcome = merge_remote_envelope_tx(
                    &tx,
                    &local_dev.device_id,
                    envelope,
                    now_ms,
                    &mut affected_days,
                )?;
                if outcome.device_id != local_dev.device_id {
                    envelopes_applied += 1;
                    hourly_buckets_applied += outcome.hourly_buckets_applied;
                    subscription_snapshots_applied += outcome.subscription_snapshots_applied;
                    applied_devices.insert(outcome.device_id, ());
                }
            }

            rebuild_remote_daily_rollups_tx(&tx, &affected_days)?;

            tx.execute(
                r#"
                UPDATE sync_state
                SET last_pull_watermark_ms = MAX(last_pull_watermark_ms, ?1),
                    last_sync_at_ms = ?2,
                    last_sync_error = NULL,
                    updated_at_ms = ?2
                WHERE id = 1
                "#,
                params![pull.next_since_ms, now_ms],
            )?;
            tx.commit()?;

            Ok(SyncPullIngestReport {
                items_received,
                envelopes_applied,
                hourly_buckets_applied,
                subscription_snapshots_applied,
                next_since_ms: pull.next_since_ms,
                server_seq_ms: pull.server_seq_ms,
                has_more: pull.has_more,
                applied_device_ids: applied_devices.into_keys().collect(),
            })
        })
    }

    /// Build a compact `< 8 KB` dashboard snapshot for `WidgetKit` and Live Activities.
    pub fn build_mobile_widget_snapshot(
        &self,
        now_ms: i64,
        tz_offset_minutes: i32,
    ) -> CoreResult<MobileWidgetSnapshot> {
        let prefs = self.get_preferences()?;
        let sync_cfg = self.get_sync_config_state()?;
        let (today_start_ms, month_start_ms) = local_day_bounds_ms(now_ms, tz_offset_minutes);

        self.with_conn(|conn| {
            let local_dev = query_local_device(conn)?;
            let today_totals = query_hourly_totals(conn, today_start_ms, now_ms, None)?;
            let month_totals = query_hourly_totals(conn, month_start_ms, now_ms, None)?;
            let latest_output_tps = query_latest_tps(conn, now_ms - DAY_MS, now_ms, None, None)?;

            let active_device_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM devices WHERE device_id != ?1",
                params![local_dev.device_id],
                |r| r.get(0),
            )?;

            let shares = query_provider_shares(conn, today_start_ms, now_ms, None, None)?;
            let top_providers = shares
                .into_iter()
                .take(4)
                .map(|item| WidgetProviderShare {
                    provider_id: item.key,
                    display_name: item.label,
                    tokens: item.tokens,
                    cost_micro_usd: item.cost_micro_usd,
                    share_percent: item.token_share_percent,
                })
                .collect();

            let mut quota_stmt = conn.prepare(
                r#"
                SELECT device_id, device_display_name, plan_id, provider_id,
                       plan_name, window_id, window_label, used_percent, resets_at_ms, updated_at_ms
                FROM remote_subscription_snapshots
                ORDER BY used_percent DESC, updated_at_ms DESC, provider_id ASC
                LIMIT 4
                "#,
            )?;
            let mut primary_quotas = Vec::new();
            let quota_rows = quota_stmt.query_map([], |row| {
                Ok(WidgetQuotaWindowItem {
                    device_id: row.get(0)?,
                    device_display_name: row.get(1)?,
                    plan_id: row.get(2)?,
                    provider_id: row.get(3)?,
                    plan_name: row.get(4)?,
                    window_id: row.get(5)?,
                    window_label: row.get(6)?,
                    used_percent: row.get(7)?,
                    resets_at_ms: row.get(8)?,
                    updated_at_ms: row.get(9)?,
                })
            })?;
            for row in quota_rows {
                primary_quotas.push(row?);
            }

            Ok(MobileWidgetSnapshot {
                schema_version: 1,
                generated_at_ms: now_ms,
                currency: prefs.currency,
                usd_to_cny_rate: prefs.usd_to_cny_rate,
                today_tokens: today_totals.tokens,
                today_cost_micro_usd: today_totals.cost_micro_usd,
                today_requests: today_totals.requests,
                month_tokens: month_totals.tokens,
                month_cost_micro_usd: month_totals.cost_micro_usd,
                active_device_count,
                latest_output_tps,
                last_sync_at_ms: sync_cfg.last_sync_at_ms,
                top_providers,
                primary_quotas,
            })
        })
    }

    /// Patch mobile-specific display and currency preferences.
    pub fn update_mobile_preferences(
        &self,
        patch: &MobilePreferencesPatch,
        now_ms: i64,
    ) -> CoreResult<AppPreferences> {
        let mut prefs = self.get_preferences()?;
        if let Some(currency) = patch.currency {
            prefs.currency = currency;
        }
        if let Some(rate) = patch.usd_to_cny_rate.as_deref() {
            let trimmed = rate.trim();
            crate::pricing::parse_exchange_rate(trimmed)?;
            prefs.usd_to_cny_rate = trimmed.to_string();
        }
        if let Some(theme) = patch.theme.as_deref() {
            let trimmed = theme.trim();
            if !matches!(trimmed, "dark" | "light" | "system") {
                return Err(CoreError::Validation(format!(
                    "Unsupported theme '{theme}': expected dark, light, or system"
                )));
            }
            prefs.theme = trimmed.to_string();
        }
        if let Some(auto_rate) = patch.exchange_rate_auto {
            prefs.exchange_rate_auto = auto_rate;
        }
        self.update_preferences(&prefs)?;
        let _ = self.with_conn(|conn| {
            conn.execute(
                "UPDATE app_preferences SET updated_at_ms = ?1 WHERE id = 1",
                params![now_ms],
            )?;
            Ok(())
        });
        self.get_preferences()
    }

    /// Parse a Frankfurter / Open ER JSON payload and update `usd_to_cny_rate` + timestamp.
    pub fn apply_remote_exchange_rate_payload(
        &self,
        _source: &str,
        raw_json: &str,
        now_ms: i64,
    ) -> CoreResult<f64> {
        let rate_decimal = parse_remote_exchange_rate_json(raw_json)?;
        let formatted = format_exchange_rate(rate_decimal);
        let numeric: f64 = formatted.parse().map_err(|e| {
            CoreError::Validation(format!("Failed to convert formatted exchange rate '{formatted}': {e}"))
        })?;
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE app_preferences
                SET usd_to_cny_rate = ?1,
                    last_rate_sync_at_ms = ?2,
                    updated_at_ms = ?2
                WHERE id = 1
                "#,
                params![formatted, now_ms],
            )?;
            Ok(())
        })?;
        Ok(numeric)
    }

    /// Atomically clear remote synced devices, rollups, subscription snapshots, and reset pull watermark.
    pub fn reset_remote_sync_cache(&self, now_ms: i64) -> CoreResult<SyncConfigState> {
        self.with_conn_mut(|conn| {
            let local_dev = query_local_device(conn)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "DELETE FROM usage_daily_rollups WHERE device_id != ?1",
                params![local_dev.device_id],
            )?;
            tx.execute(
                "DELETE FROM usage_hourly_rollups WHERE device_id != ?1",
                params![local_dev.device_id],
            )?;
            tx.execute("DELETE FROM remote_subscription_snapshots", [])?;
            tx.execute(
                "DELETE FROM devices WHERE device_id != ?1",
                params![local_dev.device_id],
            )?;
            tx.execute(
                r#"
                UPDATE sync_state
                SET last_pull_watermark_ms = 0,
                    last_sync_error = NULL,
                    updated_at_ms = ?1
                WHERE id = 1
                "#,
                params![now_ms],
            )?;
            tx.commit()?;
            Ok(())
        })?;
        self.get_sync_config_state()
    }

    /// Flush passive WAL checkpoint before iOS app background suspension to prevent `0xdead10cc`.
    pub fn checkpoint_for_suspend(&self) -> CoreResult<()> {
        self.with_conn_mut(|conn| {
            conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))
                .map_err(CoreError::from)
        })
    }

    /// Resolve a display label for known providers without duplicating tables across platforms.
    pub fn provider_display_name(provider_id: &str) -> &str {
        provider_ids::display_name(provider_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SyncHourlyDelta, SyncSubscriptionSnapshot};
    use crate::sync_crypto::encrypt_sync_envelope_v1;
    use chrono::TimeZone;

    fn sample_envelope(device_id: &str, display_name: &str, base_ms: i64) -> SyncEnvelopeV1 {
        SyncEnvelopeV1 {
            schema_version: 1,
            device_id: device_id.to_string(),
            device_display_name: display_name.to_string(),
            generated_at_ms: base_ms + 5_000,
            since_watermark_ms: 0,
            hourly_deltas: vec![SyncHourlyDelta {
                provider_id: "claude_code".to_string(),
                model_id: "claude-sonnet-4-5".to_string(),
                bucket_start_ms: base_ms,
                request_count: 5,
                observable_step_count: 0,
                input_tokens: 40_000,
                output_tokens: 10_000,
                cache_read_tokens: 20_000,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                estimated_cost_micro_usd: 350_000,
                cache_saved_micro_usd: 54_000,
                total_duration_ms: 15_000,
                tps_sample_count: 5,
                tps_sum: 325.0,
                max_output_tps: 78.0,
                updated_at_ms: base_ms + 4_000,
            }],
            subscription_snapshots: vec![SyncSubscriptionSnapshot {
                plan_id: "claude_max_5x".to_string(),
                provider_id: "claude_code".to_string(),
                plan_name: "Claude Max 5x".to_string(),
                window_id: "5h".to_string(),
                window_label: "5h 滚动窗口".to_string(),
                used_percent: 64.5,
                resets_at_ms: Some(base_ms + 3 * HOUR_MS),
                source: "local_estimation".to_string(),
            }],
        }
    }

    #[test]
    fn ingest_plaintext_and_e2ee_pull_idempotent_and_widget_snapshot_compact() {
        let storage = Storage::open_in_memory().expect("open memory");
        let base_ms = chrono::Utc
            .with_ymd_and_hms(2026, 11, 16, 4, 0, 0)
            .single()
            .expect("ts")
            .timestamp_millis();

        let env_plain = sample_envelope("win-workstation", "Windows-Workstation", base_ms);
        let env_enc_1 = sample_envelope("macbook-pro", "MacBook-Pro", base_ms);
        let env_enc_2 = SyncEnvelopeV1 {
            generated_at_ms: base_ms + HOUR_MS + 5_000,
            hourly_deltas: vec![SyncHourlyDelta {
                bucket_start_ms: base_ms + HOUR_MS,
                updated_at_ms: base_ms + HOUR_MS + 4_000,
                ..env_enc_1.hourly_deltas[0].clone()
            }],
            ..env_enc_1.clone()
        };

        let salt = "abcdef0123456789abcdef0123456789";
        let passphrase = "team-sync-passphrase";
        let enc_1 =
            encrypt_sync_envelope_v1(&env_enc_1, passphrase, Some(salt), 200).expect("encrypt 1");
        let enc_2 =
            encrypt_sync_envelope_v1(&env_enc_2, passphrase, Some(salt), 300).expect("encrypt 2");

        let pull_resp = SyncPullResponseV1 {
            ok: true,
            error: None,
            server_seq_ms: 300,
            next_since_ms: 301,
            has_more: false,
            items: vec![
                SyncPullItemV1 {
                    mode: "plaintext".to_string(),
                    seq_ms: 100,
                    device_id: "win-workstation".to_string(),
                    envelope: Some(env_plain),
                    encrypted_envelope: None,
                },
                SyncPullItemV1 {
                    mode: "e2ee".to_string(),
                    seq_ms: 200,
                    device_id: "macbook-pro".to_string(),
                    envelope: None,
                    encrypted_envelope: Some(enc_1),
                },
                SyncPullItemV1 {
                    mode: "e2ee".to_string(),
                    seq_ms: 300,
                    device_id: "macbook-pro".to_string(),
                    envelope: None,
                    encrypted_envelope: Some(enc_2),
                },
            ],
        };
        let raw_json = serde_json::to_string(&pull_resp).expect("serialize pull");

        // Wrong passphrase must fail before advancing pull watermark.
        assert!(storage
            .ingest_sync_pull_payload(&raw_json, Some("wrong-pass"), base_ms + HOUR_MS)
            .is_err());
        assert_eq!(
            storage
                .get_sync_config_state()
                .expect("cfg")
                .last_pull_watermark_ms,
            0
        );

        let report = storage
            .ingest_sync_pull_payload(&raw_json, Some(passphrase), base_ms + HOUR_MS + 10_000)
            .expect("ingest pull");
        assert_eq!(report.items_received, 3);
        assert_eq!(report.envelopes_applied, 3);
        assert_eq!(report.hourly_buckets_applied, 3);
        assert_eq!(report.applied_device_ids.len(), 2);
        assert_eq!(
            storage
                .get_sync_config_state()
                .expect("cfg")
                .last_pull_watermark_ms,
            301
        );

        // Re-ingesting the same pull response must be idempotent without inflating totals.
        let _ = storage
            .ingest_sync_pull_payload(&raw_json, Some(passphrase), base_ms + HOUR_MS + 20_000)
            .expect("re-ingest");

        let bootstrap = storage
            .get_mobile_bootstrap(base_ms + HOUR_MS + 20_000, 480)
            .expect("bootstrap");
        assert_eq!(bootstrap.synced_devices.len(), 2);
        assert_eq!(bootstrap.remote_subscriptions.len(), 2);

        let snapshot = storage
            .build_mobile_widget_snapshot(base_ms + HOUR_MS + 20_000, 480)
            .expect("snapshot");
        assert_eq!(snapshot.today_tokens, 210_000);
        assert_eq!(snapshot.today_cost_micro_usd, 1_050_000);
        assert_eq!(snapshot.active_device_count, 2);
        assert_eq!(snapshot.top_providers.len(), 1);
        assert_eq!(snapshot.primary_quotas.len(), 2);
        assert!(snapshot.latest_output_tps.is_some());

        let snapshot_bytes = serde_json::to_vec(&snapshot).expect("snapshot json");
        assert!(
            snapshot_bytes.len() < 8192,
            "Widget snapshot exceeded 8KB ceiling: {} bytes",
            snapshot_bytes.len()
        );

        // Apply exchange rate JSON & update mobile preferences.
        let rate = storage
            .apply_remote_exchange_rate_payload(
                "frankfurter",
                r#"{"amount":1.0,"base":"USD","rates":{"CNY":7.2456}}"#,
                base_ms + 30_000,
            )
            .expect("apply rate");
        assert!((rate - 7.2456).abs() < 1e-6);

        let patched_prefs = storage
            .update_mobile_preferences(
                &MobilePreferencesPatch {
                    currency: Some(CurrencyCode::Cny),
                    theme: Some("light".to_string()),
                    ..Default::default()
                },
                base_ms + 40_000,
            )
            .expect("patch prefs");
        assert_eq!(patched_prefs.currency, CurrencyCode::Cny);
        assert_eq!(patched_prefs.theme, "light");

        // WAL passive checkpoint and cache reset.
        storage
            .checkpoint_for_suspend()
            .expect("checkpoint_for_suspend");

        let reset_cfg = storage
            .reset_remote_sync_cache(base_ms + 50_000)
            .expect("reset cache");
        assert_eq!(reset_cfg.last_pull_watermark_ms, 0);

        let after_reset = storage
            .get_mobile_bootstrap(base_ms + 50_000, 480)
            .expect("bootstrap after reset");
        assert!(after_reset.synced_devices.is_empty());
        assert!(after_reset.remote_subscriptions.is_empty());
    }
}
