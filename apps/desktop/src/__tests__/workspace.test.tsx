import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import {
  convertMicroUsd,
  formatCost,
  formatCountdown,
  formatPercent,
  formatTokens,
} from '../lib/format';
import { TrayPopupPage } from '../pages/TrayPopupPage';
import { useWorkspaceStore } from '../stores/workspaceStore';

// Mock ECharts canvas renderer inside jsdom tests
vi.mock('../components/EChartBox', () => ({
  EChartBox: ({ height }: { height?: number }) => (
    <div data-testid="echart-stub" style={{ height: `${height ?? 240}px` }} />
  ),
}));

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
    useWorkspaceStore.setState({
      activePage: 'overview',
      timePreset: '7d',
      providerFilter: 'all',
      currency: 'USD',
      usdToCnyRate: '7.25',
      theme: 'dark',
    });
  });

  it('computes AnalyticsFilter correctly across time presets and providers', () => {
    useWorkspaceStore.getState().setProviderFilter('codex');
    useWorkspaceStore.getState().setTimePreset('30d');
    const filter = useWorkspaceStore.getState().getFilter();
    expect(filter.provider_id).toBe('codex');
    expect(filter.end_ms - filter.start_ms).toBeGreaterThanOrEqual(29 * 86_400_000);
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
