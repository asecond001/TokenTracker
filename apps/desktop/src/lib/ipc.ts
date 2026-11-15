import { invoke } from '@tauri-apps/api/core';
import type {
  AnalyticsFilter,
  AppPreferences,
  BootstrapState,
  CatalogSyncResult,
  ExportReportResult,
  IngestionCursor,
  ModelPricingEntry,
  ModelStatsReport,
  MonthlyQuery,
  MonthlyReport,
  ObservedRateLimitWindow,
  OfficialApiReconcileResult,
  OverviewReport,
  ProviderConfig,
  ScanReport,
  SpeedLatencyReport,
  SubscriptionPlan,
  SubscriptionPlanStatus,
  SubscriptionPlanTemplate,
  SyncEnvelopeV1,
  TraySummary,
  TrendsQuery,
  TrendsReport,
} from '../types/ipc';

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && Boolean(window.__TAURI_INTERNALS__);
}

function nowMinusHours(hours: number): number {
  return Date.now() - hours * 3_600_000;
}

function defaultPreviewPreferences(now: number): AppPreferences {
  return {
    currency: 'USD',
    usd_to_cny_rate: '7.25',
    theme: 'dark',
    minimize_to_tray: true,
    launch_at_startup: false,
    otlp_enabled: false,
    otlp_port: 4318,
    device_display_name: 'win-workstation',
    bg_mode: 'none',
    bg_image_path: null,
    bg_preset_id: 'aurora_slate',
    bg_opacity: 0.18,
    bg_blur_px: 16,
    auto_sync_pricing: true,
    exchange_rate_auto: true,
    last_rate_sync_at_ms: null,
    last_pricing_sync_at_ms: null,
    updated_at_ms: now,
  };
}

