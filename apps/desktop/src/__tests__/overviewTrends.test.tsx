import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { ipc } from '../lib/ipc';
import { OverviewPage } from '../pages/OverviewPage';
import { TrendsPage } from '../pages/TrendsPage';
import { useWorkspaceStore } from '../stores/workspaceStore';
import {
  renderWithQuery,
  makePreferences,
  makeBootstrap,
  makeTraySummary,
  makeOverviewReport,
  makeTrendsReport,
  resetWorkspaceState,
} from './testUtils';

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
    await waitFor(() =>
      expect(screen.getByTestId('echart-stub')).toHaveAttribute(
        'data-series-names',
        'Claude Code,Antigravity,上一周期'
      )
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
