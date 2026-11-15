export type ProviderId = 'claude_code' | 'codex' | 'antigravity';
export type CollectionMode = 'local_log' | 'otlp' | 'official_api';
export type CurrencyCode = 'USD' | 'CNY';
export type ThemeMode = 'system' | 'dark' | 'light';
export type QuotaUnit = 'micro_usd' | 'tokens';
export type TrendGranularity = 'fifteen_minutes' | 'hourly' | 'daily' | 'weekly';
export type StackDimension = 'provider' | 'model' | 'token_type';

export interface DeviceRecord {
  device_id: string;
  device_name: string;
  platform: string;
  created_at_ms: number;
}

export interface ProviderConfig {
  provider_id: string;
  enabled: boolean;
  collection_mode: CollectionMode;
  custom_log_paths: string[];
  has_credential: boolean;
  last_sync_at_ms: number | null;
  last_error: string | null;
  updated_at_ms: number;
}

export interface IngestionCursor {
  source_path: string;
  provider_id: string;
  file_identity: string;
  byte_offset: number;
  last_line_hash: string;
  updated_at_ms: number;
}

export interface AppPreferences {
  currency: CurrencyCode;
  usd_to_cny_rate: string;
  theme: ThemeMode | string;
  minimize_to_tray: boolean;
  launch_at_startup: boolean;
  otlp_enabled: boolean;
  otlp_port: number;
  device_display_name: string;
  updated_at_ms: number;
}

export interface ScanReport {
  files_scanned: number;
  events_inserted: number;
  events_deduplicated: number;
  errors: string[];
  scanned_at_ms: number;
}

export interface BootstrapState {
  db_path: string;
  device: DeviceRecord;
  preferences: AppPreferences;
  providers: ProviderConfig[];
  collector_watching: boolean;
  otlp_running: boolean;
  otlp_port: number | null;
  last_scan: ScanReport | null;
}

export interface AnalyticsFilter {
  start_ms: number;
  end_ms: number;
  provider_id: string | null;
  tz_offset_minutes: number;
}

export interface TimeBucketPoint {
  bucket_start_ms: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  cost_micro_usd: number;
  requests: number;
}

export interface ProviderSummaryRow {
  provider_id: string;
  enabled: boolean;
  period_tokens: number;
  period_cost_micro_usd: number;
  today_tokens: number;
  today_cost_micro_usd: number;
  month_tokens: number;
  month_cost_micro_usd: number;
  cache_hit_ratio: number;
  observable_steps: number;
  active_window_label: string | null;
  active_window_used_percent: number | null;
  last_sync_at_ms: number | null;
  last_error: string | null;
}

export interface OverviewReport {
  total_tokens: number;
  total_cost_micro_usd: number;
  cache_hit_ratio: number;
  avg_output_tps: number;
  peak_output_tps: number;
  latest_output_tps: number | null;
  total_requests: number;
  observable_steps: number;
  tokens_delta_pct: number | null;
  cost_delta_pct: number | null;
  cache_delta_pct: number | null;
  tps_delta_pct: number | null;
  series_granularity: string;
  series: TimeBucketPoint[];
  providers: ProviderSummaryRow[];
}

export interface TrendsQuery {
  filter: AnalyticsFilter;
  granularity: TrendGranularity;
  dimension: StackDimension;
}

export interface TrendPoint {
  bucket_start_ms: number;
  tokens: number;
  cost_micro_usd: number;
  requests: number;
}

export interface TrendSeries {
  key: string;
  label: string;
  total_tokens: number;
  total_cost_micro_usd: number;
  points: TrendPoint[];
}

export interface TrendComparisonRow {
  key: string;
  label: string;
  current_tokens: number;
  previous_tokens: number;
  delta_pct: number | null;
  current_cost_micro_usd: number;
  previous_cost_micro_usd: number;
  requests: number;
}

export interface TrendsReport {
  buckets: number[];
  series: TrendSeries[];
  comparisons: TrendComparisonRow[];
}

export interface MonthlyQuery {
  year: number;
  month: number;
  provider_id: string | null;
  tz_offset_minutes: number;
}

export interface MonthlyDailyLedgerRow {
  date: string;
  requests: number;
  observable_steps: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  total_tokens: number;
  cost_micro_usd: number;
  cache_saved_micro_usd: number;
  top_model: string | null;
}

export interface HeatmapDayCell {
  date: string;
  total_tokens: number;
  cost_micro_usd: number;
  requests: number;
  top_model: string | null;
}

