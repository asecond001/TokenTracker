import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useState } from 'react';
import { ActionNotice } from '../components/ActionNotice';
import { ProvisionProgress, provisionStepPlan } from '../components/ProvisionProgress';
import { TimeRangeScrubber, pickScrubberWindow } from '../components/TimeRangeScrubber';
import { TimeRangeControl } from '../components/TimeRangeControl';
import { installActionAnchorTracking, resetActionAnchor } from '../lib/actionAnchor';
import { IpcCallError, toIpcCallError } from '../lib/ipc';
import { buildReceiptModel } from '../lib/receipt';
import {
  buildCustomRangeKey,
  computeTimeRange,
  formatResolvedDateSpan,
  selectCustomRange,
  useWorkspaceStore,
} from '../stores/workspaceStore';
import { makeMonthlyReport, resetWorkspaceState } from './testUtils';

function mockRect(el: Element, rect: Partial<DOMRect>) {
  const full = {
    x: rect.left ?? 0,
    y: rect.top ?? 0,
    left: 0,
    top: 0,
    right: 0,
    bottom: 0,
    width: 0,
    height: 0,
    ...rect,
    toJSON: () => ({}),
  } as DOMRect;
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue(full);
}

describe('v0.4.5 desktop fixes & features', () => {
  beforeEach(() => {
    resetWorkspaceState();
    resetActionAnchor();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('normalizes Rust IpcError payloads into Error instances with the real message', () => {
    const err = toIpcCallError('provision_cloud_sync_worker', {
      code: 'COLLECTOR_ERROR',
      message: 'Wrangler 执行超时（>30s）',
    });
    expect(err).toBeInstanceOf(Error);
    expect(err).toBeInstanceOf(IpcCallError);
    expect(err.message).toBe('Wrangler 执行超时（>30s）');
    expect((err as IpcCallError).code).toBe('COLLECTOR_ERROR');
    expect(toIpcCallError('x', 'plain failure').message).toBe('plain failure');
    expect(toIpcCallError('x', { code: 'DB_ERROR' }).message).toContain('DB_ERROR');
    const original = new Error('kept');
    expect(toIpcCallError('x', original)).toBe(original);
  });

  it('renders action feedback next to the button that triggered it', () => {
    installActionAnchorTracking();

    function Harness() {
      const [text, setText] = useState<string | null>(null);
      return (
        <div>
          <button type="button" onClick={() => setText('一键配置失败：Wrangler 执行超时')}>
            一键配置
          </button>
          <ActionNotice text={text} kind="error" onClose={() => setText(null)} />
        </div>
      );
    }

    render(<Harness />);
    const button = screen.getByRole('button', { name: '一键配置' });
    mockRect(button, { left: 300, top: 200, right: 380, bottom: 228, width: 80, height: 28 });
    fireEvent.pointerDown(button);
    fireEvent.click(button);

    const notice = screen.getByTestId('action-notice');
    expect(notice).toHaveTextContent('一键配置失败：Wrangler 执行超时');
    expect(notice).toHaveAttribute('role', 'alert');
    // Portalled to body (not inside the page flow) and placed under the button.
    expect(notice.parentElement).toBe(document.body);
    expect(notice.style.left).toBe('300px');
    expect(notice.style.top).toBe('234px');

    fireEvent.click(screen.getByRole('button', { name: '关闭提示' }));
    expect(screen.queryByTestId('action-notice')).not.toBeInTheDocument();
  });

  it('falls back to an inline strip and auto-hides success notices', () => {
    vi.useFakeTimers();
    try {
      const onClose = vi.fn();
      render(<ActionNotice text="已保存" kind="success" onClose={onClose} />);
      const notice = screen.getByTestId('action-notice');
      expect(notice.parentElement).not.toBe(document.body);
      act(() => {
        vi.advanceTimersByTime(5100);
      });
      expect(onClose).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('shows provisioning steps with the current step spinning and failures highlighted', () => {
    expect(provisionStepPlan('wrangler_cli', [])).not.toContain('browser_login');
    expect(provisionStepPlan('wrangler_cli', ['preparing', 'checking_auth', 'browser_login'])).toContain(
      'browser_login'
    );
    expect(provisionStepPlan('cloudflare_api', [])).toContain('resolving_account');

    const { rerender } = render(
      <ProvisionProgress
        mode="wrangler_cli"
        seen={['preparing', 'checking_auth', 'deploying']}
        status="running"
        startedAtMs={Date.now()}
      />
    );
    const progress = screen.getByTestId('provision-progress');
    const states = Array.from(progress.querySelectorAll('[data-step]')).map(
      (el) => `${el.getAttribute('data-step')}:${el.getAttribute('data-state')}`
    );
    expect(states).toEqual([
      'preparing:done',
      'checking_auth:done',
      'deploying:active',
      'writing_secret:pending',
      'saving:pending',
      'initial_sync:pending',
    ]);

    rerender(
      <ProvisionProgress
        mode="wrangler_cli"
        seen={['preparing', 'checking_auth']}
        status="failed"
        startedAtMs={Date.now()}
      />
    );
    expect(
      screen.getByTestId('provision-progress').querySelector('[data-step="checking_auth"]')
    ).toHaveAttribute('data-state', 'failed');
  });

  it('computes minute-precision custom ranges, live "now" anchors and query keys', () => {
    const now = new Date(2026, 10, 18, 15, 0, 0, 0);
    const start = new Date(2026, 10, 18, 13, 30, 0, 0).getTime();

    const live = computeTimeRange('custom', { precise: true, startMs: start, endMs: null }, now);
    expect(live).toEqual({ start_ms: start, end_ms: now.getTime(), all_time: false });
    expect(formatResolvedDateSpan('custom', { precise: true, startMs: start, endMs: null }, now)).toBe(
      '11-18 13:30 ~ 现在'
    );

    const yesterday = new Date(2026, 10, 17, 0, 0, 0, 0).getTime();
    const todayAfternoon = new Date(2026, 10, 18, 15, 0, 0, 0).getTime();
    const fixed = computeTimeRange(
      'custom',
      { precise: true, startMs: todayAfternoon, endMs: yesterday },
      now
    );
    expect(fixed).toEqual({ start_ms: yesterday, end_ms: todayAfternoon, all_time: false });

    const recentStart = Date.now() - 90 * 60_000;
    useWorkspaceStore.getState().setCustomPreciseRange(recentStart, null);
    const state = useWorkspaceStore.getState();
    expect(state.timePreset).toBe('custom');
    expect(state.customPrecise).toBe(true);
    expect(buildCustomRangeKey(state)).toBe(`p:${recentStart}:now`);
    expect(selectCustomRange(state)).toMatchObject({
      precise: true,
      startMs: recentStart,
      endMs: null,
    });

    // Picking whole days again drops the precise override.
    useWorkspaceStore.getState().setCustomDateRange('2026-11-01', '2026-11-05');
    expect(useWorkspaceStore.getState().customPrecise).toBe(false);
    expect(buildCustomRangeKey(useWorkspaceStore.getState())).toBe('d:2026-11-01:2026-11-05');

    expect(pickScrubberWindow(now.getTime() - 90 * 60_000, now.getTime())).toBe('24h');
    expect(pickScrubberWindow(now.getTime() - 3 * 86_400_000, now.getTime())).toBe('7d');
    expect(pickScrubberWindow(now.getTime() - 20 * 86_400_000, now.getTime())).toBe('30d');
  });

  it('drags a new range on the scrubber track and reports live changes plus a final commit', () => {
    const windowStart = new Date(2026, 10, 17, 15, 0, 0, 0).getTime();
    const windowEnd = windowStart + 86_400_000;
    const onChange = vi.fn();
    const onCommit = vi.fn();
    render(
      <TimeRangeScrubber
        windowStartMs={windowStart}
        windowEndMs={windowEnd}
        stepMs={5 * 60_000}
        startMs={windowStart}
        endMs={windowEnd}
        onChange={onChange}
        onCommit={onCommit}
      />
    );
    const track = screen.getByTestId('time-range-scrubber');
    mockRect(track, { left: 0, right: 960, width: 960, top: 0, bottom: 32, height: 32 });

    // Band fills the track, so dragging inside it draws a new range (75% → 100%).
    const band = track.querySelector('[data-scrub="move"]') as HTMLElement;
    fireEvent.pointerDown(band, { pointerId: 1, button: 0, clientX: 720 });
    fireEvent.pointerMove(track, { pointerId: 1, clientX: 840 });
    fireEvent.pointerMove(track, { pointerId: 1, clientX: 960 });
    fireEvent.pointerUp(track, { pointerId: 1, clientX: 960 });

    expect(onChange).toHaveBeenCalled();
    expect(onCommit).toHaveBeenCalledTimes(1);
    const [commitStart, commitEnd] = onCommit.mock.calls[0];
    expect(commitStart).toBe(windowStart + 18 * 3_600_000);
    expect(commitEnd).toBe(windowEnd);

    // Keyboard nudges on a handle commit immediately.
    onCommit.mockClear();
    fireEvent.keyDown(screen.getByRole('slider', { name: '起始时间' }), { key: 'ArrowRight' });
    expect(onCommit).toHaveBeenCalledWith(windowStart + 5 * 60_000, windowEnd);
  });

  it('opens the custom range popover inside the window and restores the previous range on 取消', () => {
    render(<TimeRangeControl />);
    fireEvent.click(screen.getByRole('button', { name: '自定义' }));
    const dialog = screen.getByRole('dialog', { name: '自定义日期范围' });
    expect(dialog.parentElement).toBe(document.body);
    expect(screen.getByTestId('time-range-scrubber')).toBeInTheDocument();
    expect(screen.getByTestId('scrubber-range-label')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '24 小时' }));
    expect(useWorkspaceStore.getState().customPrecise).toBe(true);
    expect(useWorkspaceStore.getState().timePreset).toBe('custom');

    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    expect(useWorkspaceStore.getState().timePreset).toBe('7d');
    expect(useWorkspaceStore.getState().customPrecise).toBe(false);
  });

  it('builds a receipt-style usage bill from aggregates only', () => {
    const report = makeMonthlyReport({
      subscription_monthly_fee_micro_usd: 20_000_000,
      roi_multiplier: 1.5,
    });
    const model = buildReceiptModel(report, 'USD', '7.25', new Date(2026, 10, 18, 9, 30));
    const text = model.lines
      .map((l) => ('text' in l ? l.text : 'left' in l ? `${l.left}|${'mid' in l ? l.mid : ''}|${l.right}` : l.kind))
      .join('\n');

    expect(model.fileName).toBe('tokenvane-receipt-2026-11.png');
    expect(text).toContain('AI 用量小票 · 2026-11');
    expect(text).toContain('claude-opus-4-6|');
    expect(text).toContain('API 等效合计||$30.00');
    expect(text).toContain('订阅实付||$20.00');
    expect(text).toContain('回本倍数||1.5×');
    expect(text).toContain('最忙一天||11-15');
    expect(text).toContain('折合拿铁||≈ 6 杯');
    expect(text).not.toMatch(/[A-Z]:\\|\/Users\//);

    const cny = buildReceiptModel(report, 'CNY', '7.25', new Date(2026, 10, 18, 9, 30));
    expect(cny.lines.some((l) => l.kind === 'row' && l.left === '折合拿铁' && l.right === '≈ 7 杯')).toBe(true);
  });
});