function previewSubscriptionTemplates(): SubscriptionPlanTemplate[] {
  return [
    {
      template_id: 'claude_pro',
      provider_id: 'claude_code',
      plan_name: 'Claude Pro ($20/mo)',
      monthly_fee_micro_usd: 20_000_000,
      description: 'Anthropic Claude Pro 个人订阅（含 5h 滚动与 7d 周参考窗口）',
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 15_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
        {
          window_id: '7d',
          label: '7d 周额度',
          duration_hours: 168,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 75_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
    {
      template_id: 'claude_max_5x',
      provider_id: 'claude_code',
      plan_name: 'Claude Max 5x ($100/mo)',
      monthly_fee_micro_usd: 100_000_000,
      description: 'Anthropic Claude Max 5x 高配额订阅（5 倍 Pro 参考额度）',
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 75_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
        {
          window_id: '7d',
          label: '7d 周额度',
          duration_hours: 168,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 375_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
    {
      template_id: 'claude_max_20x',
      provider_id: 'claude_code',
      plan_name: 'Claude Max 20x ($200/mo)',
      monthly_fee_micro_usd: 200_000_000,
      description: 'Anthropic Claude Max 20x 重度编码订阅（20 倍 Pro 参考额度）',
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 200_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
        {
          window_id: '7d',
          label: '7d 周额度',
          duration_hours: 168,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 1_200_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
    {
      template_id: 'codex_plus',
      provider_id: 'codex',
      plan_name: 'ChatGPT Plus / Codex ($20/mo)',
      monthly_fee_micro_usd: 20_000_000,
      description: 'OpenAI ChatGPT Plus（可自动对齐 Codex 日志官方 5h / 7d 限额）',
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 18_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
        {
          window_id: '7d',
          label: '7d 周额度',
          duration_hours: 168,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 90_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
    {
      template_id: 'codex_pro',
      provider_id: 'codex',
      plan_name: 'ChatGPT Pro / Codex ($200/mo)',
      monthly_fee_micro_usd: 200_000_000,
      description: 'OpenAI ChatGPT Pro（可自动对齐 Codex 日志官方 5h / 7d 限额）',
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 150_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
        {
          window_id: '7d',
          label: '7d 周额度',
          duration_hours: 168,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 900_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
    {
      template_id: 'google_ai_pro',
      provider_id: 'antigravity',
      plan_name: 'Google AI Pro ($19.99/mo)',
      monthly_fee_micro_usd: 19_990_000,
      description: 'Google AI Pro / Antigravity 个人订阅（19.99 USD/月参考窗口）',
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 20_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
        {
          window_id: '7d',
          label: '7d 周额度',
          duration_hours: 168,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 100_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
    {
      template_id: 'google_ai_ultra',
      provider_id: 'antigravity',
      plan_name: 'Google AI Ultra ($249.99/mo)',
      monthly_fee_micro_usd: 249_990_000,
      description: 'Google AI Ultra / Antigravity 旗舰订阅（249.99 USD/月高额度窗口）',
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 250_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
        {
          window_id: '7d',
          label: '7d 周额度',
          duration_hours: 168,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 1_250_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
  ];
}

/**
 * Lightweight browser-preview fallback used ONLY when the React UI is loaded directly
 * in Vite/Vitest outside Tauri native runtime. Inside Tauri desktop, all calls go directly
 * to Rust + SQLite via `invoke`.
 */
function buildPreviewFallback<T>(command: string, args?: Record<string, unknown>): T {
  const now = Date.now();
  const baseHour = Math.floor(now / 3_600_000) * 3_600_000;
  switch (command) {
    case 'get_bootstrap_state': {
      const data: BootstrapState = {
        db_path: '%LOCALAPPDATA%\\TokenTracker\\tokentracker.db',
        device: {
          device_id: 'preview-local-device',
          device_name: 'win-workstation',
          platform: 'windows',
          created_at_ms: nowMinusHours(72),
        },
        preferences: defaultPreviewPreferences(now),
        providers: [
          {
            provider_id: 'claude_code',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: now - 15_000,
            last_error: null,
            updated_at_ms: now,
          },
          {
            provider_id: 'codex',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: now - 20_000,
            last_error: null,
            updated_at_ms: now,
          },
          {
            provider_id: 'antigravity',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: now - 10_000,
            last_error: null,
            updated_at_ms: now,
          },
        ],
        collector_watching: true,
        otlp_running: false,
        otlp_port: null,
        last_scan: {
          files_scanned: 0,
          events_inserted: 0,
          events_deduplicated: 0,
          errors: [],
          scanned_at_ms: now,
        },
      };
      return data as T;
    }
    case 'get_overview': {
      const data: OverviewReport = {
        total_tokens: 0,
        total_cost_micro_usd: 0,
        cache_hit_ratio: 0,
        avg_output_tps: 0,
        peak_output_tps: 0,
        latest_output_tps: null,
        total_requests: 0,
        observable_steps: 0,
        tokens_delta_pct: null,
        cost_delta_pct: null,
        cache_delta_pct: null,
        tps_delta_pct: null,
        series_granularity: 'hourly',
        series: [
          {
            bucket_start_ms: baseHour,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            cost_micro_usd: 0,
            requests: 0,
          },
        ],
        provider_series: [],
        providers: [],
      };
      return data as T;
    }
    case 'get_trends':
      return {
        buckets: [],
        series: [],
        comparisons: [],
        effective_granularity: 'daily',
      } as T;
    case 'get_monthly':
      return {
        year: new Date().getFullYear(),
        month: new Date().getMonth() + 1,
        api_equivalent_cost_micro_usd: 0,
        subscription_monthly_fee_micro_usd: 0,
        roi_multiplier: null,
        month_total_tokens: 0,
        month_total_requests: 0,
        month_cache_saved_micro_usd: 0,
        daily_ledger: [],
        heatmap_365d: [],
      } as T;
    case 'get_models_pricing':
      return {
        rows: [],
        total_cost_micro_usd: 0,
        total_cache_saved_micro_usd: 0,
        catalog: [],
      } as T;
    case 'get_subscriptions':
    case 'get_provider_configs':
    case 'get_ingestion_cursors':
    case 'get_observed_rate_limits':
      return [] as T;
    case 'get_subscription_templates':
      return previewSubscriptionTemplates() as T;
    case 'get_speed_latency':
      return { providers: [], models: [] } as T;
    case 'get_tray_summary':
      return {
        today_tokens: 0,
        today_cost_micro_usd: 0,
        today_requests: 0,
        latest_output_tps: null,
        subscriptions: [],
        currency: 'USD',
        usd_to_cny_rate: '7.25',
        active_providers_count: 3,
        total_ingested_events: 0,
      } as T;
    case 'get_preferences':
      return defaultPreviewPreferences(now) as T;
    case 'update_preferences':
      return ((args?.prefs as AppPreferences | undefined) ?? defaultPreviewPreferences(now)) as T;
    case 'get_background_image_data_url':
      return null as T;
    case 'import_background_image':
      return ((args?.sourcePath as string | undefined) || 'preview-wallpaper.png') as T;
    case 'delete_subscription_plan':
    case 'clear_background_image':
      return undefined as T;
    case 'trigger_scan_now':
      return {
        files_scanned: 0,
        events_inserted: 0,
        events_deduplicated: 0,
        errors: [],
        scanned_at_ms: now,
      } as T;
    case 'sync_pricing_and_rate':
      return {
        rate_updated: false,
        usd_to_cny_rate: '7.25',
        pricing_updated: false,
        synced_models_count: 0,
        recomputed_events_count: 0,
        synced_at_ms: now,
        rate_source: null,
        pricing_source: null,
        warning: null,
      } as T;
    case 'preview_sync_envelope_v1':
      return {
        schema_version: 1,
        device_id: 'preview-local-device',
        device_display_name: 'win-workstation',
        generated_at_ms: now,
        since_watermark_ms: 0,
        hourly_deltas: [],
        subscription_snapshots: [],
      } as T;
    default:
      return {} as T;
  }
}

async function callIpc<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    return buildPreviewFallback<T>(command, args);
  }
  return invoke<T>(command, args);
}

export const ipc = {
  getBootstrapState: (): Promise<BootstrapState> => callIpc('get_bootstrap_state'),

  getOverview: (filter: AnalyticsFilter): Promise<OverviewReport> =>
    callIpc('get_overview', { filter }),

  getTrends: (query: TrendsQuery): Promise<TrendsReport> =>
    callIpc('get_trends', { query }),

  getMonthly: (query: MonthlyQuery): Promise<MonthlyReport> =>
    callIpc('get_monthly', { query }),

  getModelsPricing: (filter: AnalyticsFilter): Promise<ModelStatsReport> =>
    callIpc('get_models_pricing', { filter }),

  upsertModelPricingOverride: (
    entry: ModelPricingEntry,
    recomputeHistory: boolean
  ): Promise<number> =>
    callIpc('upsert_model_pricing_override', {
      entry,
      recomputeHistory,
    }),

  deleteModelPricingOverride: (
    modelPattern: string,
    recomputeHistory: boolean
  ): Promise<number> =>
    callIpc('delete_model_pricing_override', {
      modelPattern,
      recomputeHistory,
    }),

  recomputeAllCosts: (): Promise<number> => callIpc('recompute_all_costs'),

  syncPricingAndRate: (force = false): Promise<CatalogSyncResult> =>
    callIpc('sync_pricing_and_rate', { force }),

  getSubscriptions: (): Promise<SubscriptionPlanStatus[]> => callIpc('get_subscriptions'),

  upsertSubscriptionPlan: (plan: SubscriptionPlan): Promise<void> =>
    callIpc('upsert_subscription_plan', { plan }),

  deleteSubscriptionPlan: (planId: string): Promise<void> =>
    callIpc('delete_subscription_plan', { planId }),

  getSubscriptionTemplates: (): Promise<SubscriptionPlanTemplate[]> =>
    callIpc('get_subscription_templates'),

  getObservedRateLimits: (): Promise<ObservedRateLimitWindow[]> =>
    callIpc('get_observed_rate_limits'),

  getSpeedLatency: (filter: AnalyticsFilter): Promise<SpeedLatencyReport> =>
    callIpc('get_speed_latency', { filter }),

  getTraySummary: (tzOffsetMinutes: number): Promise<TraySummary> =>
    callIpc('get_tray_summary', { tzOffsetMinutes }),

  getProviderConfigs: (): Promise<ProviderConfig[]> => callIpc('get_provider_configs'),

  updateProviderConfig: (config: ProviderConfig): Promise<void> =>
    callIpc('update_provider_config', { config }),

  getIngestionCursors: (): Promise<IngestionCursor[]> => callIpc('get_ingestion_cursors'),

  triggerScanNow: (): Promise<ScanReport> => callIpc('trigger_scan_now'),

  setProviderApiKey: (providerId: string, apiKey: string): Promise<void> =>
    callIpc('set_provider_api_key', { providerId, apiKey }),

  deleteProviderApiKey: (providerId: string): Promise<void> =>
    callIpc('delete_provider_api_key', { providerId }),

  reconcileProviderOfficialApi: (
    providerId: string,
    lookbackDays: number
  ): Promise<OfficialApiReconcileResult> =>
    callIpc('reconcile_provider_official_api', { providerId, lookbackDays }),

  getPreferences: (): Promise<AppPreferences> => callIpc('get_preferences'),

  updatePreferences: (prefs: AppPreferences): Promise<AppPreferences> =>
    callIpc('update_preferences', { prefs }),

  importBackgroundImage: (sourcePath: string): Promise<string> =>
    callIpc('import_background_image', { sourcePath }),

  clearBackgroundImage: (): Promise<void> => callIpc('clear_background_image'),

  getBackgroundImageDataUrl: (): Promise<string | null> =>
    callIpc('get_background_image_data_url'),

  exportMonthlyReport: (
    query: MonthlyQuery,
    format: 'csv' | 'json',
    savePath?: string | null
  ): Promise<ExportReportResult> =>
    callIpc('export_monthly_report', {
      query,
      format,
      savePath: savePath ?? null,
    }),

  previewSyncEnvelopeV1: (sinceWatermarkMs: number): Promise<SyncEnvelopeV1> =>
    callIpc('preview_sync_envelope_v1', { sinceWatermarkMs }),

  openMainWorkspace: (): Promise<void> => callIpc('open_main_workspace'),

  quitApplication: (): Promise<void> => callIpc('quit_application'),
};
