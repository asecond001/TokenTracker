import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { TitleBar } from '../components/TitleBar';
import { WorkspaceBackdrop } from '../components/WorkspaceBackdrop';
import {
  TOKEN_TYPE_COLORS,
  convertMicroUsd,
  formatCost,
  formatCountdown,
  formatPercent,
  formatTokens,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { DataSourcesPage } from '../pages/DataSourcesPage';
import { ModelsPricingPage } from '../pages/ModelsPricingPage';
import { MonthlyPage, computeHeatmapIntensityLevels } from '../pages/MonthlyPage';
import { OverviewPage } from '../pages/OverviewPage';
import { PreferencesPage } from '../pages/PreferencesPage';
import { SpeedLatencyPage } from '../pages/SpeedLatencyPage';
import { SubscriptionsPage } from '../pages/SubscriptionsPage';
import { TrayPopupPage } from '../pages/TrayPopupPage';
import { TrendsPage } from '../pages/TrendsPage';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type {
  AppPreferences,
  BalanceRefreshOutcome,
  BootstrapState,
  CatalogSyncResult,
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
  SyncStatusView,
  TraySummary,
  TrendsReport,
} from '../types/ipc';

interface EChartMockOption {
  series?: Array<{ name?: unknown; lineStyle?: { width?: number; opacity?: number }; itemStyle?: { color?: string }; areaStyle?: { opacity?: number }; data?: unknown }>;
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
    last_rate_sync_at_ms: null,
    last_pricing_sync_at_ms: null,
    updated_at_ms: 1_770_000_000_000,
    ...overrides,
  };
}

function makeBootstrap(prefs: AppPreferences = makePreferences()): BootstrapState {
  return {
    db_path: 'C:\\Users\\dev\\AppData\\Local\\TokenTracker\\tokentracker.db',
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
          { bucket_start_ms: b1, tokens: 140_000, cost_micro_usd: 2_100_000, requests: 11 },
          { bucket_start_ms: b2, tokens: 170_000, cost_micro_usd: 2_700_000, requests: 14 },
        ],
      },
      {
        key: 'antigravity',
        label: 'Antigravity',
        total_tokens: 180_000,
        total_cost_micro_usd: 2_100_000,
        points: [
          { bucket_start_ms: b1, tokens: 80_000, cost_micro_usd: 900_000, requests: 6 },
          { bucket_start_ms: b2, tokens: 100_000, cost_micro_usd: 1_200_000, requests: 8 },
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

describe('TokenTracker formatting & currency conversion', () => {
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

    expect(screen.getByText('TokenTracker')).toBeInTheDocument();
    expect(await screen.findByText(/API 等效成本/)).toBeInTheDocument();
    expect(screen.getByText('Token 构成趋势')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '数据源与合规' }));
    expect(await screen.findByText('数据源配置与合规采集边界')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '偏好设置' }));
    expect(await screen.findByText('偏好设置与同步契约')).toBeInTheDocument();
  });

  it('renders TrayPopupPage compact summary without errors', async () => {
    renderWithQuery(<TrayPopupPage />);
    expect(await screen.findByText('TokenTracker 实时摘要')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '打开工作台' })).toBeInTheDocument();
  });
});