export interface MonthlyReport {
  year: number;
  month: number;
  api_equivalent_cost_micro_usd: number;
  subscription_monthly_fee_micro_usd: number;
  roi_multiplier: number | null;
  month_total_tokens: number;
  month_total_requests: number;
  month_cache_saved_micro_usd: number;
  daily_ledger: MonthlyDailyLedgerRow[];
  heatmap_365d: HeatmapDayCell[];
}

export interface ModelPricingEntry {
  model_pattern: string;
  provider_id: string;
  input_per_m_micro_usd: number;
  output_per_m_micro_usd: number;
  cache_read_per_m_micro_usd: number;
  cache_write_per_m_micro_usd: number;
  reasoning_per_m_micro_usd: number;
  is_user_override: boolean;
  effective_from: string;
}

export interface ModelStatRow {
  model_id: string;
  provider_id: string;
  requests: number;
  observable_steps: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  total_tokens: number;
  cache_saved_micro_usd: number;
  estimated_cost_micro_usd: number;
  cost_share_ratio: number;
  avg_output_tps: number;
  effective_pricing: ModelPricingEntry | null;
}

export interface ModelStatsReport {
  rows: ModelStatRow[];
  total_cost_micro_usd: number;
  total_cache_saved_micro_usd: number;
  catalog: ModelPricingEntry[];
}

export interface SubscriptionWindowRule {
  window_id: string;
  label: string;
  duration_hours: number;
  rolling: boolean;
  unit: QuotaUnit;
  limit_value: number;
  warning_percent: number;
  critical_percent: number;
}

export interface SubscriptionPlan {
  plan_id: string;
  provider_id: string;
  plan_name: string;
  enabled: boolean;
  monthly_fee_micro_usd: number;
  windows: SubscriptionWindowRule[];
  updated_at_ms: number;
}

export interface SubscriptionWindowStatus {
  window_id: string;
  label: string;
  duration_hours: number;
  rolling: boolean;
  unit: QuotaUnit;
  used_value: number;
  limit_value: number;
  used_percent: number;
  warning_percent: number;
  critical_percent: number;
  burn_rate_per_hour: number;
  projected_exhaustion_hours: number | null;
  resets_at_ms: number | null;
  resets_in_seconds: number;
  source: string;
  alert_level: string;
}

export interface SubscriptionPlanStatus {
  plan: SubscriptionPlan;
  windows: SubscriptionWindowStatus[];
}

export interface ModelSpeedStats {
  model_id: string;
  provider_id: string;
  sample_count: number;
  mean_tps: number;
  p50_tps: number;
  p95_tps: number;
  max_tps: number;
  min_tps: number;
  latest_tps: number;
  avg_duration_ms: number;
  avg_ttft_ms: number | null;
}

export interface ContextSpeedSample {
  model_id: string;
  provider_id: string;
  context_tokens: number;
  output_tps: number;
  duration_ms: number;
  timestamp_ms: number;
}

export interface ContextDecayBin {
  bin_label: string;
  sample_count: number;
  avg_tps: number;
  p50_tps: number;
  avg_duration_ms: number;
}

export interface SpeedLatencyReport {
  models: ModelSpeedStats[];
  scatter_points: ContextSpeedSample[];
  context_bins: ContextDecayBin[];
}

export interface TraySummary {
  today_tokens: number;
  today_cost_micro_usd: number;
  today_requests: number;
  latest_output_tps: number | null;
  subscriptions: SubscriptionPlanStatus[];
  currency: CurrencyCode;
  usd_to_cny_rate: string;
  active_providers_count: number;
  total_ingested_events: number;
}

export interface OfficialApiReconcileResult {
  provider_id: string;
  endpoint: string;
  status_code: number;
  buckets_received: number;
  total_input_tokens: number;
  total_output_tokens: number;
  reported_cost_usd: number | null;
  checked_at_ms: number;
}

export interface ExportReportResult {
  path: string | null;
  content: string;
  bytes: number;
}

export interface SyncHourlyDelta {
  provider_id: string;
  model_id: string;
  bucket_start_ms: number;
  request_count: number;
  observable_step_count: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  estimated_cost_micro_usd: number;
  cache_saved_micro_usd: number;
  total_duration_ms: number;
  tps_sample_count: number;
  tps_sum: number;
  max_output_tps: number;
  updated_at_ms: number;
}

export interface SyncSubscriptionSnapshot {
  plan_id: string;
  provider_id: string;
  plan_name: string;
  window_id: string;
  window_label: string;
  used_percent: number;
  resets_at_ms: number | null;
  source: string;
}

export interface SyncEnvelopeV1 {
  schema_version: number;
  device_id: string;
  device_display_name: string;
  generated_at_ms: number;
  since_watermark_ms: number;
  hourly_deltas: SyncHourlyDelta[];
  subscription_snapshots: SyncSubscriptionSnapshot[];
}
