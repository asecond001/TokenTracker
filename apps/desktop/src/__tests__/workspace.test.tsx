import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { TimeRangeControl } from '../components/TimeRangeControl';
import { TitleBar } from '../components/TitleBar';
import { WorkspaceBackdrop } from '../components/WorkspaceBackdrop';
import {
  TOKEN_TYPE_COLORS,
  convertMicroUsd,
  formatBucketInTz,
  formatCost,
  formatCountdown,
  formatPercent,
  formatTokens,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { QrCodeSvg, encodeQrMatrix, gfMul, rsComputeRemainder } from '../lib/qrcode';
import { DataSourcesPage } from '../pages/DataSourcesPage';
import { FloatingMonitorPage } from '../pages/FloatingMonitorPage';
import { ModelsPricingPage } from '../pages/ModelsPricingPage';
import { MonthlyPage, computeHeatmapIntensityLevels } from '../pages/MonthlyPage';
import { OverviewPage } from '../pages/OverviewPage';
import { PreferencesPage } from '../pages/PreferencesPage';
import { SpeedLatencyPage } from '../pages/SpeedLatencyPage';
import { SubscriptionsPage } from '../pages/SubscriptionsPage';
import { TrayPopupPage } from '../pages/TrayPopupPage';
import { TrendsPage } from '../pages/TrendsPage';
import { computeTimeRange, useWorkspaceStore } from '../stores/workspaceStore';
import type {
  AppPreferences,
  BalanceRefreshOutcome,
  BootstrapState,
  CatalogSyncResult,
  CloudSyncProvisionResult,
  CustomRuleTestResult,
  DeviceRecord,
  ModelStatsReport,
  MonthlyReport,
  ObservedRateLimitWindow,
  OverviewReport,
  ProviderConfig,
  RemoteSubscriptionSnapshot,
  SpeedLatencyReport,
  SubscriptionPlanStatus,
  SubscriptionPlanTemplate,
  SyncPairingBundle,
  SyncStatusView,
  TraySummary,
  TrendsReport,
} from '../types/ipc';

interface EChartMockOption {
  series?: Array<{
    type?: string;
    stack?: string;
    name?: unknown;
    lineStyle?: { width?: number; opacity?: number };
    itemStyle?: { color?: string; borderWidth?: number; borderColor?: string };
    areaStyle?: { opacity?: number };
    data?: unknown;
  }>;
  visualMap?: { type?: string; dimension?: number; pieces?: Array<{ value: number; color: string }> };
}

// Mock ECharts canvas renderer inside jsdom tests while exposing series names & option JSON for assertions
vi.mock('../components/EChartBox', () => ({
  EChartBox: ({ height, option }: { height?: number; option?: EChartMockOption }) => {
    const seriesNames = Array.isArray(option?.series)
      ? option.series.map((s) => String(s.name ?? '')).join(',')
      : '';
    return (
      <div
        data-testid="echart-stub"
        data-series-names={seriesNames}
        data-option-json={JSON.stringify(option ?? {})}
        style={{ height: `${height ?? 240}px` }}
      />
    );
  },
}));

const originalMatchMedia = window.matchMedia;

function installMatchMediaStub(lightMatches: boolean) {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: (query: string) => ({
      matches: query === '(prefers-color-scheme: light)' ? lightMatches : false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}

function renderWithQuery(ui: React.ReactElement) {
  const client = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

function makePreferences(overrides: Partial<AppPreferences> = {}): AppPreferences {
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
    auto_check_updates: true,
    floating_window_enabled: false,
    last_rate_sync_at_ms: null,
    last_pricing_sync_at_ms: null,
    last_subscription_sync_at_ms: null,
    updated_at_ms: 1_770_000_000_000,
    ...overrides,
  };
}

function makeBootstrap(prefs: AppPreferences = makePreferences()): BootstrapState {
  return {
    db_path: 'C:\\Users\\dev\\AppData\\Local\\TokenVane\\tokenvane.db',
    device: {
      device_id: 'dev-test-1',
      device_name: 'win-workstation',
      platform: 'windows',
      created_at_ms: 1_770_000_000_000,
    },
    preferences: prefs,
    providers: [],
    collector_watching: true,
    otlp_running: false,
    otlp_port: null,
    last_scan: null,
  };
}

function makeTraySummary(overrides: Partial<TraySummary> = {}): TraySummary {
  return {
    today_tokens: 150_000,
    today_cost_micro_usd: 2_500_000,
    today_requests: 12,
    latest_output_tps: 68.4,
    subscriptions: [],
    provider_shares: [],
    active_sessions: [],
    currency: 'USD',
    usd_to_cny_rate: '7.25',
    active_providers_count: 2,
    total_ingested_events: 48,
    ...overrides,
  };
}

function makeOverviewReport(overrides: Partial<OverviewReport> = {}): OverviewReport {
  const bucketTs = 1_770_000_000_000;
  return {
    total_tokens: 420_000,
    total_cost_micro_usd: 6_450_000,
    today_tokens: 150_000,
    today_cost_micro_usd: 2_500_000,
    today_requests: 12,
    today_cache_hit_ratio: 0.62,
    today_cache_saved_micro_usd: 450_000,
    today_tokens_delta_pct: 14.5,
    today_cost_delta_pct: 9.8,
    today_hourly_pulse: [
      {
        bucket_start_ms: bucketTs,
        input_tokens: 40_000,
        output_tokens: 20_000,
        cache_read_tokens: 80_000,
        cache_write_tokens: 10_000,
        reasoning_tokens: 0,
        cost_micro_usd: 2_500_000,
        requests: 12,
      },
    ],
    cache_hit_ratio: 0.64,
    avg_output_tps: 74.2,
    peak_output_tps: 142.0,
    latest_output_tps: 88.1,
    total_requests: 35,
    observable_steps: 90,
    tokens_delta_pct: 18.4,
    cost_delta_pct: 12.5,
    cache_delta_pct: 4.2,
    tps_delta_pct: -2.1,
    series_granularity: 'daily',
    series: [
      {
        bucket_start_ms: bucketTs,
        input_tokens: 120_000,
        output_tokens: 60_000,
        cache_read_tokens: 200_000,
        cache_write_tokens: 40_000,
        reasoning_tokens: 0,
        cost_micro_usd: 6_450_000,
        requests: 35,
      },
    ],
    provider_series: [
      {
        key: 'claude_code',
        label: 'Claude Code',
        total_tokens: 270_000,
        total_cost_micro_usd: 4_200_000,
        points: [
          {
            bucket_start_ms: bucketTs,
            tokens: 270_000,
            cost_micro_usd: 4_200_000,
            requests: 20,
          },
        ],
      },
      {
        key: 'codex',
        label: 'Codex',
        total_tokens: 150_000,
        total_cost_micro_usd: 2_250_000,
        points: [
          {
            bucket_start_ms: bucketTs,
            tokens: 150_000,
            cost_micro_usd: 2_250_000,
            requests: 15,
          },
        ],
      },
    ],
    providers: [
      {
        provider_id: 'claude_code',
        enabled: true,
        period_tokens: 270_000,
        period_cost_micro_usd: 4_200_000,
        today_tokens: 90_000,
        today_cost_micro_usd: 1_400_000,
        month_tokens: 270_000,
        month_cost_micro_usd: 4_200_000,
        cache_hit_ratio: 0.71,
        observable_steps: 55,
        active_window_label: null,
        active_window_used_percent: null,
        last_sync_at_ms: bucketTs,
        last_error: null,
      },
      {
        provider_id: 'codex',
        enabled: true,
        period_tokens: 150_000,
        period_cost_micro_usd: 2_250_000,
        today_tokens: 60_000,
        today_cost_micro_usd: 1_100_000,
        month_tokens: 150_000,
        month_cost_micro_usd: 2_250_000,
        cache_hit_ratio: 0.52,
        observable_steps: 35,
        active_window_label: null,
        active_window_used_percent: null,
        last_sync_at_ms: bucketTs,
        last_error: null,
      },
    ],
    provider_shares: [],
    top_model_shares: [],
    active_sessions: [],
    ...overrides,
  };
}

function makeTrendsReport(overrides: Partial<TrendsReport> = {}): TrendsReport {
  const b1 = 1_769_904_000_000;
  const b2 = 1_769_990_400_000;
  return {
    buckets: [b1, b2],
    effective_granularity: 'daily',
    series: [
      {
        key: 'claude_code',
        label: 'Claude Code',
        total_tokens: 310_000,
        total_cost_micro_usd: 4_800_000,
        points: [
          {
            bucket_start_ms: b1,
            tokens: 140_000,
            cost_micro_usd: 2_100_000,
            requests: 11,
            cache_hit_ratio: 0.58,
          },
          {
            bucket_start_ms: b2,
            tokens: 170_000,
            cost_micro_usd: 2_700_000,
            requests: 14,
            cache_hit_ratio: 0.68,
          },
        ],
      },
      {
        key: 'antigravity',
        label: 'Antigravity',
        total_tokens: 180_000,
        total_cost_micro_usd: 2_100_000,
        points: [
          {
            bucket_start_ms: b1,
            tokens: 80_000,
            cost_micro_usd: 900_000,
            requests: 6,
            cache_hit_ratio: 0.42,
          },
          {
            bucket_start_ms: b2,
            tokens: 100_000,
            cost_micro_usd: 1_200_000,
            requests: 8,
            cache_hit_ratio: 0.51,
          },
        ],
      },
    ],
    comparisons: [
      {
        key: 'claude_code',
        label: 'Claude Code',
        current_tokens: 310_000,
        previous_tokens: 200_000,
        delta_pct: 55.0,
        current_cost_micro_usd: 4_800_000,
        previous_cost_micro_usd: 3_000_000,
        cost_delta_pct: 60.0,
        cache_hit_ratio: 0.64,
        requests: 25,
      },
      {
        key: 'antigravity',
        label: 'Antigravity',
        current_tokens: 180_000,
        previous_tokens: 150_000,
        delta_pct: 20.0,
        current_cost_micro_usd: 2_100_000,
        previous_cost_micro_usd: 1_700_000,
        cost_delta_pct: 23.5,
        cache_hit_ratio: 0.47,
        requests: 14,
      },
    ],
    ...overrides,
  };
}

function makeMonthlyReport(overrides: Partial<MonthlyReport> = {}): MonthlyReport {
  return {
    year: 2026,
    month: 11,
    api_equivalent_cost_micro_usd: 30_000_000,
    subscription_monthly_fee_micro_usd: 0,
    roi_multiplier: null,
    month_total_tokens: 1_850_000,
    month_total_requests: 142,
    month_cache_saved_micro_usd: 12_400_000,
    provider_shares: [
      {
        key: 'claude_code',
        label: 'Claude Code',
        tokens: 1_250_000,
        token_share_percent: 67.6,
        cost_micro_usd: 21_000_000,
        cost_share_percent: 70.0,
      },
      {
        key: 'codex',
        label: 'Codex',
        tokens: 600_000,
        token_share_percent: 32.4,
        cost_micro_usd: 9_000_000,
        cost_share_percent: 30.0,
      },
    ],
    model_shares: [
      {
        key: 'claude-opus-4-6',
        label: 'claude-opus-4-6',
        tokens: 1_250_000,
        token_share_percent: 67.6,
        cost_micro_usd: 21_000_000,
        cost_share_percent: 70.0,
      },
    ],
    daily_ledger: [
      {
        date: '2026-11-15',
        requests: 32,
        observable_steps: 78,
        input_tokens: 90_000,
        output_tokens: 45_000,
        cache_read_tokens: 320_000,
        cache_write_tokens: 25_000,
        reasoning_tokens: 12_000,
        total_tokens: 492_000,
        cost_micro_usd: 8_200_000,
        cache_saved_micro_usd: 4_100_000,
        top_model: 'claude-opus-4-6',
      },
    ],
    heatmap_365d: [
      {
        date: '2026-11-15',
        total_tokens: 492_000,
        cost_micro_usd: 8_200_000,
        requests: 32,
        top_model: 'claude-opus-4-6',
      },
    ],
    ...overrides,
  };
}

function makeSpeedLatencyReport(
  overrides: Partial<SpeedLatencyReport> = {}
): SpeedLatencyReport {
  return {
    providers: [
      {
        provider_id: 'claude_code',
        sample_count: 18,
        mean_tps: 64.2,
        p50_tps: 62.0,
        p95_tps: 88.5,
        max_tps: 94.0,
        min_tps: 38.0,
        avg_duration_ms: 4_200,
        avg_ttft_ms: 680,
      },
      {
        provider_id: 'codex',
        sample_count: 9,
        mean_tps: 108.4,
        p50_tps: 105.0,
        p95_tps: 132.0,
        max_tps: 140.0,
        min_tps: 78.0,
        avg_duration_ms: 2_100,
        avg_ttft_ms: 420,
      },
    ],
    models: [
      {
        model_id: 'claude-opus-4-6',
        provider_id: 'claude_code',
        sample_count: 18,
        mean_tps: 64.2,
        p50_tps: 62.0,
        p95_tps: 88.5,
        max_tps: 94.0,
        min_tps: 38.0,
        latest_tps: 67.8,
        avg_duration_ms: 4_200,
        avg_ttft_ms: 680,
      },
      {
        model_id: 'gpt-5.3-codex',
        provider_id: 'codex',
        sample_count: 9,
        mean_tps: 108.4,
        p50_tps: 105.0,
        p95_tps: 132.0,
        max_tps: 140.0,
        min_tps: 78.0,
        latest_tps: 112.5,
        avg_duration_ms: 2_100,
        avg_ttft_ms: 420,
      },
    ],
    ...overrides,
  };
}

function makePlanStatus(overrides: Partial<SubscriptionPlanStatus> = {}): SubscriptionPlanStatus {
  const now = 1_770_000_000_000;
  return {
    plan: {
      plan_id: 'claude_max_5x',
      provider_id: 'claude_code',
      plan_name: 'Claude Max 5x',
      enabled: true,
      monthly_fee_micro_usd: 100_000_000,
      is_user_configured: true,
      updated_at_ms: now,
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
      ],
    },
    windows: [
      {
        window_id: '5h',
        label: '5h 滚动窗口',
        duration_hours: 5,
        rolling: true,
        unit: 'micro_usd',
        used_value: 25_000_000,
        limit_value: 75_000_000,
        used_percent: 33.3,
        warning_percent: 80,
        critical_percent: 95,
        burn_rate_per_hour: 5_000_000,
        projected_exhaustion_hours: 10.0,
        resets_at_ms: now + 7200_000,
        resets_in_seconds: 7200,
        source: 'local_estimate',
        alert_level: 'normal',
      },
    ],
    today_cost_micro_usd: 4_000_000,
    week_cost_micro_usd: 22_000_000,
    month_cost_micro_usd: 85_000_000,
    month_tokens: 1_200_000,
    ...overrides,
  };
}

function makeTemplates(): SubscriptionPlanTemplate[] {
  return [
    {
      template_id: 'claude_max_5x',
      provider_id: 'claude_code',
      plan_name: 'Claude Max 5x ($100/mo)',
      monthly_fee_micro_usd: 100_000_000,
      description: 'Anthropic Claude Max 5x 高配额订阅',
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
      ],
    },
    {
      template_id: 'google_ai_ultra',
      provider_id: 'antigravity',
      plan_name: 'Google AI Ultra ($249.99/mo)',
      monthly_fee_micro_usd: 249_990_000,
      description: 'Google AI Ultra Antigravity 高配额订阅',
      windows: [
        {
          window_id: '5h',
          label: '5h 参考窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 200_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
  ];
}

function resetWorkspaceState() {
  useWorkspaceStore.setState({
    activePage: 'overview',
    timePreset: '7d',
    providerFilter: 'all',
    deviceFilter: 'all',
    currency: 'USD',
    usdToCnyRate: '7.25',
    theme: 'dark',
    resolvedTheme: 'dark',
  });
  document.documentElement.dataset.theme = 'dark';
}

describe('TokenVane formatting & currency conversion', () => {
  it('formats token quantities concisely', () => {
    expect(formatTokens(0)).toBe('0');
    expect(formatTokens(950)).toBe('950');
    expect(formatTokens(12_500)).toBe('12.5K');
    expect(formatTokens(1_500_000)).toBe('1.50M');
    expect(formatTokens(2_250_000_000)).toBe('2.25B');
  });

  it('converts micro-USD accurately to USD and CNY', () => {
    expect(convertMicroUsd(1_500_000, 'USD', '7.25')).toBeCloseTo(1.5, 6);
    expect(convertMicroUsd(1_500_000, 'CNY', '7.25')).toBeCloseTo(10.875, 6);
    expect(formatCost(1_500_000, 'USD', '7.25')).toBe('$1.50');
    expect(formatCost(1_500_000, 'CNY', '7.25')).toBe('¥10.88');
  });

  it('formats percentages and reset countdowns cleanly', () => {
    expect(formatPercent(0.842, true)).toBe('84.2%');
    expect(formatCountdown(0)).toBe('已重置 / 空闲');
    expect(formatCountdown(3 * 3600 + 25 * 60)).toBe('3h 25m');
    expect(formatCountdown(2 * 86_400 + 4 * 3600)).toBe('2d 4h');
  });
});

describe('Workspace store & UI shell navigation', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      value: originalMatchMedia,
    });
  });

  it('computes AnalyticsFilter correctly across time presets and providers', () => {
    useWorkspaceStore.getState().setProviderFilter('codex');
    useWorkspaceStore.getState().setTimePreset('30d');
    const filter = useWorkspaceStore.getState().getFilter();
    expect(filter.provider_id).toBe('codex');
    expect(filter.all_time).toBe(false);
    expect(filter.end_ms - filter.start_ms).toBeGreaterThanOrEqual(29 * 86_400_000);

    useWorkspaceStore.getState().setTimePreset('all');
    const allFilter = useWorkspaceStore.getState().getFilter();
    expect(allFilter.all_time).toBe(true);
    expect(allFilter.start_ms).toBe(0);
  });

  it('renders Linear-style flat workspace and navigates to DataSources and Preferences', async () => {
    renderWithQuery(<App />);

    expect(screen.getByText('TokenVane')).toBeInTheDocument();
    expect(await screen.findByText(/今日 API 等效费用/)).toBeInTheDocument();
    expect(screen.getByText('今日 24h 分时活跃度')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '数据源与合规' }));
    expect(await screen.findByRole('heading', { name: '数据源与合规' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '偏好设置' }));
    expect(await screen.findByRole('heading', { name: '偏好设置' })).toBeInTheDocument();
  });

  it('renders TrayPopupPage compact summary without errors', async () => {
    renderWithQuery(<TrayPopupPage />);
    expect(await screen.findByText('TokenVane 实时摘要')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '打开工作台' })).toBeInTheDocument();
  });
});