describe('Overview & Trends v0.1.1 defaults and all-time preset', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('defaults Overview to Provider dimension and hides deltas under All time preset', async () => {
    const overviewSpy = vi
      .spyOn(ipc, 'getOverview')
      .mockResolvedValue(makeOverviewReport());

    renderWithQuery(<OverviewPage />);

    const providerBtn = await screen.findByRole('button', { name: '按 Provider' });
    const tokenTypeBtn = screen.getByRole('button', { name: '按 Token 类型' });
    expect(providerBtn).toHaveAttribute('data-active', 'true');
    expect(tokenTypeBtn).toHaveAttribute('data-active', 'false');

    const chart = await screen.findByTestId('echart-stub');
    expect(chart).toHaveAttribute('data-series-names', 'Claude Code,Codex');
    expect(screen.getByText('+12.5%')).toBeInTheDocument();

    // Switching to Token Type changes series rendered
    fireEvent.click(tokenTypeBtn);
    expect(tokenTypeBtn).toHaveAttribute('data-active', 'true');
    expect(screen.getByTestId('echart-stub')).toHaveAttribute(
      'data-series-names',
      'Input,Output,Cache Read,Cache Write'
    );

    // Switching timePreset to 'all' sends all_time: true and hides DeltaBadge items
    act(() => {
      useWorkspaceStore.getState().setTimePreset('all');
    });
    await waitFor(() => {
      expect(overviewSpy).toHaveBeenLastCalledWith(
        expect.objectContaining({ all_time: true, start_ms: 0 })
      );
    });
    expect(screen.queryByText('+12.5%')).not.toBeInTheDocument();
    expect(screen.queryByText('+18.4%')).not.toBeInTheDocument();
  });

  it('defaults Trends to Auto granularity & Provider dimension and hides previous-period deltas for All preset', async () => {
    const trendsSpy = vi
      .spyOn(ipc, 'getTrends')
      .mockResolvedValue(makeTrendsReport({ effective_granularity: 'daily' }));

    renderWithQuery(<TrendsPage />);

    expect(await screen.findByText('自动粒度: 天')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '自动' })).toHaveAttribute('data-active', 'true');
    expect(screen.getByRole('button', { name: '按 Provider' })).toHaveAttribute(
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
    expect(screen.getByText('环比变化')).toBeInTheDocument();

    // Select '全部' via App's TitleBar & navigate to Trends to verify full flow
    cleanup();
    trendsSpy.mockResolvedValue(makeTrendsReport({ effective_granularity: 'weekly' }));
    const prefs = makePreferences();
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap(prefs));
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(prefs);
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(makeTraySummary());
    vi.spyOn(ipc, 'getOverview').mockResolvedValue(makeOverviewReport());

    renderWithQuery(<App />);
    fireEvent.click(await screen.findByRole('button', { name: '全部' }));
    fireEvent.click(screen.getByRole('button', { name: '趋势分析' }));

    await waitFor(() => {
      expect(trendsSpy).toHaveBeenLastCalledWith(
        expect.objectContaining({
          filter: expect.objectContaining({ all_time: true, start_ms: 0 }),
        })
      );
    });
    expect(await screen.findByText('自动粒度: 周')).toBeInTheDocument();
    expect(screen.getByText('全周期累计分布汇总')).toBeInTheDocument();
    expect(screen.queryByText('上一周期 Token')).not.toBeInTheDocument();
    expect(screen.queryByText('环比变化')).not.toBeInTheDocument();
  });
});

