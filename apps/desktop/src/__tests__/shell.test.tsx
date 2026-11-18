import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { TitleBar } from '../components/TitleBar';
import { WorkspaceBackdrop } from '../components/WorkspaceBackdrop';
import { TOKEN_TYPE_COLORS } from '../lib/format';
import { ipc } from '../lib/ipc';
import { ModelsPricingPage } from '../pages/ModelsPricingPage';
import { MonthlyPage, computeHeatmapIntensityLevels } from '../pages/MonthlyPage';
import { OverviewPage } from '../pages/OverviewPage';
import { PreferencesPage } from '../pages/PreferencesPage';
import { TrayPopupPage } from '../pages/TrayPopupPage';
import { TrendsPage } from '../pages/TrendsPage';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type { CatalogSyncResult, ModelStatsReport } from '../types/ipc';
import {
  originalMatchMedia,
  installMatchMediaStub,
  renderWithQuery,
  makePreferences,
  makeBootstrap,
  makeTraySummary,
  makeOverviewReport,
  makeTrendsReport,
  makeMonthlyReport,
  resetWorkspaceState,
  type EChartMockOption,
} from './testUtils';

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
      subscription_allocations: [],
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

    // 模型对比 (merged models & speed): visible; the 速度 view keeps it on empty state
    fireEvent.click(screen.getByRole('button', { name: '模型对比' }));
    expect(await screen.findByRole('heading', { name: '模型对比' })).toBeInTheDocument();
    expect(screen.getByTestId('time-range-control')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('tab', { name: '速度' }));
    expect(
      await screen.findByText(/当前时间窗等待有效耗时样本/)
    ).toBeInTheDocument();
    expect(screen.getByTestId('time-range-control')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '速度监测' })).not.toBeInTheDocument();

    // 统计: every figure follows the shared range control
    fireEvent.click(screen.getByRole('button', { name: '统计' }));
    expect(await screen.findByRole('heading', { name: '统计' })).toBeInTheDocument();
    expect(screen.getByTestId('time-range-control')).toBeInTheDocument();

    // Usage, DataSources, Preferences: absent

    fireEvent.click(screen.getByRole('button', { name: '用量' }));
    expect(
      await screen.findByText('添加账户以同步配额与余额')
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
    // Line is the default; the dashed previous-period overlay follows the real series.
    await waitFor(() =>
      expect(screen.getByTestId('echart-stub')).toHaveAttribute(
        'data-series-names',
        'Claude Code,Antigravity,上一周期'
      )
    );
    expect(chart).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '对比上一周期' }));
    expect(screen.getByTestId('echart-stub')).toHaveAttribute(
      'data-series-names',
      'Claude Code,Antigravity'
    );

    fireEvent.click(screen.getByRole('button', { name: '柱状' }));
    const barOption = JSON.parse(
      screen.getByTestId('echart-stub').getAttribute('data-option-json') || '{}'
    ) as EChartMockOption;
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
      subscription_allocations: [],
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