describe('Overview & Trends defaults, Today Live Pulse, and all-time preset', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('queries strictly today bounds on OverviewPage regardless of global timePreset and renders intraday pulse', async () => {
    const overviewSpy = vi
      .spyOn(ipc, 'getOverview')
      .mockResolvedValue(makeOverviewReport());

    renderWithQuery(<OverviewPage />);

    expect(await screen.findByText('今日实时概览')).toBeInTheDocument();
    expect(screen.queryByTestId('time-range-control')).not.toBeInTheDocument();

    const chart = await screen.findByTestId('echart-stub');
    expect(chart).toHaveAttribute('data-series-names', 'Input,Output,Cache Read,Cache Write');
    expect(screen.getByText('较昨日 +9.8%')).toBeInTheDocument();
    expect(screen.getByText('较昨日 +14.5%')).toBeInTheDocument();

    // Even if global timePreset becomes 'all', OverviewPage continues querying only today's bounds
    act(() => {
      useWorkspaceStore.getState().setTimePreset('all');
    });
    expect(overviewSpy).toHaveBeenCalledWith(
      expect.objectContaining({ all_time: false })
    );
    expect(overviewSpy).not.toHaveBeenCalledWith(
      expect.objectContaining({ all_time: true })
    );
  });

  it('defaults Trends to Auto granularity & Provider dimension and hides previous-period deltas for All preset', async () => {
    const trendsSpy = vi
      .spyOn(ipc, 'getTrends')
      .mockResolvedValue(makeTrendsReport({ effective_granularity: 'daily' }));

    renderWithQuery(<TrendsPage />);

    expect(await screen.findByText('自动: 天')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '自动' })).toHaveAttribute('data-active', 'true');
    expect(screen.getByRole('button', { name: '按模型提供商' })).toHaveAttribute(
      'data-active',
      'true'
    );
    expect(screen.getByTestId('echart-stub')).toHaveAttribute(
      'data-series-names',
      'Claude Code,Antigravity'
    );
    expect(trendsSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        granularity: 'auto',
        dimension: 'provider',
        filter: expect.objectContaining({ all_time: false }),
      })
    );
    expect(screen.getByText('区间增量变化对比')).toBeInTheDocument();
    expect(screen.getByText('上一周期 Token')).toBeInTheDocument();
    expect(screen.getByText('Token 环比')).toBeInTheDocument();
    expect(screen.getByText('费用环比')).toBeInTheDocument();
    expect(screen.getByRole('columnheader', { name: '缓存命中率' })).toBeInTheDocument();

    // Navigate to TrendsPage, select '全部', and verify all-time period deltas hide cleanly
    cleanup();
    trendsSpy.mockResolvedValue(makeTrendsReport({ effective_granularity: 'weekly' }));
    const prefs = makePreferences();
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap(prefs));
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(prefs);
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(makeTraySummary());
    vi.spyOn(ipc, 'getOverview').mockResolvedValue(makeOverviewReport());

    renderWithQuery(<App />);
    fireEvent.click(await screen.findByRole('button', { name: '趋势分析' }));
    fireEvent.click(await screen.findByRole('button', { name: '全部' }));

    await waitFor(() => {
      expect(trendsSpy).toHaveBeenLastCalledWith(
        expect.objectContaining({
          filter: expect.objectContaining({ all_time: true, start_ms: 0 }),
        })
      );
    });
    expect(await screen.findByText('自动: 周')).toBeInTheDocument();
    expect(screen.getByText('全周期累计分布汇总')).toBeInTheDocument();
    expect(screen.queryByText('上一周期 Token')).not.toBeInTheDocument();
    expect(screen.queryByText('Token 环比')).not.toBeInTheDocument();
    expect(screen.queryByText('费用环比')).not.toBeInTheDocument();
    expect(screen.getByRole('columnheader', { name: '缓存命中率' })).toBeInTheDocument();
  });
});

describe('Usage page unified account connection, observed rate limits & quota deletion', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('shows clean positive empty state and renders authoritative observed rate limits without bind button', async () => {
    const observed: ObservedRateLimitWindow[] = [
      {
        provider_id: 'codex',
        window_name: '5h',
        used_percent: 34.5,
        window_minutes: 300,
        resets_at_ms: Date.now() + 7200_000,
        observed_at_ms: Date.now() - 60_000,
      },
    ];
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue(observed);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByRole('heading', { name: '用量' })).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: '添加账户' })).toHaveLength(1);
    expect(await screen.findByText('配额窗口')).toBeInTheDocument();
    expect(screen.getByText('官方实时限额')).toBeInTheDocument();
    expect(screen.getByText('5h 官方限额')).toBeInTheDocument();
    expect(screen.getByText('34.5%')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '绑定订阅' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '添加我的订阅' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '添加按量计费' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '同步套餐目录' })).not.toBeInTheDocument();
  });

  it('connects provider CLI account via unified Add Account login drawer', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    const refreshSpy = vi.spyOn(ipc, 'refreshProviderBalance').mockResolvedValue({
      snapshot: {
        provider_id: 'antigravity',
        currency: 'QUOTA',
        total_balance_micro: null,
        granted_balance_micro: null,
        topped_up_balance_micro: null,
        daily_quota_used_micro: null,
        daily_quota_limit_micro: null,
        fetched_at_ms: now,
      },
      rate_limits_updated: 4,
      endpoint: 'agy -p "/usage" --output-format json',
      from_cache: false,
      cooldown_remaining_secs: 0,
    });
    const loginSpy = vi
      .spyOn(ipc, 'launchProviderCliLogin')
      .mockResolvedValue('已在系统终端打开登录窗口，完成授权后点击同步即可');

    renderWithQuery(<SubscriptionsPage />);

    expect(
      await screen.findByText('接入提供商账号或官方 API 密钥以同步实时配额与余额')
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '添加账户' }));
    expect(await screen.findByRole('heading', { name: '添加账户' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '启动终端登录' }));
    await waitFor(() => {
      expect(loginSpy).toHaveBeenCalledWith('antigravity');
    });

    fireEvent.click(screen.getByRole('button', { name: '同步账户用量' }));
    await waitFor(() => {
      expect(refreshSpy).toHaveBeenCalledWith('antigravity');
    });
  });

  it('deletes an authoritative quota entry after inline confirmation and hides bound observed snapshots', async () => {
    const plan = makePlanStatus();
    const observed: ObservedRateLimitWindow[] = [
      {
        provider_id: 'claude_code',
        window_name: '5h',
        used_percent: 25,
        window_minutes: 300,
        resets_at_ms: Date.now() + 3600_000,
        observed_at_ms: Date.now() - 30_000,
      },
    ];
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([plan]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue(observed);
    const deleteSpy = vi.spyOn(ipc, 'deleteSubscriptionPlan').mockResolvedValue(undefined);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('1 项数据源')).toBeInTheDocument();
    expect(screen.getByText('Claude Max 5x')).toBeInTheDocument();
    // Bound provider (claude_code) must not duplicate inside unbound observed rows
    expect(screen.queryByText('官方实时限额')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '删除 Claude Max 5x' }));
    fireEvent.click(screen.getByRole('button', { name: '确认' }));

    await waitFor(() => {
      expect(deleteSpy).toHaveBeenCalledWith('claude_max_5x');
    });
  });
});

describe('MonthlyPage ROI reference visibility', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('completely hides subscription fee and ROI section when monthly fee is 0', async () => {
    vi.spyOn(ipc, 'getMonthly').mockResolvedValue(
      makeMonthlyReport({
        subscription_monthly_fee_micro_usd: 0,
        roi_multiplier: null,
      })
    );

    renderWithQuery(<MonthlyPage />);

    expect(await screen.findByText('$30.00')).toBeInTheDocument();
    expect(screen.getByText('当月按量 API 等效价值 (USD)')).toBeInTheDocument();
    expect(
      screen.queryByText('订阅回本参考（基于您录入的订阅月费计算）')
    ).not.toBeInTheDocument();
    expect(screen.queryByText('订阅月费基准')).not.toBeInTheDocument();
    expect(screen.queryByText(/等效产出/)).not.toBeInTheDocument();
  });

  it('renders collapsible secondary ROI strip when user configured monthly fee > 0', async () => {
    vi.spyOn(ipc, 'getMonthly').mockResolvedValue(
      makeMonthlyReport({
        api_equivalent_cost_micro_usd: 30_000_000,
        subscription_monthly_fee_micro_usd: 20_000_000,
        roi_multiplier: 1.5,
      })
    );

    renderWithQuery(<MonthlyPage />);

    const toggleBtn = await screen.findByRole('button', {
      name: /订阅回本参考/,
    });
    expect(toggleBtn).toBeInTheDocument();
    expect(screen.getByText('已录入月费 $20.00')).toBeInTheDocument();
    expect(screen.getByText('等效产出 1.50x')).toBeInTheDocument();
    expect(screen.getByText('当月 API 等效产出')).toBeInTheDocument();
    expect(screen.getByText('订阅月费基准')).toBeInTheDocument();

    // Collapsing hides the inner comparison bars cleanly
    fireEvent.click(toggleBtn);
    expect(screen.queryByText('订阅月费基准')).not.toBeInTheDocument();
  });
});

