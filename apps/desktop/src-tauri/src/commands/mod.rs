use chrono::Utc;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;
use tokentracker_collectors::providers::official_api::{
    reconcile_official_usage_api, OfficialApiReconcileResult,
};
use tokentracker_collectors::ScanReport;
use tokentracker_core::analytics::{
    export_monthly_csv, export_monthly_json, AnalyticsFilter, ModelStatsReport, MonthlyQuery,
    MonthlyReport, OverviewReport, SpeedLatencyReport, TraySummary, TrendsQuery, TrendsReport,
};
use tokentracker_core::domain::{
    AppPreferences, DeviceRecord, IngestionCursor, ModelPricingEntry, ProviderConfig,
    SubscriptionPlan, SyncEnvelopeV1,
};
use tokentracker_core::subscription::SubscriptionPlanStatus;
use tokentracker_core::CoreError;

use crate::state::AppState;
use crate::tray::show_main_window;

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
    tauri::async_runtime::spawn_blocking(move || coord.scan_now())
        .await
        .map_err(|e| IpcError::new("TASK_JOIN_ERROR", e.to_string()))?
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn set_provider_api_key(
    state: State<'_, AppState>,
    provider_id: String,
    api_key: String,
) -> Result<(), IpcError> {
    state.credentials.set_api_key(&provider_id, &api_key)?;
    state
        .storage
        .set_provider_has_credential(&provider_id, true)?;
    Ok(())
}

#[tauri::command]
pub fn delete_provider_api_key(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<(), IpcError> {
    state.credentials.delete_api_key(&provider_id)?;
    state
        .storage
        .set_provider_has_credential(&provider_id, false)?;
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

    state.storage.get_preferences().map_err(IpcError::from)
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
pub fn open_main_workspace(app: AppHandle) {
    show_main_window(&app);
}

#[tauri::command]
pub fn quit_application(app: AppHandle, state: State<'_, AppState>) {
    state.shutdown_background_services();
    app.exit(0);
}