describe('Subscriptions v0.1.1 zero-preset, templates, deletion & observed rate limits', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('shows zero-preset empty state and unbound observed rate limit snapshots on fresh install', async () => {
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
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue(makeTemplates());

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('0 项已配置')).toBeInTheDocument();
    expect(screen.getByText('尚未添加个人订阅配置')).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: '添加我的订阅' }).length).toBeGreaterThanOrEqual(
      1
    );

    expect(
      await screen.findByText('官方日志可观测限额快照（未绑定月费套餐）')
    ).toBeInTheDocument();
    expect(screen.getByText('官方日志可观测 · 未绑定月费套餐')).toBeInTheDocument();
    expect(screen.getByText('5h 官方限额')).toBeInTheDocument();
    expect(screen.getByText('34.5%')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '绑定订阅' })).toBeInTheDocument();
  });

  it('creates a user-configured plan from template with is_user_configured: true', async () => {
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    const templatesSpy = vi
      .spyOn(ipc, 'getSubscriptionTemplates')
      .mockResolvedValue(makeTemplates());
    const upsertSpy = vi.spyOn(ipc, 'upsertSubscriptionPlan').mockResolvedValue(undefined);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('尚未添加个人订阅配置')).toBeInTheDocument();
    const addButtons = screen.getAllByRole('button', { name: '添加我的订阅' });
    fireEvent.click(addButtons[0]);
    expect(await screen.findByText('添加我的订阅配置')).toBeInTheDocument();
    expect(templatesSpy).toHaveBeenCalled();

    fireEvent.click(await screen.findByText('Claude Max 5x ($100/mo)'));
    const nameInput = screen.getByPlaceholderText('例如 Claude Max 5x 或 Google AI Ultra');
    expect(nameInput).toHaveValue('Claude Max 5x ($100/mo)');

    fireEvent.click(screen.getByRole('button', { name: '保存订阅配置' }));

    await waitFor(() => {
      expect(upsertSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          provider_id: 'claude_code',
          plan_name: 'Claude Max 5x ($100/mo)',
          monthly_fee_micro_usd: 100_000_000,
          is_user_configured: true,
        })
      );
    });
  });

  it('deletes an existing subscription plan after inline confirmation and hides bound observed snapshots', async () => {
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
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue(makeTemplates());
    const deleteSpy = vi.spyOn(ipc, 'deleteSubscriptionPlan').mockResolvedValue(undefined);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('1 项已配置')).toBeInTheDocument();
    expect(screen.getByText('Claude Max 5x')).toBeInTheDocument();
    // Bound provider (claude_code) should not appear as an unbound snapshot group
    expect(
      screen.queryByText('官方日志可观测限额快照（未绑定月费套餐）')
    ).not.toBeInTheDocument();

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
      name: /订阅回本参考（基于您录入的订阅月费计算）/,
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

    expect(await screen.findByText('Provider 维度速度与延迟总览')).toBeInTheDocument();
    expect(screen.getByText('模型 × Provider 速率对比')).toBeInTheDocument();
    expect(screen.getByText('模型 × Provider 速度与耗时明细表')).toBeInTheDocument();

    // Context-size scatter/decay UI from v0.1.0 is completely gone
    expect(screen.queryByText(/上下文大小/)).not.toBeInTheDocument();
    expect(screen.queryByText(/衰减/)).not.toBeInTheDocument();

    const byModelBtn = screen.getByRole('button', { name: '按模型分组' });
    const byProviderBtn = screen.getByRole('button', { name: '按 Provider 分组' });
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

    expect(await screen.findByText('TokenTracker 实时摘要')).toBeInTheDocument();
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

    expect(await screen.findByText('外观与自定义背景')).toBeInTheDocument();

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

    // Overview (default active page): TimeRangeControl visible
    expect(await screen.findByTestId('time-range-control')).toBeInTheDocument();
    fireEvent.click(within(screen.getByTestId('time-range-control')).getByRole('button', { name: '30 天' }));
    expect(useWorkspaceStore.getState().timePreset).toBe('30d');

    // Trends: visible
    fireEvent.click(screen.getByRole('button', { name: '趋势分析' }));
    expect(await screen.findByTestId('time-range-control')).toBeInTheDocument();

    // Models & Pricing: visible
    fireEvent.click(screen.getByRole('button', { name: '模型与价格' }));
    expect(await screen.findByTestId('time-range-control')).toBeInTheDocument();

    // Speed & Latency: visible even on empty state
    fireEvent.click(screen.getByRole('button', { name: '速度监测' }));
    expect(
      await screen.findByText(/当前时间窗内暂无有效耗时样本/)
    ).toBeInTheDocument();
    expect(screen.getByTestId('time-range-control')).toBeInTheDocument();

    // Monthly, Subscriptions, DataSources, Preferences: absent
    fireEvent.click(screen.getByRole('button', { name: '月度统计' }));
    expect(await screen.findByText('月度账单与活跃热力图')).toBeInTheDocument();
    expect(screen.queryByTestId('time-range-control')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '订阅用量' }));
    expect(await screen.findByText('尚未添加个人订阅配置')).toBeInTheDocument();
    expect(screen.queryByTestId('time-range-control')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '数据源与合规' }));
    expect(await screen.findByText('数据源配置与合规采集边界')).toBeInTheDocument();
    expect(screen.queryByTestId('time-range-control')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '偏好设置' }));
    expect(await screen.findByText('偏好设置与同步契约')).toBeInTheDocument();
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

  it('eliminates stacked area border strokes, filters zero-token series, and decouples TokenType colors', async () => {
    vi.spyOn(ipc, 'getOverview').mockResolvedValue(
      makeOverviewReport({
        provider_series: [
          ...makeOverviewReport().provider_series,
          {
            key: 'antigravity',
            label: 'Antigravity',
            total_tokens: 0,
            total_cost_micro_usd: 0,
            points: [
              {
                bucket_start_ms: 1_770_000_000_000,
                tokens: 0,
                cost_micro_usd: 0,
                requests: 0,
              },
            ],
          },
        ],
      })
    );

    renderWithQuery(<OverviewPage />);

    const chart = await screen.findByTestId('echart-stub');
    expect(chart).toHaveAttribute('data-series-names', 'Claude Code,Codex');
    const darkOption = JSON.parse(chart.getAttribute('data-option-json') || '{}') as EChartMockOption;
    for (const s of darkOption.series ?? []) {
      expect(s.lineStyle?.width).toBe(0);
      expect(s.lineStyle?.opacity).toBe(0);
      expect(s.areaStyle?.opacity).toBe(0.42);
    }

    // Switch to light mode -> opacity 0.55; switch to Token Type -> decoupled #0ea5e9 Input color
    act(() => {
      useWorkspaceStore.getState().setTheme('light');
    });
    fireEvent.click(screen.getByRole('button', { name: '按 Token 类型' }));
    const tokenTypeOption = JSON.parse(
      screen.getByTestId('echart-stub').getAttribute('data-option-json') || '{}'
    ) as EChartMockOption;
    expect(tokenTypeOption.series?.[0]?.itemStyle?.color).toBe(TOKEN_TYPE_COLORS.input);
    expect(tokenTypeOption.series?.[0]?.itemStyle?.color).not.toBe('#6366f1');
    expect(tokenTypeOption.series?.[0]?.lineStyle?.width).toBe(0);
    expect(tokenTypeOption.series?.[0]?.areaStyle?.opacity).toBe(0.55);
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
      synced_at_ms: Date.now(),
      rate_source: 'frankfurter',
      pricing_source: 'models.dev',
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

    // Now verify PreferencesPage controls & v0.2.0 badge
    cleanup();
    resetWorkspaceState();
    renderWithQuery(<PreferencesPage />);

    expect(await screen.findByText('v0.2.0')).toBeInTheDocument();
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
      screen.getByLabelText(/自动联网更新开源模型价格目录与汇率/)
    );
    await waitFor(() => {
      expect(updatePrefsSpy).toHaveBeenCalledWith(
        expect.objectContaining({ auto_sync_pricing: false })
      );
    });

    fireEvent.click(screen.getByRole('button', { name: '刷新汇率与价格' }));
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

    const updateSyncSpy = vi
      .spyOn(ipc, 'updateSyncConfig')
      .mockImplementation(async (enabled, workerUrl, e2eeEnabled) => {
        currentSyncStatus = {
          ...currentSyncStatus,
          enabled,
          worker_url: workerUrl,
          e2ee_enabled: e2eeEnabled,
          e2ee_kdf_salt_hex: e2eeEnabled ? 'aabbccddeeff00112233445566778899' : null,
        };
        return currentSyncStatus;
      });
    const setCredsSpy = vi
      .spyOn(ipc, 'setSyncCredentials')
      .mockImplementation(async (token, pass) => {
        currentSyncStatus = {
          ...currentSyncStatus,
          has_bearer_token: Boolean(token) || currentSyncStatus.has_bearer_token,
          has_e2ee_passphrase: Boolean(pass) || currentSyncStatus.has_e2ee_passphrase,
          sse_connected: Boolean(token) && currentSyncStatus.enabled,
        };
        return currentSyncStatus;
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

    expect(await screen.findByText('云端多端同步（Cloudflare Worker）')).toBeInTheDocument();
    await waitFor(() => {
      expect(getSyncStatusSpy).toHaveBeenCalled();
    });
    expect(await screen.findByText(/30s 前/)).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('启用 Cloudflare Worker 云端同步'));
    fireEvent.click(screen.getByLabelText(/启用端到端加密/));
    fireEvent.change(screen.getByLabelText('Worker 服务地址'), {
      target: { value: 'https://sync.example.workers.dev' },
    });
    fireEvent.click(screen.getByRole('button', { name: '保存同步配置' }));

    await waitFor(() => {
      expect(updateSyncSpy).toHaveBeenCalledWith(
        true,
        'https://sync.example.workers.dev',
        true
      );
    });
    expect(await screen.findByText('aabbccddeeff00112233445566778899')).toBeInTheDocument();

    const tokenInput = screen.getByLabelText('Sync Bearer Token') as HTMLInputElement;
    const passInput = screen.getByLabelText('E2EE 同步口令') as HTMLInputElement;
    fireEvent.change(tokenInput, { target: { value: 'tok-secret-123' } });
    fireEvent.change(passInput, { target: { value: 'e2ee-pass-999' } });
    fireEvent.click(screen.getByRole('button', { name: '保存同步凭据' }));

    await waitFor(() => {
      expect(setCredsSpy).toHaveBeenCalledWith('tok-secret-123', 'e2ee-pass-999');
    });
    expect(tokenInput.value).toBe('');
    expect(passInput.value).toBe('');
    expect(await screen.findByText('SSE 实时已连接')).toBeInTheDocument();

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
    expect(screen.getByText('42.5%')).toBeInTheDocument();
  });
});