describe('SpeedLatencyPage Provider overview and Model × Provider comparison', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders Provider overview, Model × Provider comparison switch, and sortable table without context-size decay', async () => {
    vi.spyOn(ipc, 'getSpeedLatency').mockResolvedValue(makeSpeedLatencyReport());

    renderWithQuery(<SpeedLatencyPage />);

    expect(await screen.findByText('模型提供商维度速度与延迟总览')).toBeInTheDocument();
    expect(screen.getByText('模型 × 模型提供商速率对比')).toBeInTheDocument();
    expect(screen.getByText('模型 × 模型提供商速度与耗时明细表')).toBeInTheDocument();

    // Context-size scatter/decay UI from v0.1.0 is completely gone
    expect(screen.queryByText(/上下文大小/)).not.toBeInTheDocument();
    expect(screen.queryByText(/衰减/)).not.toBeInTheDocument();

    const byModelBtn = screen.getByRole('button', { name: '按模型分组' });
    const byProviderBtn = screen.getByRole('button', { name: '按模型提供商分组' });
    expect(byModelBtn).toHaveAttribute('data-active', 'true');
    fireEvent.click(byProviderBtn);
    expect(byProviderBtn).toHaveAttribute('data-active', 'true');

    const table = screen.getByRole('table');
    const rowsBefore = within(table).getAllByRole('row');
    // Default sorting by sample_count desc puts claude-opus-4-6 (18) first
    expect(within(rowsBefore[1]).getByText('claude-opus-4-6')).toBeInTheDocument();
    expect(within(rowsBefore[2]).getByText('gpt-5.3-codex')).toBeInTheDocument();

    // Clicking '样本数' header toggles ascending sort -> gpt-5.3-codex (9) becomes first
    fireEvent.click(within(table).getByText(/^样本数/));
    const rowsAfter = within(table).getAllByRole('row');
    expect(within(rowsAfter[1]).getByText('gpt-5.3-codex')).toBeInTheDocument();
  });
});

describe('Theme switching & TrayPopup hydration', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      value: originalMatchMedia,
    });
  });

  it('immediately updates dataset.theme when cycling theme from TitleBar and clicking Preferences theme buttons', async () => {
    installMatchMediaStub(true);

    const persistThemeSpy = vi.fn();
    render(
      <TitleBar
        collectorWatching={true}
        isScanning={false}
        onScanNow={() => {}}
        onPersistTheme={(next) => {
          useWorkspaceStore.getState().setTheme(next);
          persistThemeSpy(next);
        }}
      />
    );

    const cycleBtn = screen.getByRole('button', { name: '切换主题' });
    // dark -> light
    fireEvent.click(cycleBtn);
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(useWorkspaceStore.getState().resolvedTheme).toBe('light');
    expect(persistThemeSpy).toHaveBeenLastCalledWith('light');

    // light -> system (matchMedia lightMatches=true resolves to 'light')
    fireEvent.click(cycleBtn);
    expect(useWorkspaceStore.getState().theme).toBe('system');
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(persistThemeSpy).toHaveBeenLastCalledWith('system');

    // Now test PreferencesPage direct buttons with OS dark preference
    cleanup();
    resetWorkspaceState();
    installMatchMediaStub(false);

    const basePrefs = makePreferences({ theme: 'dark' });
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap(basePrefs));
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(basePrefs);
    const updatePrefsSpy = vi
      .spyOn(ipc, 'updatePreferences')
      .mockImplementation(async (next) => next);

    renderWithQuery(<PreferencesPage />);

    const lightBtn = await screen.findByRole('button', { name: '浅色 (Light)' });
    fireEvent.click(lightBtn);

    await waitFor(() => {
      expect(updatePrefsSpy).toHaveBeenCalledWith(expect.objectContaining({ theme: 'light' }));
    });
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(useWorkspaceStore.getState().theme).toBe('light');
    expect(useWorkspaceStore.getState().resolvedTheme).toBe('light');

    // Switch to '跟随系统' when prefers-color-scheme: light is false -> resolves back to 'dark'
    fireEvent.click(screen.getByRole('button', { name: '跟随系统' }));
    await waitFor(() => {
      expect(updatePrefsSpy).toHaveBeenLastCalledWith(
        expect.objectContaining({ theme: 'system' })
      );
    });
    expect(useWorkspaceStore.getState().theme).toBe('system');
    expect(useWorkspaceStore.getState().resolvedTheme).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('hydrates TrayPopupPage theme and currency from getPreferences on mount', async () => {
    installMatchMediaStub(false);
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(
      makePreferences({ theme: 'light', currency: 'CNY', usd_to_cny_rate: '7.10' })
    );
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({ today_cost_micro_usd: 1_000_000, currency: 'CNY', usd_to_cny_rate: '7.10' })
    );

    renderWithQuery(<TrayPopupPage />);

    expect(await screen.findByText('TokenVane 实时摘要')).toBeInTheDocument();
    await waitFor(() => {
      expect(document.documentElement.dataset.theme).toBe('light');
      expect(useWorkspaceStore.getState().currency).toBe('CNY');
    });
    expect(await screen.findByText('¥7.10')).toBeInTheDocument();
  });
});

describe('Custom Background controls & <WorkspaceBackdrop />', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders WorkspaceBackdrop across none, gradient, and image Data URL modes', async () => {
    const dataUrlSpy = vi
      .spyOn(ipc, 'getBackgroundImageDataUrl')
      .mockResolvedValue('data:image/png;base64,iVBORw0KGgo=');

    const { container, rerender } = renderWithQuery(
      <WorkspaceBackdrop preferences={makePreferences({ bg_mode: 'none' })} />
    );
    expect(container.firstChild).toBeNull();
    expect(dataUrlSpy).not.toHaveBeenCalled();

    rerender(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <WorkspaceBackdrop
          preferences={makePreferences({
            bg_mode: 'gradient',
            bg_preset_id: 'nordic_frost',
            bg_opacity: 0.35,
            bg_blur_px: 20,
          })}
        />
      </QueryClientProvider>
    );
    const gradientLayer = screen.getByTestId('workspace-backdrop-gradient');
    const innerGlow = gradientLayer.firstElementChild as HTMLElement;
    expect(innerGlow.style.opacity).toBe('0.35');
    expect(innerGlow.style.filter).toBe('blur(20px)');
    expect(dataUrlSpy).not.toHaveBeenCalled();

    rerender(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <WorkspaceBackdrop
          preferences={makePreferences({
            bg_mode: 'image',
            bg_image_path: 'wallpaper_1770000000.png',
            bg_opacity: 0.25,
            bg_blur_px: 12,
          })}
        />
      </QueryClientProvider>
    );

    const imageBackdrop = await screen.findByTestId('workspace-backdrop-image');
    const img = imageBackdrop.querySelector('img') as HTMLImageElement;
    expect(img).not.toBeNull();
    expect(img.getAttribute('src')).toBe('data:image/png;base64,iVBORw0KGgo=');
    expect(img.style.filter).toBe('blur(12px)');
    const veil = imageBackdrop.lastElementChild as HTMLElement;
    expect(veil.style.opacity).toBe('0.75');
    expect(dataUrlSpy).toHaveBeenCalledTimes(1);
  });

  it('updates background presets, opacity/blur sliders, and clears wallpaper in PreferencesPage', async () => {
    const initialPrefs = makePreferences({
      bg_mode: 'none',
      bg_image_path: 'wallpaper_old.png',
      bg_preset_id: 'aurora_slate',
      bg_opacity: 0.18,
      bg_blur_px: 16,
    });
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap(initialPrefs));
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(initialPrefs);
    const updatePrefsSpy = vi
      .spyOn(ipc, 'updatePreferences')
      .mockImplementation(async (next) => next);
    const clearBgSpy = vi.spyOn(ipc, 'clearBackgroundImage').mockResolvedValue(undefined);

    renderWithQuery(<PreferencesPage />);

    expect(await screen.findByText('自定义背景')).toBeInTheDocument();

    // Switch to Gradient mode
    fireEvent.click(screen.getByRole('button', { name: '微光渐变' }));
    await waitFor(() => {
      expect(updatePrefsSpy).toHaveBeenCalledWith(
        expect.objectContaining({ bg_mode: 'gradient' })
      );
    });

    // All 4 presets appear; select '北欧霜蓝'
    expect(await screen.findByText('极光石板')).toBeInTheDocument();
    expect(screen.getByText('黑曜薄雾')).toBeInTheDocument();
    expect(screen.getByText('琥珀暮光')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /北欧霜蓝/ }));
    await waitFor(() => {
      expect(updatePrefsSpy).toHaveBeenLastCalledWith(
        expect.objectContaining({ bg_mode: 'gradient', bg_preset_id: 'nordic_frost' })
      );
    });

    // Adjust opacity & blur sliders and release pointer to flush persistence
    const opacitySlider = screen.getByLabelText('背景可见度');
    fireEvent.change(opacitySlider, { target: { value: '0.42' } });
    fireEvent.pointerUp(opacitySlider);
    await waitFor(() => {
      expect(updatePrefsSpy).toHaveBeenLastCalledWith(
        expect.objectContaining({ bg_opacity: 0.42 })
      );
    });

    const blurSlider = screen.getByLabelText('模糊度');
    fireEvent.change(blurSlider, { target: { value: '24' } });
    fireEvent.pointerUp(blurSlider);
    await waitFor(() => {
      expect(updatePrefsSpy).toHaveBeenLastCalledWith(
        expect.objectContaining({ bg_blur_px: 24 })
      );
    });

    // Switch to Image mode and clear existing wallpaper
    fireEvent.click(screen.getByRole('button', { name: '本地壁纸' }));
    expect(await screen.findByText('当前壁纸：wallpaper_old.png')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '清除壁纸' }));
    await waitFor(() => {
      expect(clearBgSpy).toHaveBeenCalledTimes(1);
    });
  });
});

