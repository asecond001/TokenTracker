#![cfg_attr(not(test), deny(clippy::unwrap_used))]

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tokvane_core::analytics::{
    export_monthly_csv, export_monthly_json, AnalyticsFilter, MonthlyQuery, TrendsQuery,
};
use tokvane_core::domain::{ModelPricingEntry, PaygAccount, SubscriptionPlan};
use tokvane_core::pricing::{parse_litellm_catalog, parse_models_dev_catalog};
use tokvane_core::subscription::parse_subscription_catalog_json;
use tokvane_core::sync_mobile::{
    build_cloudflare_worker_upload_bundle, MobilePreferencesPatch, SyncPullIngestReport,
};
use tokvane_core::{CoreError, Storage};

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("Storage error: {msg}")]
    Storage { msg: String },
    #[error("Validation error: {msg}")]
    Validation { msg: String },
    #[error("Serialization error: {msg}")]
    Serialization { msg: String },
    #[error("Crypto error: {msg}")]
    Crypto { msg: String },
}

impl From<CoreError> for FfiError {
    fn from(value: CoreError) -> Self {
        match value {
            CoreError::Sqlite(err) => Self::Storage {
                msg: err.to_string(),
            },
            CoreError::TimeOverflow(msg) => Self::Storage { msg },
            CoreError::Json(err) => Self::Serialization {
                msg: err.to_string(),
            },
            CoreError::Decimal(err) => Self::Serialization {
                msg: err.to_string(),
            },
            CoreError::NotFound(msg) => Self::Validation { msg },
            CoreError::Validation(msg) => {
                if msg.contains("E2EE") || msg.contains("passphrase") || msg.contains("AES-256-GCM") {
                    Self::Crypto { msg }
                } else {
                    Self::Validation { msg }
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiSyncIngestOutcome {
    pub items_received: u64,
    pub envelopes_applied: u64,
    pub hourly_buckets_applied: u64,
    pub subscription_snapshots_applied: u64,
    pub payg_snapshots_applied: u64,
    pub next_since_ms: i64,
    pub server_seq_ms: i64,
    pub has_more: bool,
    pub applied_device_ids: Vec<String>,
}

impl From<SyncPullIngestReport> for FfiSyncIngestOutcome {
    fn from(value: SyncPullIngestReport) -> Self {
        Self {
            items_received: value.items_received as u64,
            envelopes_applied: value.envelopes_applied as u64,
            hourly_buckets_applied: value.hourly_buckets_applied as u64,
            subscription_snapshots_applied: value.subscription_snapshots_applied as u64,
            payg_snapshots_applied: value.payg_snapshots_applied as u64,
            next_since_ms: value.next_since_ms,
            server_seq_ms: value.server_seq_ms,
            has_more: value.has_more,
            applied_device_ids: value.applied_device_ids,
        }
    }
}

fn to_json<T: Serialize>(value: &T) -> Result<String, FfiError> {
    serde_json::to_string(value).map_err(|err| FfiError::Serialization {
        msg: err.to_string(),
    })
}

fn parse_json<T: for<'de> serde::Deserialize<'de>>(raw: &str) -> Result<T, FfiError> {
    serde_json::from_str(raw).map_err(|err| FfiError::Serialization {
        msg: err.to_string(),
    })
}

#[derive(uniffi::Object)]
pub struct MobileCoreEngine {
    storage: Storage,
}

#[uniffi::export]
impl MobileCoreEngine {
    #[uniffi::constructor]
    pub fn open(db_path: String, local_device_name: String) -> Result<Arc<Self>, FfiError> {
        let trimmed = db_path.trim();
        if trimmed.is_empty() {
            return Err(FfiError::Validation {
                msg: "Database path cannot be empty".to_string(),
            });
        }
        let storage = Storage::open_mobile_file(Path::new(trimmed), &local_device_name)?;
        Ok(Arc::new(Self { storage }))
    }

    pub fn get_bootstrap_json(
        &self,
        now_ms: i64,
        tz_offset_minutes: i32,
    ) -> Result<String, FfiError> {
        let bootstrap = self.storage.get_mobile_bootstrap(now_ms, tz_offset_minutes)?;
        to_json(&bootstrap)
    }

    pub fn ingest_pull_response_json(
        &self,
        pull_body_json: String,
        e2ee_passphrase: Option<String>,
        now_ms: i64,
    ) -> Result<FfiSyncIngestOutcome, FfiError> {
        let report = self.storage.ingest_sync_pull_payload(
            &pull_body_json,
            e2ee_passphrase.as_deref(),
            now_ms,
        )?;
        Ok(report.into())
    }

    pub fn query_overview_json(
        &self,
        filter_json: String,
        now_ms: i64,
    ) -> Result<String, FfiError> {
        let filter: AnalyticsFilter = parse_json(&filter_json)?;
        let report = self.storage.query_overview(&filter, now_ms)?;
        to_json(&report)
    }

    pub fn query_trends_json(&self, query_json: String) -> Result<String, FfiError> {
        let query: TrendsQuery = parse_json(&query_json)?;
        let report = self.storage.query_trends(&query)?;
        to_json(&report)
    }

    pub fn query_monthly_json(&self, query_json: String) -> Result<String, FfiError> {
        let query: MonthlyQuery = parse_json(&query_json)?;
        let report = self.storage.query_monthly(&query)?;
        to_json(&report)
    }

    pub fn query_models_pricing_json(&self, filter_json: String) -> Result<String, FfiError> {
        let filter: AnalyticsFilter = parse_json(&filter_json)?;
        let report = self.storage.query_models_pricing(&filter)?;
        to_json(&report)
    }

    pub fn query_speed_latency_json(&self, filter_json: String) -> Result<String, FfiError> {
        let filter: AnalyticsFilter = parse_json(&filter_json)?;
        let report = self.storage.query_speed_latency(&filter)?;
        to_json(&report)
    }

    pub fn query_remote_subscriptions_json(&self) -> Result<String, FfiError> {
        let items = self.storage.list_remote_subscription_snapshots()?;
        to_json(&items)
    }

    pub fn build_widget_snapshot_json(
        &self,
        now_ms: i64,
        tz_offset_minutes: i32,
    ) -> Result<String, FfiError> {
        let snapshot = self
            .storage
            .build_mobile_widget_snapshot(now_ms, tz_offset_minutes)?;
        to_json(&snapshot)
    }

    pub fn export_monthly_report(
        &self,
        query_json: String,
        format: String,
    ) -> Result<String, FfiError> {
        let query: MonthlyQuery = parse_json(&query_json)?;
        let report = self.storage.query_monthly(&query)?;
        match format.trim().to_ascii_lowercase().as_str() {
            "csv" => Ok(export_monthly_csv(&report)),
            "json" => export_monthly_json(&report).map_err(FfiError::from),
            other => Err(FfiError::Validation {
                msg: format!("Unsupported export format '{other}': expected csv or json"),
            }),
        }
    }

    pub fn update_mobile_preferences_json(
        &self,
        patch_json: String,
        now_ms: i64,
    ) -> Result<String, FfiError> {
        let patch: MobilePreferencesPatch = parse_json(&patch_json)?;
        let updated = self.storage.update_mobile_preferences(&patch, now_ms)?;
        to_json(&updated)
    }

    pub fn update_sync_config(
        &self,
        enabled: bool,
        worker_url: Option<String>,
        e2ee_enabled: bool,
        now_ms: i64,
    ) -> Result<String, FfiError> {
        let _ = now_ms;
        let state = self.storage.update_sync_config_state(
            enabled,
            worker_url.as_deref(),
            e2ee_enabled,
            None,
        )?;
        to_json(&state)
    }

    pub fn reset_sync_watermark(&self, now_ms: i64) -> Result<String, FfiError> {
        let state = self.storage.reset_remote_sync_cache(now_ms)?;
        to_json(&state)
    }

    pub fn apply_exchange_rate_payload(
        &self,
        source: String,
        body_json: String,
        now_ms: i64,
    ) -> Result<f64, FfiError> {
        self.storage
            .apply_remote_exchange_rate_payload(&source, &body_json, now_ms)
            .map_err(FfiError::from)
    }

    pub fn checkpoint_for_suspend(&self) -> Result<(), FfiError> {
        self.storage
            .checkpoint_for_suspend()
            .map_err(FfiError::from)
    }

    pub fn apply_pricing_catalog_payload(
        &self,
        source: String,
        body_json: String,
        effective_date: String,
        now_ms: i64,
    ) -> Result<u64, FfiError> {
        let entries = match source.trim().to_ascii_lowercase().as_str() {
            "models_dev" | "models.dev" => parse_models_dev_catalog(&body_json, &effective_date)?,
            "litellm" => parse_litellm_catalog(&body_json, &effective_date)?,
            other => {
                return Err(FfiError::Validation {
                    msg: format!("Unsupported pricing catalog source '{other}'"),
                });
            }
        };
        let (_, count, _) = self
            .storage
            .apply_synced_pricing_catalog(&entries, now_ms)?;
        Ok(count as u64)
    }

    pub fn apply_subscription_catalog_payload(
        &self,
        body_json: String,
        now_ms: i64,
    ) -> Result<u64, FfiError> {
        let templates = parse_subscription_catalog_json(&body_json)?;
        let (_, count) = self
            .storage
            .apply_synced_subscription_catalog(&templates, now_ms)?;
        Ok(count as u64)
    }

    pub fn upsert_model_pricing_json(&self, entry_json: String) -> Result<(), FfiError> {
        let entry: ModelPricingEntry = parse_json(&entry_json)?;
        self.storage
            .upsert_model_pricing(&entry)
            .map_err(FfiError::from)
    }

    pub fn delete_model_pricing_override(&self, model_pattern: String) -> Result<(), FfiError> {
        self.storage
            .delete_model_pricing_override(&model_pattern)
            .map_err(FfiError::from)
    }

    pub fn query_payg_accounts_json(
        &self,
        now_ms: i64,
        tz_offset_minutes: i32,
    ) -> Result<String, FfiError> {
        let items = self.storage.query_payg_accounts(now_ms, tz_offset_minutes)?;
        to_json(&items)
    }

    pub fn upsert_payg_account_json(&self, account_json: String) -> Result<(), FfiError> {
        let account: PaygAccount = parse_json(&account_json)?;
        self.storage
            .upsert_payg_account(&account)
            .map_err(FfiError::from)
    }

    pub fn delete_payg_account(&self, account_id: String) -> Result<(), FfiError> {
        self.storage
            .delete_payg_account(&account_id)
            .map_err(FfiError::from)
    }

    pub fn apply_official_balance_payload_json(
        &self,
        provider_id: String,
        endpoint: String,
        body_json: String,
        now_ms: i64,
    ) -> Result<String, FfiError> {
        let result = self.storage.apply_official_balance_payload(
            &provider_id,
            &endpoint,
            &body_json,
            now_ms,
        )?;
        to_json(&result)
    }

    pub fn list_subscription_templates_json(&self, now_ms: i64) -> Result<String, FfiError> {
        let items = self.storage.query_subscription_templates(now_ms)?;
        to_json(&items)
    }

    pub fn query_local_subscriptions_json(
        &self,
        now_ms: i64,
        tz_offset_minutes: i32,
    ) -> Result<String, FfiError> {
        let items = self
            .storage
            .query_subscriptions_with_tz(now_ms, tz_offset_minutes)?;
        to_json(&items)
    }

    pub fn upsert_subscription_plan_json(&self, plan_json: String) -> Result<(), FfiError> {
        let plan: SubscriptionPlan = parse_json(&plan_json)?;
        self.storage
            .upsert_subscription_plan(&plan)
            .map_err(FfiError::from)
    }

    pub fn delete_subscription_plan(&self, plan_id: String) -> Result<(), FfiError> {
        self.storage
            .delete_subscription_plan(&plan_id)
            .map_err(FfiError::from)
    }

    pub fn build_worker_upload_bundle_json(
        &self,
        worker_name: Option<String>,
        bearer_token: Option<String>,
        e2ee_enabled: bool,
        e2ee_passphrase: Option<String>,
        include_migration: bool,
    ) -> Result<String, FfiError> {
        let bundle = build_cloudflare_worker_upload_bundle(
            worker_name.as_deref(),
            bearer_token.as_deref(),
            e2ee_enabled,
            e2ee_passphrase.as_deref(),
            include_migration,
        )?;
        to_json(&bundle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tokvane_core::analytics::{
        MonthlyReport, OverviewReport, SpeedLatencyReport, TrendsReport,
    };
    use tokvane_core::domain::{
        RemoteSubscriptionSnapshot, SyncEnvelopeV1, SyncHourlyDelta, SyncSubscriptionSnapshot,
    };
    use tokvane_core::sync_mobile::{SyncPullItemV1, SyncPullResponseV1};

    struct TempDbGuard(PathBuf);

    impl Drop for TempDbGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_db_path(name: &str) -> (TempDbGuard, String) {
        let dir = std::env::temp_dir().join(format!(
            "tokvane_ffi_test_{}_{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("mobile.sqlite");
        (TempDbGuard(dir), db.to_string_lossy().into_owned())
    }

    #[test]
    fn ffi_engine_roundtrip_queries_and_export() {
        let (_guard, db_path) = temp_db_path("roundtrip");
        let engine = MobileCoreEngine::open(db_path, "iPhone-Test".to_string()).expect("open");

        let base_ms: i64 = 1_763_265_600_000; // 2025-11-16T04:00:00Z
        let pull = SyncPullResponseV1 {
            ok: true,
            error: None,
            server_seq_ms: 500,
            next_since_ms: 501,
            has_more: false,
            items: vec![SyncPullItemV1 {
                mode: "plaintext".to_string(),
                seq_ms: 500,
                device_id: "desktop-dev".to_string(),
                envelope: Some(SyncEnvelopeV1 {
                    schema_version: 1,
                    device_id: "desktop-dev".to_string(),
                    device_display_name: "Dev-Workstation".to_string(),
                    generated_at_ms: base_ms + 1_000,
                    since_watermark_ms: 0,
                    hourly_deltas: vec![SyncHourlyDelta {
                        provider_id: "codex".to_string(),
                        model_id: "gpt-5.3-codex".to_string(),
                        bucket_start_ms: base_ms,
                        request_count: 2,
                        observable_step_count: 0,
                        input_tokens: 10_000,
                        output_tokens: 2_500,
                        cache_read_tokens: 5_000,
                        cache_write_tokens: 0,
                        reasoning_tokens: 1_000,
                        estimated_cost_micro_usd: 120_000,
                        cache_saved_micro_usd: 20_000,
                        total_duration_ms: 4_000,
                        tps_sample_count: 2,
                        tps_sum: 130.0,
                        max_output_tps: 72.0,
                        updated_at_ms: base_ms + 900,
                    }],
                    subscription_snapshots: vec![SyncSubscriptionSnapshot {
                        plan_id: "chatgpt_pro".to_string(),
                        provider_id: "codex".to_string(),
                        plan_name: "ChatGPT Pro".to_string(),
                        window_id: "5h".to_string(),
                        window_label: "5h 滚动窗口".to_string(),
                        used_percent: 42.0,
                        resets_at_ms: Some(base_ms + 3_600_000),
                        source: "local_estimation".to_string(),
                        monthly_fee_micro_usd: 200_000_000,
                    }],
                    payg_snapshots: vec![tokvane_core::domain::SyncPaygSnapshot {
                        account_id: "auto_deepseek_harness".to_string(),
                        provider_id: "deepseek_harness".to_string(),
                        account_name: "DeepSeek".to_string(),
                        enabled: true,
                        currency: Some("CNY".to_string()),
                        total_balance_micro: Some(12_000_000),
                        granted_balance_micro: None,
                        topped_up_balance_micro: None,
                        remaining_manual_balance_micro_usd: None,
                        monthly_budget_micro_usd: None,
                        today_cost_micro_usd: 0,
                        month_cost_micro_usd: 0,
                        fetched_at_ms: Some(base_ms),
                        updated_at_ms: base_ms,
                    }],
                }),
                encrypted_envelope: None,
            }],
        };

        let pull_json = serde_json::to_string(&pull).expect("serialize pull");
        let outcome = engine
            .ingest_pull_response_json(pull_json, None, base_ms + 2_000)
            .expect("ingest");
        assert_eq!(outcome.envelopes_applied, 1);
        assert_eq!(outcome.hourly_buckets_applied, 1);
        assert_eq!(outcome.payg_snapshots_applied, 1);

        let bootstrap_json = engine
            .get_bootstrap_json(base_ms + 5_000, 480)
            .expect("bootstrap");
        assert!(bootstrap_json.contains("Dev-Workstation"));

        let filter_json = serde_json::json!({
            "start_ms": base_ms - 3_600_000,
            "end_ms": base_ms + 3_600_000,
            "provider_id": null,
            "device_id": null,
            "tz_offset_minutes": 480,
            "all_time": false
        })
        .to_string();

        let ov: OverviewReport = serde_json::from_str(
            &engine
                .query_overview_json(filter_json.clone(), base_ms + 5_000)
                .expect("overview"),
        )
        .expect("parse overview");
        assert_eq!(ov.total_tokens, 18_500);

        let speed: SpeedLatencyReport = serde_json::from_str(
            &engine
                .query_speed_latency_json(filter_json.clone())
                .expect("speed"),
        )
        .expect("parse speed");
        assert!(speed.from_hourly_rollups);
        assert_eq!(speed.providers.len(), 1);

        let trends: TrendsReport = serde_json::from_str(
            &engine
                .query_trends_json(
                    serde_json::json!({
                        "filter": serde_json::from_str::<serde_json::Value>(&filter_json).expect("v"),
                        "granularity": "fifteen_minutes",
                        "dimension": "provider"
                    })
                    .to_string(),
                )
                .expect("trends"),
        )
        .expect("parse trends");
        assert_eq!(trends.series.len(), 1);

        let subs: Vec<RemoteSubscriptionSnapshot> = serde_json::from_str(
            &engine
                .query_remote_subscriptions_json()
                .expect("subscriptions"),
        )
        .expect("parse subs");
        assert_eq!(subs.len(), 1);

        let monthly_query = serde_json::json!({
            "year": 2025,
            "month": 11,
            "provider_id": null,
            "device_id": null,
            "tz_offset_minutes": 480
        })
        .to_string();
        let monthly: MonthlyReport = serde_json::from_str(
            &engine
                .query_monthly_json(monthly_query.clone())
                .expect("monthly"),
        )
        .expect("parse monthly");
        assert_eq!(monthly.month_total_tokens, 18_500);
        assert_eq!(monthly.subscription_monthly_fee_micro_usd, 200_000_000);

        // New v0.4.6 parity FFI endpoints: PAYG, official balance ingest, pricing override,
        // local subscription plan & templates, and Cloudflare Worker upload bundle.
        engine
            .upsert_payg_account_json(
                serde_json::json!({
                    "account_id": "ds_ios",
                    "provider_id": "deepseek_harness",
                    "account_name": "DeepSeek iOS",
                    "enabled": true,
                    "manual_balance_micro_usd": 5_000_000,
                    "balance_anchor_ms": base_ms,
                    "monthly_budget_micro_usd": 20_000_000,
                    "is_auto": false,
                    "updated_at_ms": base_ms
                })
                .to_string(),
            )
            .expect("upsert payg");
        let payg_json = engine
            .query_payg_accounts_json(base_ms + 5_000, 480)
            .expect("query payg");
        assert!(payg_json.contains("ds_ios"));
        let bal_json = engine
            .apply_official_balance_payload_json(
                "deepseek_harness".to_string(),
                "https://api.deepseek.com/user/balance".to_string(),
                r#"{"balance_infos":[{"currency":"CNY","total_balance":"19.5"}]}"#.to_string(),
                base_ms + 6_000,
            )
            .expect("official bal");
        assert!(bal_json.contains("19500000"));
        engine
            .delete_payg_account("ds_ios".to_string())
            .expect("delete payg");

        engine
            .upsert_model_pricing_json(
                serde_json::json!({
                    "model_pattern": "custom-test-model",
                    "provider_id": "codex",
                    "input_per_m_micro_usd": 1_000_000,
                    "output_per_m_micro_usd": 2_000_000,
                    "cache_read_per_m_micro_usd": 100_000,
                    "cache_write_per_m_micro_usd": 0,
                    "reasoning_per_m_micro_usd": 2_000_000,
                    "is_user_override": true,
                    "effective_from": "2026-11-01"
                })
                .to_string(),
            )
            .expect("upsert pricing");
        engine
            .delete_model_pricing_override("custom-test-model".to_string())
            .expect("delete pricing");

        let tpls_json = engine
            .list_subscription_templates_json(base_ms + 6_000)
            .expect("templates");
        assert!(tpls_json.contains("claude_pro"));
        engine
            .upsert_subscription_plan_json(
                serde_json::json!({
                    "plan_id": "ios_custom_plan",
                    "provider_id": "claude_code",
                    "plan_name": "Claude Pro iOS",
                    "enabled": true,
                    "monthly_fee_micro_usd": 20_000_000,
                    "windows": [],
                    "is_user_configured": true,
                    "auto_source": null,
                    "updated_at_ms": base_ms + 6_000
                })
                .to_string(),
            )
            .expect("upsert sub");
        let local_subs_json = engine
            .query_local_subscriptions_json(base_ms + 6_000, 480)
            .expect("local subs");
        assert!(local_subs_json.contains("ios_custom_plan"));
        engine
            .delete_subscription_plan("ios_custom_plan".to_string())
            .expect("delete sub");

        let worker_bundle = engine
            .build_worker_upload_bundle_json(None, None, true, None, true)
            .expect("worker bundle");
        assert!(worker_bundle.contains("tokenvane-sync-worker"));
        assert!(worker_bundle.contains("multipart_body_base64"));

        let csv = engine
            .export_monthly_report(monthly_query, "csv".to_string())
            .expect("export csv");
        assert!(csv.contains("date"));

        let snap = engine
            .build_widget_snapshot_json(base_ms + 5_000, 480)
            .expect("snapshot");
        assert!(snap.len() < 8192);

        let cfg_json = engine
            .update_sync_config(
                true,
                Some("https://sync.example.workers.dev".to_string()),
                true,
                base_ms + 6_000,
            )
            .expect("update sync config");
        assert!(cfg_json.contains("sync.example.workers.dev"));

        let reset_json = engine
            .reset_sync_watermark(base_ms + 7_000)
            .expect("reset");
        assert!(reset_json.contains("\"last_pull_watermark_ms\":0"));

        engine.checkpoint_for_suspend().expect("checkpoint");
    }
}
