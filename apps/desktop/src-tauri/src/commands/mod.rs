use std::ffi::OsStr;
use std::path::PathBuf;

use chrono::Utc;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tokentracker_collectors::providers::official_api::{
    reconcile_official_usage_api, OfficialApiReconcileResult,
};
use tokentracker_collectors::{
    build_sync_pairing_uri, clear_sync_credentials as clear_sync_creds, has_sync_credentials,
    normalize_worker_url, refresh_provider_balance as collect_provider_balance,
    refresh_provider_balance_force as collect_provider_balance_force,
    set_sync_credentials as set_sync_creds, BalanceRefreshOutcome, ScanReport, SyncRunReport,
    SyncStatusView, SYNC_BEARER_TOKEN_KEY, SYNC_E2EE_PASSPHRASE_KEY,
};
use tokentracker_core::analytics::{
    export_monthly_csv, export_monthly_json, AnalyticsFilter, ModelStatsReport, MonthlyQuery,
    MonthlyReport, OverviewReport, SpeedLatencyReport, TraySummary, TrendsQuery, TrendsReport,
};
use tokentracker_core::declarative::test_custom_rule_line;
use tokentracker_core::domain::{
    AppPreferences, CatalogSyncResult, CustomProviderRule, CustomRuleTestResult, DeviceRecord,
    IngestionCursor, ModelPricingEntry, ObservedRateLimitWindow, PaygAccount, PaygAccountStatus,
    ProviderBalanceSnapshot, ProviderConfig, RemoteSubscriptionSnapshot, SubscriptionPlan,
    SubscriptionPlanTemplate, SyncEnvelopeV1,
};
use tokentracker_core::subscription::SubscriptionPlanStatus;
use tokentracker_core::sync_crypto::generate_kdf_salt_hex;
use tokentracker_core::CoreError;

use crate::state::AppState;
use crate::tray::{
    resize_floating_window_internal, set_floating_window_visible, show_main_window,
    toggle_floating_window_with, FLOATING_MONITOR_LABEL, TRAY_POPUP_LABEL,
};

const MAX_BG_IMAGE_BYTES: u64 = 15_728_640; // 15 MiB

pub fn parse_tauri_theme(theme: &str) -> Option<tauri::Theme> {
    match theme.trim().to_ascii_lowercase().as_str() {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => None,
    }
}

pub fn apply_theme_to_app(app: &AppHandle, theme: &str) {
    let target = parse_tauri_theme(theme);
    app.set_theme(target);
    for label in ["main", TRAY_POPUP_LABEL, FLOATING_MONITOR_LABEL] {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.set_theme(target);
        }
    }
}

fn managed_backgrounds_dir(state: &AppState) -> PathBuf {
    state
        .db_path
        .parent()
        .map(|p| p.join("backgrounds"))
        .unwrap_or_else(|| PathBuf::from("backgrounds"))
}

fn is_valid_managed_filename(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && !name.contains(':')
}