describe('v0.1.2 Interaction consolidation, chart fixes & catalog auto sync', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      value: originalMatchMedia,
    });
  });

  it('removes time presets from TitleBar and mounts <TimeRangeControl /> only in the 4 time-filtered views', async () => {
    render(
      <TitleBar
        collectorWatching={true}
        isScanning={false}
        onScanNow={() => {}}
      />
    );
    expect(screen.queryByTestId('time-range-control')).not.toBeInTheDocument();
    for (const label of ['今天', '7 天', '30 天', '本月', '全部', '自定义']) {
      expect(screen.queryByRole('button', { name: label })).not.toBeInTheDocument();
    }
    cleanup();

    const prefs = makePreferences();
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap(prefs));
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(prefs);
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(makeTraySummary());
    vi.spyOn(ipc, 'getOverview').mockResolvedValue(makeOverviewReport());
    vi.spyOn(ipc, 'getTrends').mockResolvedValue(makeTrendsReport());
    vi.spyOn(ipc, 'getMonthly').mockResolvedValue(makeMonthlyReport());
    vi.spyOn(ipc, 'getModelsPricing').mockResolvedValue({
      rows: [],
      total_cost_micro_usd: 0,
      total_cache_saved_micro_usd: 0,
      catalog: [],
    });
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSpeedLatency').mockResolvedValue({ providers: [], models: [] });

    renderWithQuery(<App />);

    // Overview (default active page): Today-only live pulse, no TimeRangeControl
    expect(await screen.findByText('今日实时概览')).toBeInTheDocument();
    expect(screen.queryByTestId('time-range-control')).not.toBeInTheDocument();

    // Trends: visible
    fireEvent.click(screen.getByRole('button', { name: '趋势分析' }));
    expect(await screen.findByTestId('time-range-control')).toBeInTheDocument();
    fireEvent.click(within(screen.getByTestId('time-range-control')).getByRole('button', { name: '30 天' }));
    expect(useWorkspaceStore.getState().timePreset).toBe('30d');

    // Models & Pricing: visible
    fireEvent.click(screen.getByRole('button', { name: '模型与价格' }));
    expect(await screen.findByTestId('time-range-control')).toBeInTheDocument();

    // Speed & Latency: visible even on empty state
    fireEvent.click(screen.getByRole('button', { name: '速度监测' }));
    expect(
      await screen.findByText(/当前时间窗等待有效耗时样本/)
    ).toBeInTheDocument();
    expect(screen.getByTestId('time-range-control')).toBeInTheDocument();

    // Monthly, Usage, DataSources, Preferences: absent
    fireEvent.click(screen.getByRole('button', { name: '月度统计' }));
    expect(await screen.findByRole('heading', { name: '月度统计' })).toBeInTheDocument();
    expect(screen.queryByTestId('time-range-control')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '用量' }));
    expect(
      await screen.findByText('接入提供商账号或官方 API 密钥以同步实时配额与余额')
    ).toBeInTheDocument();
    expect(screen.queryByTestId('time-range-control')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '数据源与合规' }));
    expect(await screen.findByRole('heading', { name: '数据源与合规' })).toBeInTheDocument();
    expect(screen.queryByTestId('time-range-control')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '偏好设置' }));
    expect(await screen.findByRole('heading', { name: '偏好设置' })).toBeInTheDocument();
    expect(screen.queryByTestId('time-range-control')).not.toBeInTheDocument();
  });

  it('renders MonthlyPage 365-day heatmap with piecewise dimension=1 and positive quartile levels', async () => {
    const levels = computeHeatmapIntensityLevels([
      { date: '2026-11-10', total_tokens: 0, cost_micro_usd: 0, requests: 0, top_model: null },
      { date: '2026-11-11', total_tokens: 0, cost_micro_usd: 0, requests: 3, top_model: null },
      { date: '2026-11-12', total_tokens: 1_000, cost_micro_usd: 100, requests: 1, top_model: 'm' },
      { date: '2026-11-13', total_tokens: 5_000, cost_micro_usd: 500, requests: 2, top_model: 'm' },
      { date: '2026-11-14', total_tokens: 20_000, cost_micro_usd: 2_000, requests: 4, top_model: 'm' },
      { date: '2026-11-15', total_tokens: 100_000, cost_micro_usd: 9_000, requests: 10, top_model: 'm' },
    ]);
    expect(levels.get('2026-11-10')).toBe(0);
    expect(levels.get('2026-11-11')).toBe(1); // observable_only positive color
    expect(levels.get('2026-11-12')).toBe(1);
    expect(levels.get('2026-11-15')).toBe(4);

    vi.spyOn(ipc, 'getMonthly').mockResolvedValue(
      makeMonthlyReport({
        heatmap_365d: [
          {
            date: '2026-11-11',
            total_tokens: 0,
            cost_micro_usd: 0,
            requests: 2,
            top_model: null,
          },
          {
            date: '2026-11-15',
            total_tokens: 492_000,
            cost_micro_usd: 8_200_000,
            requests: 32,
            top_model: 'claude-opus-4-6',
          },
        ],
      })
    );

    renderWithQuery(<MonthlyPage />);
    expect(await screen.findByTestId('heatmap-legend')).toBeInTheDocument();

    await waitFor(() => {
      const chart = screen.getByTestId('echart-stub');
      const parsed = JSON.parse(chart.getAttribute('data-option-json') || '{}') as EChartMockOption;
      expect(parsed.visualMap?.type).toBe('piecewise');
      expect(parsed.visualMap?.dimension).toBe(1);
      expect(parsed.visualMap?.pieces?.map((p) => p.value)).toEqual([0, 1, 2, 3, 4]);
      const seriesData = parsed.series?.[0]?.data as
        | Array<[string, number, number, number, string, number]>
        | undefined;
      expect(seriesData).toHaveLength(2);
      expect(seriesData?.[0][1]).toBe(1);
      expect(seriesData?.[1][1]).toBeGreaterThanOrEqual(1);
    });
  });

  it('filters zero-token series and decouples TokenType colors & bar/line modes in TrendsPage & OverviewPage pulse', async () => {
    vi.spyOn(ipc, 'getOverview').mockResolvedValue(makeOverviewReport());
    renderWithQuery(<OverviewPage />);
    const pulseChart = await screen.findByTestId('echart-stub');
    const pulseOption = JSON.parse(
      pulseChart.getAttribute('data-option-json') || '{}'
    ) as EChartMockOption;
    expect(pulseOption.series?.[0]?.itemStyle?.color).toBe(TOKEN_TYPE_COLORS.input);
    expect(pulseOption.series?.[0]?.itemStyle?.color).toBe('#3b82f6');
    cleanup();

    vi.spyOn(ipc, 'getTrends').mockResolvedValue(
      makeTrendsReport({
        series: [
          ...makeTrendsReport().series,
          {
            key: 'codex',
            label: 'Codex',
            total_tokens: 0,
            total_cost_micro_usd: 0,
            points: [
              { bucket_start_ms: 1_769_904_000_000, tokens: 0, cost_micro_usd: 0, requests: 0 },
              { bucket_start_ms: 1_769_990_400_000, tokens: 0, cost_micro_usd: 0, requests: 0 },
            ],
          },
        ],
      })
    );

    renderWithQuery(<TrendsPage />);
    const chart = await screen.findByTestId('echart-stub');
    expect(chart).toHaveAttribute('data-series-names', 'Claude Code,Antigravity');

    const barOption = JSON.parse(chart.getAttribute('data-option-json') || '{}') as EChartMockOption;
    for (const s of barOption.series ?? []) {
      expect(s.type).toBe('bar');
      expect(s.stack).toBe('total');
    }

    fireEvent.click(screen.getByRole('button', { name: '折线' }));
    const lineOption = JSON.parse(
      screen.getByTestId('echart-stub').getAttribute('data-option-json') || '{}'
    ) as EChartMockOption;
    expect(lineOption.series?.[0]?.type).toBe('line');
    expect(lineOption.series?.[0]?.stack).toBeUndefined();
    expect(lineOption.series?.[0]?.lineStyle?.width).toBe(2);
  });

  it('supports manual catalog/rate sync and exchange_rate_auto toggling across ModelsPricingPage and PreferencesPage', async () => {
    const initialPrefs = makePreferences({
      usd_to_cny_rate: '7.25',
      auto_sync_pricing: true,
      exchange_rate_auto: true,
      last_pricing_sync_at_ms: Date.now() - 120_000,
    });
    const catalogReport: ModelStatsReport = {
      rows: [],
      total_cost_micro_usd: 0,
      total_cache_saved_micro_usd: 0,
      catalog: [
        {
          model_pattern: 'claude-sonnet-4-5',
          provider_id: 'claude_code',
          input_per_m_micro_usd: 3_000_000,
          output_per_m_micro_usd: 15_000_000,
          cache_read_per_m_micro_usd: 300_000,
          cache_write_per_m_micro_usd: 3_750_000,
          reasoning_per_m_micro_usd: 15_000_000,
          is_user_override: false,
          effective_from: '2026-11-16',
        },
      ],
    };
    const syncResult: CatalogSyncResult = {
      rate_updated: true,
      usd_to_cny_rate: '7.185',
      pricing_updated: true,
      synced_models_count: 42,
      recomputed_events_count: 15,
      subscriptions_updated: true,
      synced_plans_count: 40,
      synced_at_ms: Date.now(),
      rate_source: 'frankfurter',
      pricing_source: 'models.dev',
      subscription_source: 'pages',
      warning: null,
    };

    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap(initialPrefs));
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(initialPrefs);
    vi.spyOn(ipc, 'getModelsPricing').mockResolvedValue(catalogReport);
    const updatePrefsSpy = vi
      .spyOn(ipc, 'updatePreferences')
      .mockImplementation(async (next) => next);
    const syncSpy = vi.spyOn(ipc, 'syncPricingAndRate').mockResolvedValue(syncResult);

    renderWithQuery(<ModelsPricingPage />);
    expect(
      await screen.findByText(/自动汇率 1 USD = 7\.25 CNY · 已同步公开模型单价 1 款/)
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '立即同步' }));
    await waitFor(() => {
      expect(syncSpy).toHaveBeenCalledWith(true);
    });
    expect(await screen.findByText(/汇率已同步 \(1 USD = 7\.185 CNY/)).toBeInTheDocument();

    // Open drawer & lock exchange rate
    fireEvent.click(screen.getByRole('button', { name: '汇率与定价抽屉' }));
    fireEvent.click(screen.getByRole('button', { name: '锁定此汇率' }));
    await waitFor(() => {
      expect(updatePrefsSpy).toHaveBeenCalledWith(
        expect.objectContaining({ exchange_rate_auto: false })
      );
    });

    // Now verify PreferencesPage controls & version badge
    cleanup();
    resetWorkspaceState();
    renderWithQuery(<PreferencesPage />);

    expect(await screen.findByText(/^v0\.\d+\.\d+$/)).toBeInTheDocument();
    const rateInput = screen.getByLabelText('USD 对 CNY 汇率') as HTMLInputElement;
    expect(rateInput.readOnly).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: '手动锁定' }));
    await waitFor(() => {
      expect(updatePrefsSpy).toHaveBeenCalledWith(
        expect.objectContaining({ exchange_rate_auto: false })
      );
    });
    expect((screen.getByLabelText('USD 对 CNY 汇率') as HTMLInputElement).readOnly).toBe(false);

    fireEvent.click(
      screen.getByLabelText(/自动联网更新模型价格、汇率与 AI 订阅目录/)
    );
    await waitFor(() => {
      expect(updatePrefsSpy).toHaveBeenCalledWith(
        expect.objectContaining({ auto_sync_pricing: false })
      );
    });

    fireEvent.click(screen.getByRole('button', { name: '刷新目录与汇率' }));
    await waitFor(() => {
      expect(syncSpy).toHaveBeenLastCalledWith(true);
    });
  });
});

