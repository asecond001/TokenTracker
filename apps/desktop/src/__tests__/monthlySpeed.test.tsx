import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ipc } from '../lib/ipc';
import { MonthlyPage } from '../pages/MonthlyPage';
import { SpeedLatencyPage } from '../pages/SpeedLatencyPage';
import {
  renderWithQuery,
  makeMonthlyReport,
  makeSpeedLatencyReport,
  resetWorkspaceState,
} from './testUtils';

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