fn detect_image_format(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(("png", "image/png"))
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("jpg", "image/jpeg"))
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("webp", "image/webp"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("gif", "image/gif"))
    } else if bytes.starts_with(b"BM") {
        Some(("bmp", "image/bmp"))
    } else {
        None
    }
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn cleanup_backgrounds_dir(dir: &PathBuf, keep_filename: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(keep) = keep_filename {
            if entry.file_name() == OsStr::new(keep) {
                continue;
            }
        }
        let _ = std::fs::remove_file(path);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IpcError {
    pub code: String,
    pub message: String,
}

impl IpcError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

impl From<CoreError> for IpcError {
    fn from(err: CoreError) -> Self {
        let code = match err {
            CoreError::Sqlite(_) => "DB_ERROR",
            CoreError::Validation(_) => "VALIDATION_ERROR",
            CoreError::NotFound(_) => "NOT_FOUND",
            CoreError::Json(_) => "SERIALIZATION_ERROR",
            CoreError::Decimal(_) => "PRICING_MATH_ERROR",
            CoreError::TimeOverflow(_) => "TIME_ERROR",
        };
        Self::new(code, err.to_string())
    }
}

impl From<tokentracker_collectors::CollectorError> for IpcError {
    fn from(err: tokentracker_collectors::CollectorError) -> Self {
        Self::new("COLLECTOR_ERROR", err.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapState {
    pub db_path: String,
    pub device: DeviceRecord,
    pub preferences: AppPreferences,
    pub providers: Vec<ProviderConfig>,
    pub collector_watching: bool,
    pub otlp_running: bool,
    pub otlp_port: Option<u16>,
    pub last_scan: Option<ScanReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportReportResult {
    pub path: Option<String>,
    pub content: String,
    pub bytes: usize,
}

#[tauri::command]
pub fn get_bootstrap_state(state: State<'_, AppState>) -> Result<BootstrapState, IpcError> {
    let device = state.storage.local_device()?;
    let preferences = state.storage.get_preferences()?;
    let providers = state.storage.list_provider_configs()?;
    let (otlp_running, otlp_port) = state.otlp_status();

    Ok(BootstrapState {
        db_path: state.db_path.to_string_lossy().to_string(),
        device,
        preferences,
        providers,
        collector_watching: state.coordinator.is_watching(),
        otlp_running,
        otlp_port,
        last_scan: state.coordinator.last_scan_report(),
    })
}

#[tauri::command]
pub fn get_overview(
    state: State<'_, AppState>,
    filter: AnalyticsFilter,
) -> Result<OverviewReport, IpcError> {
    let now_ms = Utc::now().timestamp_millis();
    state
        .storage
        .query_overview(&filter, now_ms)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_trends(
    state: State<'_, AppState>,
    query: TrendsQuery,
) -> Result<TrendsReport, IpcError> {
    state.storage.query_trends(&query).map_err(IpcError::from)
}

#[tauri::command]
pub fn get_monthly(
    state: State<'_, AppState>,
    query: MonthlyQuery,
) -> Result<MonthlyReport, IpcError> {
    state.storage.query_monthly(&query).map_err(IpcError::from)
}

#[tauri::command]
pub fn get_models_pricing(
    state: State<'_, AppState>,
    filter: AnalyticsFilter,
) -> Result<ModelStatsReport, IpcError> {
    state
        .storage
        .query_models_pricing(&filter)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn upsert_model_pricing_override(
    state: State<'_, AppState>,
    mut entry: ModelPricingEntry,
    recompute_history: bool,
) -> Result<usize, IpcError> {
    entry.is_user_override = true;
    state.storage.upsert_model_pricing(&entry)?;
    if recompute_history {
        state
            .storage
            .recompute_all_costs_and_rollups()
            .map_err(IpcError::from)
    } else {
        Ok(0)
    }
}

#[tauri::command]
pub fn delete_model_pricing_override(
    state: State<'_, AppState>,
    model_pattern: String,
    recompute_history: bool,
) -> Result<usize, IpcError> {
    state.storage.delete_model_pricing_override(&model_pattern)?;
    if recompute_history {
        state
            .storage
            .recompute_all_costs_and_rollups()
            .map_err(IpcError::from)
    } else {
        Ok(0)
    }
}

#[tauri::command]
pub fn recompute_all_costs(state: State<'_, AppState>) -> Result<usize, IpcError> {
    state
        .storage
        .recompute_all_costs_and_rollups()
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn sync_pricing_and_rate(
    app: AppHandle,
    state: State<'_, AppState>,
    force: bool,
) -> Result<CatalogSyncResult, IpcError> {
    state
        .run_catalog_sync(&app, force)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_subscriptions(
    state: State<'_, AppState>,
) -> Result<Vec<SubscriptionPlanStatus>, IpcError> {
    let now_ms = Utc::now().timestamp_millis();
    state
        .storage
        .query_subscriptions(now_ms)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn upsert_subscription_plan(
    state: State<'_, AppState>,
    plan: SubscriptionPlan,
) -> Result<(), IpcError> {
    state
        .storage
        .upsert_subscription_plan(&plan)
        .map_err(IpcError::from)?;
    state.sync_client.notify_local_change();
    Ok(())
}

#[tauri::command]
pub fn delete_subscription_plan(
    state: State<'_, AppState>,
    plan_id: String,
) -> Result<(), IpcError> {
    state
        .storage
        .delete_subscription_plan(&plan_id)
        .map_err(IpcError::from)?;
    state.sync_client.notify_local_change();
    Ok(())
}

#[tauri::command]
pub fn get_subscription_templates(
    state: State<'_, AppState>,
) -> Result<Vec<SubscriptionPlanTemplate>, IpcError> {
    let now_ms = Utc::now().timestamp_millis();
    state
        .storage
        .query_subscription_templates(now_ms)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_observed_rate_limits(
    state: State<'_, AppState>,
) -> Result<Vec<ObservedRateLimitWindow>, IpcError> {
    state
        .storage
        .list_observed_rate_limits()
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_speed_latency(
    state: State<'_, AppState>,
    filter: AnalyticsFilter,
) -> Result<SpeedLatencyReport, IpcError> {
    state
        .storage
        .query_speed_latency(&filter)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_tray_summary(
    state: State<'_, AppState>,
    tz_offset_minutes: i32,
) -> Result<TraySummary, IpcError> {
    let now_ms = Utc::now().timestamp_millis();
    state
        .storage
        .query_tray_summary(now_ms, tz_offset_minutes)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_provider_configs(state: State<'_, AppState>) -> Result<Vec<ProviderConfig>, IpcError> {
    state
        .storage
        .list_provider_configs()
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn update_provider_config(
    state: State<'_, AppState>,
    config: ProviderConfig,
) -> Result<(), IpcError> {
    state
        .storage
        .upsert_provider_config(&config)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_ingestion_cursors(state: State<'_, AppState>) -> Result<Vec<IngestionCursor>, IpcError> {
    state.storage.list_cursors().map_err(IpcError::from)
}

#[tauri::command]
pub async fn trigger_scan_now(state: State<'_, AppState>) -> Result<ScanReport, IpcError> {
    let coord = state.coordinator.clone();
    let report = tauri::async_runtime::spawn_blocking(move || coord.scan_now())
        .await
        .map_err(|e| IpcError::new("TASK_JOIN_ERROR", e.to_string()))?
        .map_err(IpcError::from)?;
    if report.events_inserted > 0 || report.rate_limits_updated > 0 {
        state.sync_client.notify_local_change();
    }
    Ok(report)
}

#[tauri::command]
pub async fn set_provider_api_key(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    api_key: String,
) -> Result<(), IpcError> {
    state.credentials.set_api_key(&provider_id, &api_key)?;
    state
        .storage
        .set_provider_has_credential(&provider_id, true)?;
    state
        .storage
        .reset_auto_subscription_dismissal(&provider_id)?;

    if matches!(
        provider_id.as_str(),
        "kimi_code" | "deepseek_harness" | "qcode" | "minimax_code"
    ) {
        match collect_provider_balance_force(
            &state.storage,
            &provider_id,
            state.credentials.as_ref(),
        )
        .await
        {
            Ok(outcome) => {
                let _ = state.storage.set_provider_sync_status(
                    &provider_id,
                    Some(outcome.snapshot.fetched_at_ms),
                    None,
                );
                if outcome.rate_limits_updated > 0 {
                    state.sync_client.notify_local_change();
                }
            }
            Err(err) => {
                let _ = state.storage.set_provider_sync_status(
                    &provider_id,
                    Some(Utc::now().timestamp_millis()),
                    Some(&err.to_string()),
                );
            }
        }
    }

    let _ = app.emit("usage-updated", &provider_id);
    Ok(())
}

#[tauri::command]
pub fn delete_provider_api_key(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<(), IpcError> {
    state.credentials.delete_api_key(&provider_id)?;
    state
        .storage
        .set_provider_has_credential(&provider_id, false)?;
    state
        .storage
        .clear_provider_api_artifacts(&provider_id)?;
    let _ = state
        .storage
        .set_provider_sync_status(&provider_id, None, None);
    let _ = app.emit("usage-updated", &provider_id);
    state.sync_client.notify_local_change();
    Ok(())
}

#[tauri::command]
pub async fn reconcile_provider_official_api(
    state: State<'_, AppState>,
    provider_id: String,
    lookback_days: i64,
) -> Result<OfficialApiReconcileResult, IpcError> {
    reconcile_official_usage_api(&provider_id, state.credentials.as_ref(), lookback_days)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_preferences(state: State<'_, AppState>) -> Result<AppPreferences, IpcError> {
    state.storage.get_preferences().map_err(IpcError::from)
}

#[tauri::command]
pub async fn update_preferences(
    app: AppHandle,
    state: State<'_, AppState>,
    prefs: AppPreferences,
) -> Result<AppPreferences, IpcError> {
    state.storage.update_preferences(&prefs)?;

    if let Err(err) = state
        .sync_otlp_listener_async(prefs.otlp_enabled, prefs.otlp_port)
        .await
    {
        return Err(IpcError::new("OTLP_ERROR", err));
    }

    let autolaunch = app.autolaunch();
    let autostart_res = if prefs.launch_at_startup {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    if let Err(err) = autostart_res {
        tracing::warn!("Autostart update warning: {err}");
    }

    let updated = state.storage.get_preferences()?;
    state.sync_client.notify_local_change();
    apply_theme_to_app(&app, &updated.theme);
    set_floating_window_visible(&app, updated.floating_window_enabled);
    let _ = app.emit("preferences-updated", &updated);
    Ok(updated)
}

#[tauri::command]
pub fn import_background_image(
    app: AppHandle,
    state: State<'_, AppState>,
    source_path: String,
) -> Result<String, IpcError> {
    let trimmed = source_path.trim();
    if trimmed.is_empty() {
        return Err(IpcError::new(
            "INVALID_PATH",
            "Source image path cannot be empty.",
        ));
    }

    let src = PathBuf::from(trimmed);
    let meta = std::fs::metadata(&src).map_err(|e| {
        IpcError::new(
            "BG_IO_ERROR",
            format!("Failed to inspect selected image: {e}"),
        )
    })?;
    if !meta.is_file() {
        return Err(IpcError::new(
            "INVALID_PATH",
            "Selected background path is not a file.",
        ));
    }
    if meta.len() == 0 || meta.len() > MAX_BG_IMAGE_BYTES {
        return Err(IpcError::new(
            "INVALID_IMAGE_SIZE",
            "Background image size must be between 1 byte and 15 MiB.",
        ));
    }

    let bytes = std::fs::read(&src).map_err(|e| {
        IpcError::new("BG_IO_ERROR", format!("Failed to read background image: {e}"))
    })?;
    if bytes.is_empty() || (bytes.len() as u64) > MAX_BG_IMAGE_BYTES {
        return Err(IpcError::new(
            "INVALID_IMAGE_SIZE",
            "Background image size must be between 1 byte and 15 MiB.",
        ));
    }

    let (ext, _mime) = detect_image_format(&bytes).ok_or_else(|| {
        IpcError::new(
            "UNSUPPORTED_IMAGE_FORMAT",
            "Supported background image formats: PNG, JPEG, WEBP, GIF, BMP.",
        )
    })?;

    let bg_dir = managed_backgrounds_dir(&state);
    std::fs::create_dir_all(&bg_dir).map_err(|e| {
        IpcError::new(
            "BG_IO_ERROR",
            format!("Failed to create backgrounds directory: {e}"),
        )
    })?;

    let filename = format!("wallpaper_{}.{}", Utc::now().timestamp_millis(), ext);
    let dest_path = bg_dir.join(&filename);
    std::fs::write(&dest_path, &bytes).map_err(|e| {
        IpcError::new(
            "BG_IO_ERROR",
            format!("Failed to save background image copy: {e}"),
        )
    })?;

    cleanup_backgrounds_dir(&bg_dir, Some(&filename));

    let mut prefs = state.storage.get_preferences()?;
    prefs.bg_mode = "image".to_string();
    prefs.bg_image_path = Some(filename.clone());
    state.storage.update_preferences(&prefs)?;

    let updated = state.storage.get_preferences()?;
    let _ = app.emit("preferences-updated", &updated);
    Ok(filename)
}

#[tauri::command]
pub fn clear_background_image(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    let bg_dir = managed_backgrounds_dir(&state);
    cleanup_backgrounds_dir(&bg_dir, None);

    let mut prefs = state.storage.get_preferences()?;
    prefs.bg_image_path = None;
    if prefs.bg_mode == "image" {
        prefs.bg_mode = "none".to_string();
    }
    state.storage.update_preferences(&prefs)?;

    let updated = state.storage.get_preferences()?;
    let _ = app.emit("preferences-updated", &updated);
    Ok(())
}

#[tauri::command]
pub fn get_background_image_data_url(
    state: State<'_, AppState>,
) -> Result<Option<String>, IpcError> {
    let prefs = state.storage.get_preferences()?;
    let Some(filename) = prefs
        .bg_image_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };

    if !is_valid_managed_filename(filename) {
        return Err(IpcError::new(
            "INVALID_PATH",
            "Invalid managed background image filename.",
        ));
    }

    let full_path = managed_backgrounds_dir(&state).join(filename);
    let bytes = match std::fs::read(&full_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(IpcError::new(
                "BG_IO_ERROR",
                format!("Failed to read background image: {e}"),
            ))
        }
    };

    if bytes.is_empty() || (bytes.len() as u64) > MAX_BG_IMAGE_BYTES {
        return Ok(None);
    }
    let Some((_, mime)) = detect_image_format(&bytes) else {
        return Ok(None);
    };

    Ok(Some(format!(
        "data:{mime};base64,{}",
        encode_base64(&bytes)
    )))
}

#[tauri::command]
pub fn export_monthly_report(
    state: State<'_, AppState>,
    query: MonthlyQuery,
    format: String,
    save_path: Option<String>,
) -> Result<ExportReportResult, IpcError> {
    let report = state.storage.query_monthly(&query)?;
    let content = if format.eq_ignore_ascii_case("json") {
        export_monthly_json(&report)?
    } else {
        export_monthly_csv(&report)
    };

    let bytes = content.len();
    if let Some(ref target) = save_path {
        std::fs::write(target, content.as_bytes())
            .map_err(|e| IpcError::new("EXPORT_IO_ERROR", format!("Failed to write export: {e}")))?;
    }

    Ok(ExportReportResult {
        path: save_path,
        content,
        bytes,
    })
}

#[tauri::command]
pub fn preview_sync_envelope_v1(
    state: State<'_, AppState>,
    since_watermark_ms: i64,
) -> Result<SyncEnvelopeV1, IpcError> {
    let now_ms = Utc::now().timestamp_millis();
    state
        .storage
        .build_sync_envelope_v1(since_watermark_ms, now_ms)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn list_custom_provider_rules(
    state: State<'_, AppState>,
) -> Result<Vec<CustomProviderRule>, IpcError> {
    state
        .storage
        .list_custom_provider_rules()
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn upsert_custom_provider_rule(
    state: State<'_, AppState>,
    rule: CustomProviderRule,
) -> Result<Vec<CustomProviderRule>, IpcError> {
    state.storage.upsert_custom_provider_rule(&rule)?;
    state
        .storage
        .list_custom_provider_rules()
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn delete_custom_provider_rule(
    state: State<'_, AppState>,
    rule_id: String,
) -> Result<Vec<CustomProviderRule>, IpcError> {
    state.storage.delete_custom_provider_rule(&rule_id)?;
    state
        .storage
        .list_custom_provider_rules()
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn test_custom_provider_sample_line(
    state: State<'_, AppState>,
    rule: CustomProviderRule,
    sample_line: String,
) -> Result<CustomRuleTestResult, IpcError> {
    let catalog = state.storage.list_model_pricing()?;
    Ok(test_custom_rule_line(&rule, &sample_line, &catalog))
}

#[tauri::command]
pub fn get_provider_balances(
    state: State<'_, AppState>,
) -> Result<Vec<ProviderBalanceSnapshot>, IpcError> {
    state
        .storage
        .list_provider_balances()
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_payg_accounts(
    state: State<'_, AppState>,
    tz_offset_minutes: Option<i32>,
) -> Result<Vec<PaygAccountStatus>, IpcError> {
    let now_ms = Utc::now().timestamp_millis();
    state
        .storage
        .query_payg_accounts(now_ms, tz_offset_minutes.unwrap_or(480))
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn upsert_payg_account(
    app: AppHandle,
    state: State<'_, AppState>,
    account: PaygAccount,
) -> Result<(), IpcError> {
    state.storage.upsert_payg_account(&account)?;
    let _ = app.emit("usage-updated", &account.account_id);
    Ok(())
}

#[tauri::command]
pub fn delete_payg_account(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), IpcError> {
    state.storage.delete_payg_account(&account_id)?;
    let _ = app.emit("usage-updated", &account_id);
    Ok(())
}

#[tauri::command]
pub async fn refresh_provider_balance(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<BalanceRefreshOutcome, IpcError> {
    if provider_id.trim().eq_ignore_ascii_case("antigravity") {
        let _ = state.storage.reset_auto_subscription_dismissal(&provider_id);
    }
    let outcome = collect_provider_balance(&state.storage, &provider_id, state.credentials.as_ref())
        .await
        .map_err(IpcError::from)?;
    if !outcome.from_cache {
        let _ = state.storage.set_provider_sync_status(
            &provider_id,
            Some(outcome.snapshot.fetched_at_ms),
            None,
        );
        let _ = app.emit("usage-updated", &outcome);
        if outcome.rate_limits_updated > 0 {
            state.sync_client.notify_local_change();
        }
    }
    Ok(outcome)
}

#[tauri::command]
pub fn launch_provider_cli_login(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<String, IpcError> {
    let trimmed = provider_id.trim().to_ascii_lowercase();
    let _ = state.storage.reset_auto_subscription_dismissal(&trimmed);

    let (program, cli_args): (String, Vec<&'static str>) = match trimmed.as_str() {
        "antigravity" => {
            let bin = tokentracker_collectors::providers::antigravity::resolve_agy_cli_binary()
                .map(|p| p.to_string_lossy().into_owned())
                .ok_or_else(|| {
                    IpcError::new(
                        "CLI_LOGIN_UNAVAILABLE",
                        "请先安装官方 agy CLI（%LOCALAPPDATA%\\agy\\bin\\agy.exe）",
                    )
                })?;
            (bin, Vec::new())
        }
        "codex" => ("codex".to_string(), vec!["login"]),
        "claude_code" => ("claude".to_string(), Vec::new()),
        _ => {
            return Err(IpcError::new(
                "UNSUPPORTED_LOGIN",
                "该提供商请使用 API 密钥方式接入",
            ))
        }
    };

    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("cmd.exe");
        cmd.arg("/c")
            .arg("start")
            .arg("TokenTracker Account Login")
            .arg(&program);
        for arg in &cli_args {
            cmd.arg(arg);
        }
        cmd.spawn().map_err(|e| {
            IpcError::new(
                "CLI_LOGIN_SPAWN_ERROR",
                format!("启动系统终端登录窗口失败: {e}"),
            )
        })?;
    }

    #[cfg(target_os = "macos")]
    {
        let full_cmd = if cli_args.is_empty() {
            format!("\"{}\"", program.replace('"', "\\\""))
        } else {
            format!("\"{}\" {}", program.replace('"', "\\\""), cli_args.join(" "))
        };
        let osa_script = format!(
            "tell application \"Terminal\" to do script \"{}\"",
            full_cmd.replace('"', "\\\"")
        );
        std::process::Command::new("osascript")
            .args(["-e", &osa_script])
            .spawn()
            .map_err(|e| {
                IpcError::new(
                    "CLI_LOGIN_SPAWN_ERROR",
                    format!("启动系统终端登录窗口失败: {e}"),
                )
            })?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut cmd = std::process::Command::new("x-terminal-emulator");
        cmd.arg("-e").arg(&program);
        for arg in &cli_args {
            cmd.arg(arg);
        }
        cmd.spawn().map_err(|e| {
            IpcError::new(
                "CLI_LOGIN_SPAWN_ERROR",
                format!("启动系统终端登录窗口失败: {e}"),
            )
        })?;
    }

    Ok("已在系统终端打开登录窗口，完成授权后点击同步即可".to_string())
}

#[tauri::command]
pub fn list_devices(state: State<'_, AppState>) -> Result<Vec<DeviceRecord>, IpcError> {
    state.storage.list_devices().map_err(IpcError::from)
}

#[tauri::command]
pub fn get_sync_status(state: State<'_, AppState>) -> Result<SyncStatusView, IpcError> {
    state.sync_client.status_view().map_err(IpcError::from)
}

#[tauri::command]
pub fn update_sync_config(
    state: State<'_, AppState>,
    enabled: bool,
    worker_url: Option<String>,
    e2ee_enabled: bool,
) -> Result<SyncStatusView, IpcError> {
    let current = state
        .storage
        .get_sync_config_state()
        .map_err(IpcError::from)?;
    let salt = if e2ee_enabled
        && current
            .e2ee_kdf_salt_hex
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none()
    {
        Some(generate_kdf_salt_hex().map_err(IpcError::from)?)
    } else {
        None
    };

    state
        .storage
        .update_sync_config_state(
            enabled,
            worker_url.as_deref(),
            e2ee_enabled,
            salt.as_deref(),
        )
        .map_err(IpcError::from)?;
    state.sync_client.reconfigure();
    state.sync_client.status_view().map_err(IpcError::from)
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncPairingBundle {
    pub pairing_uri: String,
    pub pairing_json: String,
    pub worker_url: String,
    pub e2ee_enabled: bool,
    pub includes_passphrase: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SyncPairingJsonPayload<'a> {
    v: u32,
    url: &'a str,
    token: &'a str,
    e2ee: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<&'a str>,
}

#[tauri::command]
pub fn save_cloud_sync_setup(
    state: State<'_, AppState>,
    enabled: bool,
    worker_url: Option<String>,
    bearer_token: Option<String>,
    e2ee_enabled: bool,
    e2ee_passphrase: Option<String>,
) -> Result<SyncStatusView, IpcError> {
    let normalized_url = match worker_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => Some(normalize_worker_url(Some(raw)).map_err(IpcError::from)?),
        None => {
            if enabled {
                return Err(IpcError::new(
                    "VALIDATION_ERROR",
                    "请填写 Worker 服务地址",
                ));
            }
            None
        }
    };

    set_sync_creds(
        state.credentials.as_ref(),
        bearer_token.as_deref(),
        e2ee_passphrase.as_deref(),
    )
    .map_err(IpcError::from)?;

    let (has_bearer, has_passphrase) =
        has_sync_credentials(state.credentials.as_ref()).map_err(IpcError::from)?;
    if enabled && !has_bearer {
        return Err(IpcError::new(
            "VALIDATION_ERROR",
            "请填写或生成访问令牌 (SYNC_TOKEN)",
        ));
    }
    if enabled && e2ee_enabled && !has_passphrase {
        return Err(IpcError::new(
            "VALIDATION_ERROR",
            "开启端到端加密时请填写或生成加密口令",
        ));
    }

    let current = state
        .storage
        .get_sync_config_state()
        .map_err(IpcError::from)?;
    let salt = if e2ee_enabled
        && current
            .e2ee_kdf_salt_hex
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none()
    {
        Some(generate_kdf_salt_hex().map_err(IpcError::from)?)
    } else {
        None
    };

    state
        .storage
        .update_sync_config_state(
            enabled,
            normalized_url.as_deref(),
            e2ee_enabled,
            salt.as_deref(),
        )
        .map_err(IpcError::from)?;
    state.sync_client.reconfigure();
    state.sync_client.status_view().map_err(IpcError::from)
}

#[tauri::command]
pub fn get_sync_pairing_bundle(
    state: State<'_, AppState>,
    include_passphrase: bool,
) -> Result<SyncPairingBundle, IpcError> {
    let cfg = state
        .storage
        .get_sync_config_state()
        .map_err(IpcError::from)?;
    let worker_url = normalize_worker_url(cfg.worker_url.as_deref()).map_err(|_| {
        IpcError::new("VALIDATION_ERROR", "请先填写并保存 Worker 服务地址")
    })?;
    let bearer_token = state
        .credentials
        .get_api_key(SYNC_BEARER_TOKEN_KEY)
        .map_err(IpcError::from)?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IpcError::new("VALIDATION_ERROR", "请先填写或生成访问令牌 (SYNC_TOKEN)"))?;

    let passphrase = if cfg.e2ee_enabled && include_passphrase {
        state
            .credentials
            .get_api_key(SYNC_E2EE_PASSPHRASE_KEY)
            .map_err(IpcError::from)?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    };

    let includes_passphrase = passphrase.is_some();
    let pairing_uri = build_sync_pairing_uri(
        &worker_url,
        &bearer_token,
        cfg.e2ee_enabled,
        passphrase.as_deref(),
    )
    .map_err(IpcError::from)?;

    let pairing_json = serde_json::to_string(&SyncPairingJsonPayload {
        v: 1,
        url: &worker_url,
        token: &bearer_token,
        e2ee: cfg.e2ee_enabled,
        key: passphrase.as_deref(),
    })
    .map_err(|e| IpcError::new("SERIALIZATION_ERROR", e.to_string()))?;

    Ok(SyncPairingBundle {
        pairing_uri,
        pairing_json,
        worker_url,
        e2ee_enabled: cfg.e2ee_enabled,
        includes_passphrase,
    })
}

#[tauri::command]
pub fn set_sync_credentials(
    state: State<'_, AppState>,
    bearer_token: Option<String>,
    e2ee_passphrase: Option<String>,
) -> Result<SyncStatusView, IpcError> {
    set_sync_creds(
        state.credentials.as_ref(),
        bearer_token.as_deref(),
        e2ee_passphrase.as_deref(),
    )
    .map_err(IpcError::from)?;
    state.sync_client.reconfigure();
    state.sync_client.status_view().map_err(IpcError::from)
}

#[tauri::command]
pub fn clear_sync_credentials(state: State<'_, AppState>) -> Result<SyncStatusView, IpcError> {
    clear_sync_creds(state.credentials.as_ref()).map_err(IpcError::from)?;
    state.sync_client.reconfigure();
    state.sync_client.status_view().map_err(IpcError::from)
}

#[tauri::command]
pub async fn trigger_cloud_sync_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SyncRunReport, IpcError> {
    let report = state
        .sync_client
        .trigger_now()
        .await
        .map_err(IpcError::from)?;
    if report.applied_hourly_buckets > 0 || report.applied_subscriptions > 0 {
        let _ = app.emit("usage-updated", &report);
    }
    Ok(report)
}

#[tauri::command]
pub fn get_remote_subscription_snapshots(
    state: State<'_, AppState>,
) -> Result<Vec<RemoteSubscriptionSnapshot>, IpcError> {
    state
        .storage
        .list_remote_subscription_snapshots()
        .map_err(IpcError::from)
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceNavigatePayload {
    pub page: Option<String>,
    pub provider_id: Option<String>,
}

#[tauri::command]
pub fn open_main_workspace(
    app: AppHandle,
    page: Option<String>,
    provider_id: Option<String>,
) {
    show_main_window(&app);
    if page.is_some() || provider_id.is_some() {
        let _ = app.emit(
            "navigate-workspace",
            WorkspaceNavigatePayload { page, provider_id },
        );
    }
}

#[tauri::command]
pub fn quit_application(app: AppHandle, state: State<'_, AppState>) {
    state.shutdown_background_services();
    app.exit(0);
}

#[tauri::command]
pub async fn toggle_floating_window(app: AppHandle, visible: Option<bool>) -> bool {
    toggle_floating_window_with(&app, visible)
}

#[tauri::command]
pub fn get_floating_window_visible(app: AppHandle, state: State<'_, AppState>) -> bool {
    app.get_webview_window(FLOATING_MONITOR_LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or_else(|| {
            state
                .storage
                .get_preferences()
                .map(|p| p.floating_window_enabled)
                .unwrap_or(false)
        })
}

#[tauri::command]
pub fn set_floating_window_compact(
    app: AppHandle,
    compact: bool,
    width: Option<f64>,
    height: Option<f64>,
) {
    resize_floating_window_internal(&app, compact, width, height);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_parsing_handles_dark_light_and_system() {
        assert_eq!(parse_tauri_theme("dark"), Some(tauri::Theme::Dark));
        assert_eq!(parse_tauri_theme(" LIGHT "), Some(tauri::Theme::Light));
        assert_eq!(parse_tauri_theme("system"), None);
        assert_eq!(parse_tauri_theme("auto"), None);
    }

    #[test]
    fn managed_filename_validation_blocks_path_traversal() {
        assert!(is_valid_managed_filename("wallpaper_1760000000000.png"));
        assert!(!is_valid_managed_filename(""));
        assert!(!is_valid_managed_filename("../wallpaper.png"));
        assert!(!is_valid_managed_filename("sub/wallpaper.png"));
        assert!(!is_valid_managed_filename("sub\\wallpaper.png"));
        assert!(!is_valid_managed_filename("C:wallpaper.png"));
    }

    #[test]
    fn image_magic_bytes_detect_supported_formats_and_reject_invalid() {
        assert_eq!(
            detect_image_format(b"\x89PNG\r\n\x1a\nxxxx"),
            Some(("png", "image/png"))
        );
        assert_eq!(
            detect_image_format(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(("jpg", "image/jpeg"))
        );
        assert_eq!(
            detect_image_format(b"RIFF\x00\x00\x00\x00WEBP"),
            Some(("webp", "image/webp"))
        );
        assert_eq!(
            detect_image_format(b"GIF89axxx"),
            Some(("gif", "image/gif"))
        );
        assert_eq!(detect_image_format(b"BM\x00\x00"), Some(("bmp", "image/bmp")));
        assert_eq!(detect_image_format(b"<svg></svg>"), None);
    }

    #[test]
    fn base64_encoder_matches_rfc4648_test_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foob"), "Zm9vYg==");
        assert_eq!(encode_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }
}