describe('v0.2.0 Phase 2 Live Summary, Phase 2 Providers/Custom Rules & Cloud Sync', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders 15m active/recent sessions & share bars in OverviewPage and TrayPopupPage click-to-workspace', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getOverview').mockResolvedValue(
      makeOverviewReport({
        today_tokens: 96_000,
        today_cost_micro_usd: 1_250_000,
        provider_shares: [
          {
            key: 'kimi_code',
            label: 'Kimi Code',
            tokens: 60_000,
            token_share_percent: 62.5,
            cost_micro_usd: 800_000,
            cost_share_percent: 64.0,
          },
          {
            key: 'claude_code',
            label: 'Claude Code',
            tokens: 36_000,
            token_share_percent: 37.5,
            cost_micro_usd: 450_000,
            cost_share_percent: 36.0,
          },
        ],
        top_model_shares: [
          {
            key: 'kimi-k2',
            label: 'kimi-k2',
            tokens: 60_000,
            token_share_percent: 62.5,
            cost_micro_usd: 800_000,
            cost_share_percent: 64.0,
          },
        ],
        active_sessions: [
          {
            session_key: 'sess-live-1',
            provider_id: 'kimi_code',
            model_id: 'kimi-k2',
            project_label: 'token-tracker',
            activity_state: 'active',
            started_at_ms: now - 180_000,
            last_active_at_ms: now - 15_000,
            request_count: 6,
            total_tokens: 60_000,
            input_tokens: 45_000,
            output_tokens: 15_000,
            cache_read_tokens: 8_000,
            estimated_cost_micro_usd: 800_000,
            latest_output_tps: 92.4,
          },
          {
            session_key: 'sess-recent-2',
            provider_id: 'claude_code',
            model_id: 'claude-sonnet-4-5',
            project_label: 'sync-worker',
            activity_state: 'recent',
            started_at_ms: now - 600_000,
            last_active_at_ms: now - 360_000,
            request_count: 3,
            total_tokens: 36_000,
            input_tokens: 24_000,
            output_tokens: 12_000,
            cache_read_tokens: 0,
            estimated_cost_micro_usd: 450_000,
            latest_output_tps: 64.0,
          },
        ],
      })
    );

    renderWithQuery(<OverviewPage />);

    expect(await screen.findByText('token-tracker')).toBeInTheDocument();
    expect(screen.getByText('1 活跃')).toBeInTheDocument();
    expect(screen.getByText('1 近期')).toBeInTheDocument();
    expect(screen.getByText('活跃中')).toBeInTheDocument();
    expect(screen.getByText('sync-worker')).toBeInTheDocument();

    cleanup();
    const openMainSpy = vi.spyOn(ipc, 'openMainWorkspace').mockResolvedValue(undefined);
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        today_tokens: 96_000,
        today_cost_micro_usd: 1_250_000,
        provider_shares: [
          {
            key: 'kimi_code',
            label: 'Kimi Code',
            tokens: 60_000,
            token_share_percent: 62.5,
            cost_micro_usd: 800_000,
            cost_share_percent: 64.0,
          },
        ],
        active_sessions: [
          {
            session_key: 'sess-live-1',
            provider_id: 'kimi_code',
            model_id: 'kimi-k2',
            project_label: 'token-tracker',
            activity_state: 'active',
            started_at_ms: now - 180_000,
            last_active_at_ms: now - 15_000,
            request_count: 6,
            total_tokens: 60_000,
            input_tokens: 45_000,
            output_tokens: 15_000,
            cache_read_tokens: 8_000,
            estimated_cost_micro_usd: 800_000,
            latest_output_tps: 92.4,
          },
        ],
      })
    );

    renderWithQuery(<TrayPopupPage />);
    expect(await screen.findByText('活跃会话（15m）')).toBeInTheDocument();
    expect(await screen.findByText('63%')).toBeInTheDocument();

    const sessionBtn = screen.getByRole('button', { name: /Kimi Code · kimi-k2/ });
    fireEvent.click(sessionBtn);
    await waitFor(() => {
      expect(openMainSpy).toHaveBeenCalledWith('overview', 'kimi_code');
    });
  });

  it('refreshes Phase 2 official balance and dry-runs declarative custom rule lines in DataSourcesPage', async () => {
    const now = Date.now();
    const configs: ProviderConfig[] = [
      {
        provider_id: 'kimi_code',
        enabled: true,
        collection_mode: 'local_log',
        custom_log_paths: [],
        has_credential: true,
        last_sync_at_ms: now - 10_000,
        last_error: null,
        updated_at_ms: now,
      },
    ];

    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue(configs);
    vi.spyOn(ipc, 'getIngestionCursors').mockResolvedValue([]);
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'getProviderBalances').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());

    const balanceOutcome: BalanceRefreshOutcome = {
      snapshot: {
        provider_id: 'kimi_code',
        currency: 'CNY',
        total_balance_micro: 88_500_000,
        granted_balance_micro: 10_000_000,
        topped_up_balance_micro: 78_500_000,
        daily_quota_used_micro: null,
        daily_quota_limit_micro: null,
        fetched_at_ms: now,
      },
      rate_limits_updated: 0,
      endpoint: 'https://api.moonshot.cn/v1/users/me/balance',
      from_cache: false,
      cooldown_remaining_secs: 60,
    };
    const refreshBalanceSpy = vi
      .spyOn(ipc, 'refreshProviderBalance')
      .mockResolvedValue(balanceOutcome);

    const dryRunResult: CustomRuleTestResult = {
      matched: true,
      provider_id: 'custom_demo',
      model_id: 'gpt-4o',
      timestamp_ms: now,
      input_tokens: 1_200,
      output_tokens: 340,
      cache_read_tokens: 500,
      cache_write_tokens: 0,
      reasoning_tokens: 0,
      estimated_cost_micro_usd: 12_500,
      diagnostic: 'parsed ok',
    };
    const testSampleSpy = vi
      .spyOn(ipc, 'testCustomProviderSampleLine')
      .mockResolvedValue(dryRunResult);

    renderWithQuery(<DataSourcesPage />);

    const refreshBtn = await screen.findByRole('button', { name: '刷新余额' });
    fireEvent.click(refreshBtn);

    await waitFor(() => {
      expect(refreshBalanceSpy).toHaveBeenCalledWith('kimi_code');
    });
    expect(await screen.findByText('CNY 88.50')).toBeInTheDocument();

    const dryRunBtn = screen.getByRole('button', { name: '试跑验证样本' });
    fireEvent.click(dryRunBtn);

    await waitFor(() => {
      expect(testSampleSpy).toHaveBeenCalledTimes(1);
    });
    expect(await screen.findByText('匹配成功')).toBeInTheDocument();
    expect(screen.getByText('模型: gpt-4o')).toBeInTheDocument();
    expect(screen.getByText('输入: 1,200')).toBeInTheDocument();
  });

  it('switches multi-device filter in TitleBar, configures E2EE sync in PreferencesPage & shows read-only remote snapshots in SubscriptionsPage', async () => {
    const now = Date.now();
    const devices: DeviceRecord[] = [
      {
        device_id: 'dev-win-local',
        device_name: 'WorkPC',
        platform: 'windows',
        created_at_ms: now - 100_000,
      },
      {
        device_id: 'dev-mac-remote',
        device_name: 'MacBookPro',
        platform: 'macos',
        created_at_ms: now - 50_000,
      },
    ];
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'listDevices').mockResolvedValue(devices);

    render(
      <TitleBar
        collectorWatching={true}
        isScanning={false}
        onScanNow={() => {}}
      />
    );

    const deviceSelect = await screen.findByLabelText('设备筛选');
    expect(within(deviceSelect).getByRole('option', { name: '全部设备' })).toBeInTheDocument();
    expect(
      within(deviceSelect).getByRole('option', { name: '本机 · WorkPC' })
    ).toBeInTheDocument();
    expect(within(deviceSelect).getByRole('option', { name: 'MacBookPro' })).toBeInTheDocument();

    fireEvent.change(deviceSelect, { target: { value: 'dev-mac-remote' } });
    expect(useWorkspaceStore.getState().deviceFilter).toBe('dev-mac-remote');

    // Now test PreferencesPage Cloud Sync & E2EE workflow
    cleanup();
    let currentSyncStatus: SyncStatusView = {
      enabled: false,
      worker_url: '',
      e2ee_enabled: false,
      e2ee_kdf_salt_hex: null,
      has_bearer_token: false,
      has_e2ee_passphrase: false,
      sse_connected: false,
      last_push_watermark_ms: 0,
      last_pull_watermark_ms: 0,
      last_sync_at_ms: now - 30_000,
      last_sync_error: null,
      updated_at_ms: now,
    };
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap());
    const getSyncStatusSpy = vi
      .spyOn(ipc, 'getSyncStatus')
      .mockImplementation(async () => currentSyncStatus);

    const saveSetupSpy = vi
      .spyOn(ipc, 'saveCloudSyncSetup')
      .mockImplementation(async (enabled, workerUrl, bearerToken, e2eeEnabled, e2eePassphrase) => {
        currentSyncStatus = {
          ...currentSyncStatus,
          enabled,
          worker_url: workerUrl,
          e2ee_enabled: e2eeEnabled,
          e2ee_kdf_salt_hex: e2eeEnabled ? 'aabbccddeeff00112233445566778899' : null,
          has_bearer_token: Boolean(bearerToken) || currentSyncStatus.has_bearer_token,
          has_e2ee_passphrase: Boolean(e2eePassphrase) || currentSyncStatus.has_e2ee_passphrase,
          sse_connected: Boolean(bearerToken || currentSyncStatus.has_bearer_token) && enabled,
        };
        return currentSyncStatus;
      });
    const pairingBundleSpy = vi
      .spyOn(ipc, 'getSyncPairingBundle')
      .mockImplementation(async (includePassphrase) => {
        const uri = includePassphrase
          ? 'tokenvane://pair?v=1&url=https%3A%2F%2Fsync.example.workers.dev&token=tok-secret-123&e2ee=1&key=e2ee-pass-999'
          : 'tokenvane://pair?v=1&url=https%3A%2F%2Fsync.example.workers.dev&token=tok-secret-123&e2ee=1';
        const bundle: SyncPairingBundle = {
          pairing_uri: uri,
          pairing_json: JSON.stringify({
            v: 1,
            url: 'https://sync.example.workers.dev',
            token: 'tok-secret-123',
            e2ee: true,
            ...(includePassphrase ? { key: 'e2ee-pass-999' } : {}),
          }),
          worker_url: 'https://sync.example.workers.dev',
          e2ee_enabled: true,
          includes_passphrase: includePassphrase,
        };
        return bundle;
      });
    const triggerSyncSpy = vi.spyOn(ipc, 'triggerCloudSyncNow').mockResolvedValue({
      pushed_hourly_deltas: 2,
      pushed_subscriptions: 1,
      pulled_items: 1,
      applied_hourly_buckets: 3,
      applied_subscriptions: 1,
      server_seq_ms: now,
      synced_at_ms: now,
    });

    renderWithQuery(<PreferencesPage />);

    expect(await screen.findByText('云端同步')).toBeInTheDocument();
    await waitFor(() => {
      expect(getSyncStatusSpy).toHaveBeenCalled();
    });
    expect(await screen.findByText(/30s 前/)).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('启用云端同步'));
    fireEvent.click(screen.getByLabelText(/端到端加密/));
    fireEvent.change(screen.getByLabelText('Worker 服务地址'), {
      target: { value: 'https://sync.example.workers.dev' },
    });
    const tokenInput = screen.getByLabelText('Sync Bearer Token') as HTMLInputElement;
    const passInput = screen.getByLabelText('E2EE 同步口令') as HTMLInputElement;
    fireEvent.change(tokenInput, { target: { value: 'tok-secret-123' } });
    fireEvent.change(passInput, { target: { value: 'e2ee-pass-999' } });
    fireEvent.click(screen.getByRole('button', { name: '保存并连接' }));

    await waitFor(() => {
      expect(saveSetupSpy).toHaveBeenCalledWith(
        true,
        'https://sync.example.workers.dev',
        'tok-secret-123',
        true,
        'e2ee-pass-999'
      );
    });
    await waitFor(() => {
      expect(pairingBundleSpy).toHaveBeenCalledWith(false);
    });
    expect(tokenInput.value).toBe('');
    expect(passInput.value).toBe('');
    expect(await screen.findByText('SSE 实时已连接')).toBeInTheDocument();
    expect(await screen.findByTestId('sync-pairing-qr')).toBeInTheDocument();
    expect(screen.getByLabelText('移动端配对链接')).toHaveValue(
      'tokenvane://pair?v=1&url=https%3A%2F%2Fsync.example.workers.dev&token=tok-secret-123&e2ee=1'
    );

    fireEvent.click(screen.getByRole('button', { name: '立即同步' }));
    await waitFor(() => {
      expect(triggerSyncSpy).toHaveBeenCalledTimes(1);
    });

    // Finally verify read-only remote subscription snapshots in SubscriptionsPage
    cleanup();
    const remoteSnaps: RemoteSubscriptionSnapshot[] = [
      {
        device_id: 'dev-mac-remote',
        device_display_name: 'MacBookPro',
        plan_id: 'minimax-plan',
        window_id: '5h',
        provider_id: 'minimax_code',
        plan_name: 'MiniMax Coding Plan',
        window_label: '5h 滚动窗口',
        used_percent: 42.5,
        resets_at_ms: now + 3_600_000,
        source: 'local_estimate',
        updated_at_ms: now - 12_000,
      },
    ];
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue([]);
    vi.spyOn(ipc, 'getRemoteSubscriptionSnapshots').mockResolvedValue(remoteSnaps);

    renderWithQuery(<SubscriptionsPage />);
    expect(await screen.findByText('跨设备同步配额快照（只读）')).toBeInTheDocument();
    expect(screen.getByText('MacBookPro')).toBeInTheDocument();
    expect(screen.getByText('只读远端快照')).toBeInTheDocument();
    expect(screen.getByText('已用 42.5%')).toBeInTheDocument();
  });

  it('one-click provisions Cloudflare Worker via Wrangler or API token, auto-fills fields, and opens mobile pairing QR', async () => {
    const now = Date.now();
    let currentSyncStatus: SyncStatusView = {
      enabled: false,
      worker_url: null,
      e2ee_enabled: false,
      e2ee_kdf_salt_hex: null,
      has_bearer_token: false,
      has_e2ee_passphrase: false,
      has_cf_api_token: false,
      sse_connected: false,
      last_push_watermark_ms: 0,
      last_pull_watermark_ms: 0,
      last_sync_at_ms: null,
      last_sync_error: null,
      updated_at_ms: now,
    };
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap());
    vi.spyOn(ipc, 'getSyncStatus').mockImplementation(async () => currentSyncStatus);

    const provisionSpy = vi
      .spyOn(ipc, 'provisionCloudSyncWorker')
      .mockImplementation(async (input) => {
        const workerUrl =
          input.mode === 'wrangler_cli'
            ? 'https://tokenvane-sync-worker.demo.workers.dev'
            : 'https://custom-sync-hub.api-edge.workers.dev';
        const bearer = input.mode === 'wrangler_cli' ? 'auto-wrangler-token-48' : 'auto-api-token-48';
        const passphrase = input.e2ee_enabled ? 'auto-e2ee-pass-32' : null;
        currentSyncStatus = {
          ...currentSyncStatus,
          enabled: true,
          worker_url: workerUrl,
          e2ee_enabled: input.e2ee_enabled,
          e2ee_kdf_salt_hex: input.e2ee_enabled ? '00112233445566778899aabbccddeeff' : null,
          has_bearer_token: true,
          has_e2ee_passphrase: Boolean(passphrase),
          has_cf_api_token: input.mode === 'cloudflare_api',
          sse_connected: true,
          last_sync_at_ms: now,
        };
        const params = new URLSearchParams({
          v: '1',
          url: workerUrl,
          token: bearer,
          e2ee: input.e2ee_enabled ? '1' : '0',
        });
        if (passphrase) {
          params.set('key', passphrase);
        }
        const result: CloudSyncProvisionResult = {
          mode: input.mode,
          worker_name: input.worker_name ?? 'tokenvane-sync-worker',
          worker_url: workerUrl,
          bearer_token: bearer,
          e2ee_passphrase: passphrase,
          initial_sync_ok: true,
          status: currentSyncStatus,
          pairing: {
            pairing_uri: `tokenvane://pair?${params.toString()}`,
            pairing_json: JSON.stringify({
              v: 1,
              url: workerUrl,
              token: bearer,
              e2ee: input.e2ee_enabled,
              ...(passphrase ? { key: passphrase } : {}),
            }),
            worker_url: workerUrl,
            e2ee_enabled: input.e2ee_enabled,
            includes_passphrase: Boolean(passphrase),
          },
        };
        return result;
      });

    renderWithQuery(<PreferencesPage />);

    // 1. Default Wrangler CLI mode: click 一键配置 without manual URL or token
    expect(await screen.findByRole('button', { name: 'Wrangler 浏览器授权' })).toHaveAttribute(
      'data-active',
      'true'
    );
    fireEvent.click(screen.getByRole('button', { name: '一键配置' }));

    await waitFor(() => {
      expect(provisionSpy).toHaveBeenCalledWith({
        mode: 'wrangler_cli',
        worker_name: 'tokenvane-sync-worker',
        bearer_token: null,
        e2ee_enabled: false,
        e2ee_passphrase: null,
        cf_api_token: null,
        cf_account_id: null,
      });
    });

    expect((screen.getByLabelText('启用云端同步') as HTMLInputElement).checked).toBe(true);
    expect(screen.getByLabelText('Worker 服务地址')).toHaveValue(
      'https://tokenvane-sync-worker.demo.workers.dev'
    );
    expect(screen.getByLabelText('Sync Bearer Token')).toHaveValue('auto-wrangler-token-48');
    expect(await screen.findByTestId('mobile-pairing-panel')).toBeInTheDocument();
    expect(screen.getByTestId('sync-pairing-qr')).toBeInTheDocument();
    expect(screen.getByLabelText('移动端配对链接')).toHaveValue(
      'tokenvane://pair?v=1&url=https%3A%2F%2Ftokenvane-sync-worker.demo.workers.dev&token=auto-wrangler-token-48&e2ee=0'
    );

    // 2. Switch to Cloudflare API 令牌 mode with custom worker name + E2EE
    fireEvent.click(screen.getByRole('button', { name: 'Cloudflare API 令牌' }));
    const cfTokenInput = screen.getByLabelText('Cloudflare API 令牌') as HTMLInputElement;
    fireEvent.change(cfTokenInput, { target: { value: 'cf-pat-secret-xyz' } });
    fireEvent.change(screen.getByLabelText('Worker 名称'), {
      target: { value: 'custom-sync-hub' },
    });
    fireEvent.change(screen.getByLabelText('Sync Bearer Token'), { target: { value: '' } });
    fireEvent.click(screen.getByLabelText(/端到端加密/));
    fireEvent.click(screen.getByRole('button', { name: '一键配置' }));

    await waitFor(() => {
      expect(provisionSpy).toHaveBeenLastCalledWith({
        mode: 'cloudflare_api',
        worker_name: 'custom-sync-hub',
        bearer_token: null,
        e2ee_enabled: true,
        e2ee_passphrase: null,
        cf_api_token: 'cf-pat-secret-xyz',
        cf_account_id: null,
      });
    });

    expect(cfTokenInput.value).toBe('');
    expect(screen.getByLabelText('Worker 服务地址')).toHaveValue(
      'https://custom-sync-hub.api-edge.workers.dev'
    );
    expect(screen.getByLabelText('Sync Bearer Token')).toHaveValue('auto-api-token-48');
    expect(screen.getByLabelText('E2EE 同步口令')).toHaveValue('auto-e2ee-pass-32');
    expect((screen.getByLabelText('配对码包含口令') as HTMLInputElement).checked).toBe(true);
    expect(screen.getByLabelText('移动端配对链接')).toHaveValue(
      'tokenvane://pair?v=1&url=https%3A%2F%2Fcustom-sync-hub.api-edge.workers.dev&token=auto-api-token-48&e2ee=1&key=auto-e2ee-pass-32'
    );
  });
});

describe('Bug fixes verification: Overview today shares, Kimi QUOTA display & Live/Auto Subscriptions', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders fixed today share headings and subtitle in OverviewPage', async () => {
    useWorkspaceStore.getState().setTimePreset('30d');
    vi.spyOn(ipc, 'getOverview').mockResolvedValue(
      makeOverviewReport({
        provider_shares: [
          {
            key: 'claude_code',
            label: 'Claude Code',
            tokens: 45_000,
            token_share_percent: 100,
            cost_micro_usd: 900_000,
            cost_share_percent: 100,
          },
        ],
        top_model_shares: [
          {
            key: 'claude-sonnet-4-5',
            label: 'claude-sonnet-4-5',
            tokens: 45_000,
            token_share_percent: 100,
            cost_micro_usd: 900_000,
            cost_share_percent: 100,
          },
        ],
      })
    );

    renderWithQuery(<OverviewPage />);

    expect(await screen.findByRole('columnheader', { name: 'Token 占比' })).toBeInTheDocument();
    expect(screen.getByText('今日 Token 占比')).toBeInTheDocument();
    expect(screen.getByRole('columnheader', { name: 'API 等效费用' })).toBeInTheDocument();
    expect(screen.getByText('今日主力模型 Top 3')).toBeInTheDocument();
    expect(screen.getByText('今日实时概览')).toBeInTheDocument();
  });

  it('renders auto-discovered official API plans, live reference tiers, and converts open drawer currency reactively', async () => {
    const now = Date.now();
    const autoPlan: SubscriptionPlanStatus = {
      plan: {
        plan_id: 'auto_kimi_code_kimi_code_allegretto',
        provider_id: 'kimi_code',
        plan_name: 'Kimi Code Allegretto',
        enabled: true,
        monthly_fee_micro_usd: 7_000_000,
        is_user_configured: true,
        auto_source: 'official_api',
        updated_at_ms: now,
        windows: [
          {
            window_id: '5h',
            label: '5h 滚动窗口',
            duration_hours: 5,
            rolling: true,
            unit: 'micro_usd',
            limit_value: 8_000_000,
            warning_percent: 80,
            critical_percent: 95,
          },
        ],
      },
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          used_value: 1_920_000,
          limit_value: 8_000_000,
          used_percent: 24.0,
          warning_percent: 80,
          critical_percent: 95,
          burn_rate_per_hour: 384_000,
          projected_exhaustion_hours: null,
          resets_at_ms: now + 3_600_000,
          resets_in_seconds: 3_600,
          source: 'official_api',
          alert_level: 'normal',
        },
      ],
      today_cost_micro_usd: 1_200_000,
      week_cost_micro_usd: 5_000_000,
      month_cost_micro_usd: 12_000_000,
      month_tokens: 450_000,
    };

    const liveTemplates: SubscriptionPlanTemplate[] = makeTemplates().map((tpl) =>
      tpl.template_id === 'claude_max_5x'
        ? {
            ...tpl,
            live_windows: [
              {
                window_id: '5h',
                label: '5h 滚动窗口',
                duration_hours: 5,
                rolling: true,
                unit: 'micro_usd',
                used_value: 18_750_000,
                limit_value: 75_000_000,
                used_percent: 25.0,
                warning_percent: 80,
                critical_percent: 95,
                burn_rate_per_hour: 3_750_000,
                projected_exhaustion_hours: 15.0,
                resets_at_ms: now + 5_400_000,
                resets_in_seconds: 5_400,
                source: 'local_estimate',
                alert_level: 'normal',
              },
            ],
          }
        : tpl
    );

    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([autoPlan]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue(liveTemplates);
    vi.spyOn(ipc, 'getRemoteSubscriptionSnapshots').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPaygAccounts').mockResolvedValue([]);
    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue([]);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('自动获取 · 官方 API')).toBeInTheDocument();
    expect(screen.getAllByText('官方 API 实时').length).toBeGreaterThanOrEqual(1);
    expect(screen.queryByText('各档位参考配额实时对照')).not.toBeInTheDocument();
    expect(screen.getByText('24.0%')).toBeInTheDocument();

    // Open unified Add Account drawer, switch to API mode & verify reactive USD -> CNY conversion
    fireEvent.click(screen.getByRole('button', { name: '添加账户' }));
    fireEvent.click(await screen.findByRole('button', { name: 'API 密钥接入' }));

    const prepaidInput = screen.getByLabelText(/预充起始余额/) as HTMLInputElement;
    fireEvent.change(prepaidInput, { target: { value: '100' } });

    act(() => {
      useWorkspaceStore.getState().setCurrency('CNY');
    });

    await waitFor(() => {
      expect((screen.getByLabelText(/预充起始余额/) as HTMLInputElement).value).toBe('725');
    });
  });

  it('renders Kimi Code QUOTA snapshot cleanly in DataSourcesPage', async () => {
    const now = Date.now();
    const configs: ProviderConfig[] = [
      {
        provider_id: 'kimi_code',
        enabled: true,
        collection_mode: 'official_api',
        custom_log_paths: [],
        has_credential: true,
        last_sync_at_ms: now - 5_000,
        last_error: null,
        updated_at_ms: now,
      },
    ];

    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue(configs);
    vi.spyOn(ipc, 'getIngestionCursors').mockResolvedValue([]);
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getProviderBalances').mockResolvedValue([
      {
        provider_id: 'kimi_code',
        currency: 'QUOTA',
        total_balance_micro: null,
        granted_balance_micro: null,
        topped_up_balance_micro: null,
        daily_quota_used_micro: 18,
        daily_quota_limit_micro: 100,
        fetched_at_ms: now,
      },
    ]);

    renderWithQuery(<DataSourcesPage />);

    expect(
      await screen.findByText('Kimi Code 实时配额已同步至「用量」')
    ).toBeInTheDocument();
    expect(screen.getByText('周期配额: 18 / 100')).toBeInTheDocument();
    expect(screen.queryByText(/QUOTA 0\.00/)).not.toBeInTheDocument();
  });
});

describe('v0.2.2 Floating Monitor, Session Titles & Subscription Tier Labels', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders FloatingMonitorPage with live token counter, session title, compact toggle, and close IPCs', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(
      makePreferences({ floating_window_enabled: true })
    );
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        today_tokens: 215_000,
        today_cost_micro_usd: 3_400_000,
        latest_output_tps: 78.4,
        active_sessions: [
          {
            session_key: '#a1b2c3',
            session_title: 'Refactor Floating Monitor HUD',
            provider_id: 'claude_code',
            model_id: 'claude-sonnet-4-5',
            project_label: 'TokenVane',
            activity_state: 'active',
            started_at_ms: now - 120_000,
            last_active_at_ms: now - 5_000,
            request_count: 8,
            total_tokens: 80_000,
            input_tokens: 60_000,
            output_tokens: 20_000,
            cache_read_tokens: 0,
            estimated_cost_micro_usd: 1_100_000,
            latest_output_tps: 78.4,
          },
        ],
      })
    );
    const compactSpy = vi.spyOn(ipc, 'setFloatingWindowCompact').mockResolvedValue(undefined);
    const toggleSpy = vi.spyOn(ipc, 'toggleFloatingWindow').mockResolvedValue(false);

    renderWithQuery(<FloatingMonitorPage />);

    expect(screen.getByText('TokenVane')).toBeInTheDocument();
    expect(await screen.findByText('Refactor Floating Monitor HUD')).toBeInTheDocument();
    const speedEl = screen.getByText('78.4 tok/s');
    expect(speedEl).toBeInTheDocument();

    // Hovering over the live speed displays the model breakdown tooltip
    fireEvent.mouseEnter(speedEl);
    const speedTooltip = await screen.findByTestId('live-speed-tooltip');
    expect(within(speedTooltip).getByText('实时速率来源模型')).toBeInTheDocument();
    expect(within(speedTooltip).getByText('claude-sonnet-4-5')).toBeInTheDocument();
    fireEvent.mouseLeave(speedEl);

    fireEvent.click(screen.getByRole('button', { name: '切换精简胶囊模式' }));
    expect(compactSpy).toHaveBeenCalledWith(true);
    expect(await screen.findByRole('button', { name: '展开悬浮窗' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '关闭悬浮窗' }));
    expect(toggleSpy).toHaveBeenCalled();
  });

  it('displays average speed across multiple active models and lists all active models on hover', async () => {
    const now = Date.now();
    window.localStorage.removeItem('tokenvane.floating_config.v1');
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(
      makePreferences({ floating_window_enabled: true })
    );
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        today_tokens: 320_000,
        today_cost_micro_usd: 4_800_000,
        latest_output_tps: 90.0,
        active_sessions: [
          {
            session_key: '#111111',
            session_title: 'Claude Task',
            provider_id: 'claude_code',
            model_id: 'claude-sonnet-4-5',
            project_label: 'TokenVane',
            activity_state: 'active',
            started_at_ms: now - 60_000,
            last_active_at_ms: now - 5_000,
            request_count: 4,
            total_tokens: 40_000,
            input_tokens: 30_000,
            output_tokens: 10_000,
            cache_read_tokens: 0,
            estimated_cost_micro_usd: 600_000,
            latest_output_tps: 80.0,
          },
          {
            session_key: '#222222',
            session_title: 'Codex Task',
            provider_id: 'codex',
            model_id: 'gpt-5.3-codex',
            project_label: 'TokenVane',
            activity_state: 'active',
            started_at_ms: now - 50_000,
            last_active_at_ms: now - 3_000,
            request_count: 3,
            total_tokens: 30_000,
            input_tokens: 20_000,
            output_tokens: 10_000,
            cache_read_tokens: 0,
            estimated_cost_micro_usd: 500_000,
            latest_output_tps: 100.0,
          },
        ],
      })
    );

    renderWithQuery(<FloatingMonitorPage />);

    // Mean of 80.0 and 100.0 is 90.0 tok/s
    const avgSpeedEl = await screen.findByText('90.0 tok/s');
    expect(avgSpeedEl).toBeInTheDocument();
    expect(screen.getByText('实时生成速率 (2 模型均值)')).toBeInTheDocument();

    fireEvent.mouseEnter(avgSpeedEl);
    const tooltip = await screen.findByTestId('live-speed-tooltip');
    expect(within(tooltip).getByText('活跃模型速率 (2 模型均值)')).toBeInTheDocument();
    expect(within(tooltip).getByText('claude-sonnet-4-5')).toBeInTheDocument();
    expect(within(tooltip).getByText('80.0 tok/s')).toBeInTheDocument();
    expect(within(tooltip).getByText('gpt-5.3-codex')).toBeInTheDocument();
    expect(within(tooltip).getByText('100.0 tok/s')).toBeInTheDocument();
  });

  it('supports categorized PreferencesPage, in-place update buttons, and customizable FloatingMonitorPage elements', async () => {
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(
      makePreferences({ floating_window_enabled: true })
    );
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap());
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'listDevices').mockResolvedValue([]);

    const { useUpdaterStore } = await import('../lib/updater');
    act(() => {
      useUpdaterStore.setState({
        status: 'available',
        availableUpdate: {
          version: '0.2.3',
          currentVersion: '0.2.2',
          date: '2026-11-16',
          body: 'Update test',
        },
        downloadedBytes: 0,
        totalBytes: null,
        errorMessage: null,
      });
    });

    renderWithQuery(
      <div>
        <TitleBar
          collectorWatching={true}
          isScanning={false}
          onScanNow={() => {}}
        />
        <PreferencesPage />
      </div>
    );

    // Top-bar update button in TitleBar
    expect(await screen.findByRole('button', { name: '立即更新客户端' })).toBeInTheDocument();
    expect(screen.getByText('更新 v0.2.3')).toBeInTheDocument();

    // Categorized sections in PreferencesPage
    expect(screen.getByText('外观')).toBeInTheDocument();
    expect(screen.getByText('深色/浅色模式')).toBeInTheDocument();
    expect(screen.getByText('自定义背景')).toBeInTheDocument();
    expect(screen.getByText('桌面与悬浮窗')).toBeInTheDocument();
    expect(screen.getByText('货币与定价')).toBeInTheDocument();
    expect(screen.getByText('系统与更新')).toBeInTheDocument();

    // Check-for-update button is replaced by 立即更新 v0.2.3 when update is available
    expect(screen.getByRole('button', { name: '立即更新 v0.2.3' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '检查更新' })).not.toBeInTheDocument();

    act(() => {
      useUpdaterStore.setState({
        status: 'idle',
        availableUpdate: null,
      });
    });

    expect(await screen.findByRole('button', { name: '检查更新' })).toBeInTheDocument();
  });

  it('displays detected subscription tier status without ever showing 未设月费', async () => {
    const now = Date.now();
    const zeroFeePlans: SubscriptionPlanStatus[] = [
      {
        plan: {
          plan_id: 'auto_kimi_code',
          provider_id: 'kimi_code',
          plan_name: 'Kimi Code Vivace',
          enabled: true,
          monthly_fee_micro_usd: 0,
          is_user_configured: true,
          auto_source: 'official_api',
          updated_at_ms: now,
          windows: [
            {
              window_id: '5h',
              label: '5h 官方限额',
              duration_hours: 5,
              rolling: true,
              unit: 'tokens',
              limit_value: 100,
              warning_percent: 80,
              critical_percent: 95,
            },
          ],
        },
        windows: [
          {
            window_id: '5h',
            label: '5h 官方限额',
            duration_hours: 5,
            rolling: true,
            unit: 'tokens',
            used_value: 25,
            limit_value: 100,
            used_percent: 25,
            warning_percent: 80,
            critical_percent: 95,
            burn_rate_per_hour: 5,
            projected_exhaustion_hours: null,
            resets_at_ms: now + 3600_000,
            resets_in_seconds: 3600,
            source: 'official_api',
            alert_level: 'normal',
          },
        ],
        today_cost_micro_usd: 1_000_000,
        week_cost_micro_usd: 4_000_000,
        month_cost_micro_usd: 9_000_000,
        month_tokens: 200_000,
      },
      {
        plan: {
          plan_id: 'custom_window_plan',
          provider_id: 'qcode',
          plan_name: 'QCode 日额度套餐',
          enabled: true,
          monthly_fee_micro_usd: 0,
          is_user_configured: true,
          auto_source: null,
          updated_at_ms: now,
          windows: [
            {
              window_id: '24h',
              label: '24h 日额度',
              duration_hours: 24,
              rolling: true,
              unit: 'tokens',
              limit_value: 100,
              warning_percent: 80,
              critical_percent: 95,
            },
          ],
        },
        windows: [
          {
            window_id: '24h',
            label: '24h 日额度',
            duration_hours: 24,
            rolling: true,
            unit: 'tokens',
            used_value: 10,
            limit_value: 100,
            used_percent: 10,
            warning_percent: 80,
            critical_percent: 95,
            burn_rate_per_hour: 1,
            projected_exhaustion_hours: null,
            resets_at_ms: null,
            resets_in_seconds: 0,
            source: 'local_log',
            alert_level: 'normal',
          },
        ],
        today_cost_micro_usd: 500_000,
        week_cost_micro_usd: 1_500_000,
        month_cost_micro_usd: 3_000_000,
        month_tokens: 80_000,
      },
    ];

    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue(zeroFeePlans);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue([]);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('Kimi Code Vivace')).toBeInTheDocument();
    expect(screen.getByText('官方同步档位')).toBeInTheDocument();
    expect(screen.getByText('官方配额窗口监测')).toBeInTheDocument();
    expect(screen.queryByText(/未设月费/)).not.toBeInTheDocument();
  });

  it('displays explicit or prompt-derived session_title in OverviewPage and TrayPopupPage', async () => {
    const now = Date.now();
    const titledSession = {
      session_key: '#ef12ab',
      session_title: 'Refactor Chart Stacking',
      provider_id: 'claude_code',
      model_id: 'claude-sonnet-4-5',
      project_label: 'TokenVane',
      activity_state: 'active',
      started_at_ms: now - 60_000,
      last_active_at_ms: now - 3_000,
      request_count: 5,
      total_tokens: 45_000,
      input_tokens: 30_000,
      output_tokens: 15_000,
      cache_read_tokens: 0,
      estimated_cost_micro_usd: 500_000,
      latest_output_tps: 68.5,
    };

    vi.spyOn(ipc, 'getOverview').mockResolvedValue(
      makeOverviewReport({
        active_sessions: [titledSession],
      })
    );
    renderWithQuery(<OverviewPage />);
    expect(await screen.findByText('Refactor Chart Stacking')).toBeInTheDocument();

    cleanup();
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        active_sessions: [titledSession],
      })
    );
    renderWithQuery(<TrayPopupPage />);
    expect(await screen.findByText('Refactor Chart Stacking')).toBeInTheDocument();
  });
});

describe('v0.2.5 Pay-As-You-Go, Credits & Floating Toggle', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useWorkspaceStore.setState({
      activePage: 'subscriptions',
      providerFilter: 'all',
      deviceFilter: 'all',
      timePreset: '7d',
      currency: 'USD',
      usdToCnyRate: '7.25',
      theme: 'dark',
      resolvedTheme: 'dark',
      tzOffsetMinutes: 480,
    });
  });

  it('renders DeepSeek CNY balance + Codex Credits and creates a PAYG account in SubscriptionsPage', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue([]);
    vi.spyOn(ipc, 'getRemoteSubscriptionSnapshots').mockResolvedValue([]);
    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue([
      {
        provider_id: 'deepseek_harness',
        enabled: true,
        collection_mode: 'official_api',
        custom_log_paths: [],
        has_credential: true,
        last_sync_at_ms: now,
        last_error: null,
        updated_at_ms: now,
      },
    ]);
    vi.spyOn(ipc, 'getPaygAccounts').mockResolvedValue([
      {
        account: {
          account_id: 'auto_deepseek_harness',
          provider_id: 'deepseek_harness',
          account_name: 'DeepSeek 官方账户余额',
          enabled: true,
          manual_balance_micro_usd: null,
          balance_anchor_ms: null,
          monthly_budget_micro_usd: 50_000_000,
          is_auto: true,
          updated_at_ms: now,
        },
        official_balance: {
          provider_id: 'deepseek_harness',
          currency: 'CNY',
          total_balance_micro: 45_200_000,
          granted_balance_micro: 10_000_000,
          topped_up_balance_micro: 35_200_000,
          daily_quota_used_micro: null,
          daily_quota_limit_micro: null,
          fetched_at_ms: now - 5_000,
        },
        remaining_manual_balance_micro_usd: null,
        today_cost_micro_usd: 1_200_000,
        month_cost_micro_usd: 12_500_000,
        budget_used_percent: 25.0,
      },
      {
        account: {
          account_id: 'auto_codex',
          provider_id: 'codex',
          account_name: 'Codex Credits',
          enabled: true,
          manual_balance_micro_usd: null,
          balance_anchor_ms: null,
          monthly_budget_micro_usd: null,
          is_auto: true,
          updated_at_ms: now,
        },
        official_balance: {
          provider_id: 'codex',
          currency: 'CREDITS',
          total_balance_micro: 766_760_000,
          granted_balance_micro: null,
          topped_up_balance_micro: null,
          daily_quota_used_micro: null,
          daily_quota_limit_micro: null,
          fetched_at_ms: now - 2_000,
        },
        remaining_manual_balance_micro_usd: null,
        today_cost_micro_usd: 2_000_000,
        month_cost_micro_usd: 8_000_000,
        budget_used_percent: null,
      },
    ]);

    const setKeySpy = vi.spyOn(ipc, 'setProviderApiKey').mockResolvedValue(undefined);
    const upsertPaygSpy = vi.spyOn(ipc, 'upsertPaygAccount').mockResolvedValue(undefined);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('DeepSeek 官方账户余额')).toBeInTheDocument();
    expect(screen.getByText('¥45.20')).toBeInTheDocument();
    expect(screen.getByText('766.76 Credits')).toBeInTheDocument();
    expect(screen.queryByText('各档位参考配额实时对照')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '添加账户' }));
    fireEvent.click(await screen.findByRole('button', { name: 'API 密钥接入' }));
    expect(document.querySelector('body > .drawer-backdrop-enter')).not.toBeNull();

    const keyInput = screen.getByLabelText(/官方 API Key/) as HTMLInputElement;
    fireEvent.change(keyInput, { target: { value: 'sk-deepseek-test-123' } });
    const budgetInput = screen.getByLabelText(/月度支出上限/) as HTMLInputElement;
    fireEvent.change(budgetInput, { target: { value: '80' } });

    fireEvent.click(screen.getByRole('button', { name: '保存账户' }));

    await waitFor(() => {
      expect(upsertPaygSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          provider_id: 'deepseek_harness',
          monthly_budget_micro_usd: 80_000_000,
        })
      );
      expect(setKeySpy).toHaveBeenCalledWith('deepseek_harness', 'sk-deepseek-test-123');
    });
  });

  it('reflects active floating window state in TitleBar and toggles via IPC', async () => {
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'listDevices').mockResolvedValue([]);
    const toggleSpy = vi.spyOn(ipc, 'toggleFloatingWindow').mockResolvedValue(true);

    renderWithQuery(
      <TitleBar
        collectorWatching={true}
        isScanning={false}
        floatingEnabled={false}
        onScanNow={vi.fn()}
      />
    );

    const btn = screen.getByRole('button', { name: '切换桌面悬浮窗' });
    expect(btn).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(btn);

    await waitFor(() => {
      expect(toggleSpy).toHaveBeenCalledTimes(1);
    });
  });
});

describe('v0.2.7 Unified 用量 Account Hub & Positive UI Copy', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('omits windowless manual plans and exposes single 添加账户 entry point with login/API modes', async () => {
    const now = Date.now();
    const windowlessPlan: SubscriptionPlanStatus = {
      plan: {
        plan_id: 'manual_cursor',
        provider_id: 'cursor',
        plan_name: 'Cursor Pro (Manual)',
        enabled: true,
        monthly_fee_micro_usd: 20_000_000,
        is_user_configured: true,
        auto_source: null,
        updated_at_ms: now,
        windows: [],
      },
      windows: [],
      today_cost_micro_usd: 5_000_000,
      week_cost_micro_usd: 20_000_000,
      month_cost_micro_usd: 50_000_000,
      month_tokens: 1_000_000,
    };

    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([windowlessPlan]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getRemoteSubscriptionSnapshots').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPaygAccounts').mockResolvedValue([]);
    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue([]);

    renderWithQuery(<SubscriptionsPage />);

    expect(
      await screen.findByText('接入提供商账号或官方 API 密钥以同步实时配额与余额')
    ).toBeInTheDocument();
    // Windowless manual plan must not be displayed
    expect(screen.queryByText('Cursor Pro (Manual)')).not.toBeInTheDocument();
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: '添加账户' })).toHaveLength(1);

    fireEvent.click(screen.getByRole('button', { name: '添加账户' }));
    expect(document.querySelector('body > .drawer-backdrop-enter')).not.toBeNull();
    expect(screen.getByRole('button', { name: '提供商账号登录' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'API 密钥接入' })).toBeInTheDocument();
  });

  it('hides non-API plans without official windows from TrayPopupPage quota section', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        subscriptions: [
          {
            plan: {
              plan_id: 'claude_max_5x',
              provider_id: 'claude_code',
              plan_name: 'Claude Max 5x',
              enabled: true,
              monthly_fee_micro_usd: 100_000_000,
              windows: [],
              is_user_configured: true,
              updated_at_ms: now,
            },
            windows: [],
            today_cost_micro_usd: 5_000_000,
            week_cost_micro_usd: 15_000_000,
            month_cost_micro_usd: 45_000_000,
            month_tokens: 500_000,
          },
          makePlanStatus({
            plan: {
              plan_id: 'codex_plus',
              provider_id: 'codex',
              plan_name: 'ChatGPT Plus / Codex',
              enabled: true,
              monthly_fee_micro_usd: 20_000_000,
              windows: [],
              is_user_configured: true,
              updated_at_ms: now,
            },
            windows: [
              {
                window_id: '5h',
                label: '5h 官方限额',
                duration_hours: 5,
                rolling: true,
                unit: 'micro_usd',
                used_value: 42,
                limit_value: 100,
                used_percent: 42.0,
                warning_percent: 80,
                critical_percent: 95,
                burn_rate_per_hour: 10,
                projected_exhaustion_hours: null,
                resets_at_ms: now + 3600_000,
                resets_in_seconds: 3600,
                source: 'official_log_snapshot',
                alert_level: 'normal',
              },
            ],
          }),
        ],
      })
    );

    renderWithQuery(<TrayPopupPage />);

    expect(await screen.findByText('Codex · 5h 官方限额')).toBeInTheDocument();
    expect(screen.queryByText('等待官方配额同步')).not.toBeInTheDocument();
  });

  it('renders Antigravity official_cli pool windows and percentage readout in SubscriptionsPage', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([
      {
        plan: {
          plan_id: 'auto_antigravity',
          provider_id: 'antigravity',
          plan_name: 'Google AI Ultra',
          enabled: true,
          monthly_fee_micro_usd: 199_990_000,
          is_user_configured: true,
          auto_source: 'official_cli',
          updated_at_ms: now,
          windows: [],
        },
        windows: [
          {
            window_id: 'gemini-5h',
            label: 'Gemini 模型池 · 5h 限额',
            duration_hours: 5,
            rolling: true,
            unit: 'tokens',
            used_value: 37,
            limit_value: 100,
            used_percent: 36.5,
            warning_percent: 80,
            critical_percent: 95,
            burn_rate_per_hour: 0,
            projected_exhaustion_hours: null,
            resets_at_ms: now + 3_600_000,
            resets_in_seconds: 3600,
            source: 'official_cli',
            alert_level: 'normal',
          },
          {
            window_id: '3p-weekly',
            label: 'Claude / GPT 模型池 · 7d 周限额',
            duration_hours: 168,
            rolling: true,
            unit: 'tokens',
            used_value: 0,
            limit_value: 100,
            used_percent: 0.5,
            warning_percent: 80,
            critical_percent: 95,
            burn_rate_per_hour: 0,
            projected_exhaustion_hours: null,
            resets_at_ms: now + 86_400_000,
            resets_in_seconds: 86400,
            source: 'official_cli',
            alert_level: 'normal',
          },
        ],
        today_cost_micro_usd: 2_000_000,
        week_cost_micro_usd: 10_000_000,
        month_cost_micro_usd: 40_000_000,
        month_tokens: 500_000,
      },
    ]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue([]);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('Google AI Ultra')).toBeInTheDocument();
    expect(screen.getByText('自动获取 · 官方 CLI')).toBeInTheDocument();
    expect(screen.getByText('Gemini 模型池 · 5h 限额')).toBeInTheDocument();
    expect(screen.getByText('Claude / GPT 模型池 · 7d 周限额')).toBeInTheDocument();
    expect(screen.getAllByText('官方 CLI 实时').length).toBe(2);
    expect(screen.getByText('已用 36.5%')).toBeInTheDocument();
    expect(screen.getByText('已用 0.5%')).toBeInTheDocument();
    expect(screen.getByText(/剩余 63\.5%/)).toBeInTheDocument();
    expect(screen.getByText(/剩余 99\.5%/)).toBeInTheDocument();
  });

  it('renders Antigravity CLI quota refresh button and synced readout in DataSourcesPage without API key', async () => {
    const now = Date.now();
    const configs: ProviderConfig[] = [
      {
        provider_id: 'antigravity',
        enabled: true,
        collection_mode: 'local_log',
        custom_log_paths: [],
        has_credential: false,
        last_sync_at_ms: now - 2_000,
        last_error: null,
        updated_at_ms: now,
      },
    ];

    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue(configs);
    vi.spyOn(ipc, 'getIngestionCursors').mockResolvedValue([]);
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getProviderBalances').mockResolvedValue([
      {
        provider_id: 'antigravity',
        currency: 'QUOTA',
        total_balance_micro: null,
        granted_balance_micro: null,
        topped_up_balance_micro: null,
        daily_quota_used_micro: null,
        daily_quota_limit_micro: null,
        fetched_at_ms: now,
      },
    ]);
    const refreshSpy = vi.spyOn(ipc, 'refreshProviderBalance').mockResolvedValue({
      snapshot: {
        provider_id: 'antigravity',
        currency: 'QUOTA',
        total_balance_micro: null,
        granted_balance_micro: null,
        topped_up_balance_micro: null,
        daily_quota_used_micro: null,
        daily_quota_limit_micro: null,
        fetched_at_ms: now,
      },
      rate_limits_updated: 4,
      endpoint: 'agy -p "/usage" --output-format json',
      from_cache: false,
      cooldown_remaining_secs: 0,
    });

    renderWithQuery(<DataSourcesPage />);

    expect(
      await screen.findByText('Antigravity 官方 CLI 配额已同步至「用量」')
    ).toBeInTheDocument();
    const refreshBtn = screen.getByRole('button', { name: '刷新 CLI 配额' });
    fireEvent.click(refreshBtn);

    await waitFor(() => {
      expect(refreshSpy).toHaveBeenCalledWith('antigravity');
    });
  });
});

describe('v0.3.0 Analytics Differentiation, Sync Pairing QR, TZ & UX Fixes', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('encodes QR matrices with GF(256) RS parity, finder patterns, and SVG path for pairing URIs', () => {
    expect(gfMul(0x57, 0x83)).toBe(0x31);
    const rs = rsComputeRemainder(new Uint8Array([0x10, 0x20, 0x0c, 0x56]), 7);
    expect(rs).toHaveLength(7);

    const uri =
      'tokenvane://pair?v=1&url=https%3A%2F%2Fsync.example.workers.dev&token=tok-123&e2ee=1';
    const matrix = encodeQrMatrix(uri);
    expect(matrix.version).toBeGreaterThanOrEqual(4);
    expect(matrix.size).toBe(17 + 4 * matrix.version);
    // Top-left 7x7 finder pattern outer border & center 3x3
    for (let i = 0; i < 7; i += 1) {
      expect(matrix.modules[0][i]).toBe(true);
      expect(matrix.modules[6][i]).toBe(true);
      expect(matrix.modules[i][0]).toBe(true);
      expect(matrix.modules[i][6]).toBe(true);
    }
    expect(matrix.modules[3][3]).toBe(true);
    expect(matrix.modules[1][1]).toBe(false);

    render(<QrCodeSvg value={uri} size={140} />);
    const svg = screen.getByTestId('sync-pairing-qr');
    expect(svg.tagName.toLowerCase()).toBe('svg');
    expect(svg.getAttribute('data-qr-version')).toBe(String(matrix.version));
    const pathEl = svg.querySelector('path');
    expect(pathEl?.getAttribute('d')).toMatch(/^M0 0/);
  });

  it('aligns computeTimeRange 7d to exact 7 local calendar days and formats buckets in explicit tzOffsetMinutes', () => {
    const fixedNow = new Date(2026, 10, 17, 15, 22, 10, 500); // Nov 17 15:22:10 local
    const range7d = computeTimeRange('7d', undefined, fixedNow);
    const expectedStart = new Date(2026, 10, 11, 0, 0, 0, 0).getTime();
    const expectedEnd = new Date(2026, 10, 17, 23, 59, 59, 999).getTime();
    expect(range7d.start_ms).toBe(expectedStart);
    expect(range7d.end_ms).toBe(expectedEnd);
    expect(Math.round((range7d.end_ms - range7d.start_ms + 1) / 86_400_000)).toBe(7);

    // Custom reversed dates swap cleanly to chronological local bounds
    const reversedCustom = computeTimeRange(
      'custom',
      { start: '2026-11-17', end: '2026-11-12' },
      fixedNow
    );
    expect(reversedCustom.start_ms).toBe(new Date(2026, 10, 12, 0, 0, 0, 0).getTime());
    expect(reversedCustom.end_ms).toBe(new Date(2026, 10, 17, 23, 59, 59, 999).getTime());

    // Bucket UTC+8 vs UTC formatting
    const utcMidnight = Date.UTC(2026, 10, 16, 16, 0, 0); // 2026-11-17 00:00 in UTC+8 (+480)
    expect(formatBucketInTz(utcMidnight, 'daily', 480)).toBe('2026-11-17');
    expect(formatBucketInTz(utcMidnight, 'hour_only', 480)).toBe('00:00');
    expect(formatBucketInTz(utcMidnight, 'daily', 0)).toBe('2026-11-16');
  });

  it('defers custom date changes in TimeRangeControl until 确定 is clicked and keeps action buttons non-wrapping', () => {
    render(<TimeRangeControl />);

    expect(useWorkspaceStore.getState().timePreset).toBe('7d');
    const initialStart = useWorkspaceStore.getState().customStartDate;
    const initialEnd = useWorkspaceStore.getState().customEndDate;

    fireEvent.click(screen.getByRole('button', { name: '自定义' }));
    expect(screen.getByRole('dialog', { name: '自定义日期范围' })).toBeInTheDocument();

    const startInput = screen.getByLabelText('开始日期');
    const endInput = screen.getByLabelText('结束日期');
    fireEvent.change(startInput, { target: { value: '2026-11-01' } });
    fireEvent.change(endInput, { target: { value: '2026-11-05' } });

    // Store must remain unchanged while editing drafts
    expect(useWorkspaceStore.getState().timePreset).toBe('7d');
    expect(useWorkspaceStore.getState().customStartDate).toBe(initialStart);
    expect(useWorkspaceStore.getState().customEndDate).toBe(initialEnd);

    // 取消 discards drafts
    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    expect(screen.queryByRole('dialog', { name: '自定义日期范围' })).not.toBeInTheDocument();
    expect(useWorkspaceStore.getState().timePreset).toBe('7d');

    // Reopen, edit drafts, confirm commits custom dates cleanly with whitespace-nowrap button
    fireEvent.click(screen.getByRole('button', { name: '自定义' }));
    fireEvent.change(screen.getByLabelText('开始日期'), { target: { value: '2026-11-01' } });
    fireEvent.change(screen.getByLabelText('结束日期'), { target: { value: '2026-11-05' } });
    const confirmBtn = screen.getByRole('button', { name: '确定' });
    expect(confirmBtn.className).toContain('whitespace-nowrap');
    fireEvent.click(confirmBtn);

    expect(useWorkspaceStore.getState().timePreset).toBe('custom');
    expect(useWorkspaceStore.getState().customStartDate).toBe('2026-11-01');
    expect(useWorkspaceStore.getState().customEndDate).toBe('2026-11-05');
  });

  it('collapses floating window element toggles and opacity slider when floating_window_enabled is false', async () => {
    const prefs = makePreferences({ floating_window_enabled: false });
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(prefs);
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap(prefs));
    vi.spyOn(ipc, 'updatePreferences').mockImplementation(async (next) => next);

    renderWithQuery(<PreferencesPage />);

    const toggle = (await screen.findByLabelText('显示桌面实时悬浮窗')) as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    expect(screen.queryByRole('button', { name: 'Token 用量' })).not.toBeInTheDocument();
    expect(screen.queryByLabelText('悬浮窗不透明度')).not.toBeInTheDocument();

    fireEvent.click(toggle);
    expect(await screen.findByRole('button', { name: 'Token 用量' })).toBeInTheDocument();
    expect(screen.getByLabelText('悬浮窗不透明度')).toBeInTheDocument();
  });

  it('supports specialized analytics modes in TrendsPage, MonthlyPage, ModelsPricingPage & SpeedLatencyPage', async () => {
    vi.spyOn(ipc, 'getTrends').mockResolvedValue(makeTrendsReport());
    renderWithQuery(<TrendsPage />);

    expect(await screen.findByTestId('trends-date-span')).toBeInTheDocument();
    expect(screen.getByText('区间总 Token')).toBeInTheDocument();

    // Switch metric to 缓存命中率 -> keeps 柱状 / 折线 switcher visible, defaults to grouped bars, and switches cleanly to line
    fireEvent.click(screen.getByRole('button', { name: '缓存命中率' }));
    expect(screen.getByRole('button', { name: '按 Token 类型' })).toBeDisabled();
    expect(screen.getByRole('button', { name: '柱状' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '折线' })).toBeInTheDocument();

    const cacheBarOpt = JSON.parse(
      screen.getByTestId('echart-stub').getAttribute('data-option-json') || '{}'
    ) as EChartMockOption & { series?: Array<{ stack?: string }> };
    expect(cacheBarOpt.series?.[0]?.type).toBe('bar');
    expect(cacheBarOpt.series?.[0]?.stack).toBeUndefined();
    expect(cacheBarOpt.series?.[0]?.data).toEqual([58, 68]);

    fireEvent.click(screen.getByRole('button', { name: '折线' }));
    const cacheOpt = JSON.parse(
      screen.getByTestId('echart-stub').getAttribute('data-option-json') || '{}'
    ) as EChartMockOption;
    expect(cacheOpt.series?.[0]?.type).toBe('line');
    expect(cacheOpt.series?.[0]?.data).toEqual([58, 68]);

    cleanup();
    vi.spyOn(ipc, 'getMonthly').mockResolvedValue(makeMonthlyReport());
    renderWithQuery(<MonthlyPage />);
    expect(await screen.findByRole('columnheader', { name: '缓存节省 (USD)' })).toBeInTheDocument();
    expect(screen.getByText('当月账单构成')).toBeInTheDocument();
    expect(screen.getByText('提供商账单占比')).toBeInTheDocument();
    expect(screen.getByText('主力模型账单 Top 5')).toBeInTheDocument();

    cleanup();
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getModelsPricing').mockResolvedValue({
      rows: [
        {
          model_id: 'claude-opus-4-6',
          provider_id: 'claude_code',
          requests: 20,
          observable_steps: 45,
          input_tokens: 100_000,
          output_tokens: 30_000,
          cache_read_tokens: 300_000,
          cache_write_tokens: 20_000,
          reasoning_tokens: 0,
          total_tokens: 450_000,
          cache_saved_micro_usd: 3_500_000,
          estimated_cost_micro_usd: 6_000_000,
          cost_share_ratio: 1,
          avg_output_tps: 76.5,
          effective_pricing: null,
        },
      ],
      total_cost_micro_usd: 6_000_000,
      total_cache_saved_micro_usd: 3_500_000,
      catalog: [
        {
          model_pattern: 'claude-opus-4-6',
          provider_id: 'claude_code',
          input_per_m_micro_usd: 15_000_000,
          output_per_m_micro_usd: 75_000_000,
          cache_read_per_m_micro_usd: 1_500_000,
          cache_write_per_m_micro_usd: 18_750_000,
          reasoning_per_m_micro_usd: 75_000_000,
          is_user_override: false,
          effective_from: '2026-11-01',
        },
      ],
    });
    renderWithQuery(<ModelsPricingPage />);
    expect(await screen.findByTestId('models-efficiency-chart')).toBeInTheDocument();
    expect(screen.getByText('76.5 tok/s')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '单价目录' }));
    expect(await screen.findByLabelText('搜索单价目录')).toBeInTheDocument();
    expect(screen.getByText('共 1 条单价规则')).toBeInTheDocument();

    cleanup();
    vi.spyOn(ipc, 'getSpeedLatency').mockResolvedValue(makeSpeedLatencyReport());
    renderWithQuery(<SpeedLatencyPage />);
    expect(await screen.findByText('全局加权均值')).toBeInTheDocument();
    expect(screen.getByText('最快模型 P95')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '响应耗时与首字延迟 (ms)' }));
    const latOpt = JSON.parse(
      screen.getByTestId('echart-stub').getAttribute('data-option-json') || '{}'
    ) as EChartMockOption;
    expect(latOpt.series?.map((s) => s.name)).toEqual(['平均首字延迟', '平均总耗时']);
  });
});
