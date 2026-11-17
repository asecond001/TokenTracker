import { QueryClient } from '@tanstack/react-query';
import { type MockInstance, afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createUsageRefresher, isUsageDerivedQuery } from '../lib/liveRefresh';

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
  document.dispatchEvent(new Event('visibilitychange'));
}

describe('liveRefresh', () => {
  let queryClient: QueryClient;
  let invalidate: MockInstance<QueryClient['invalidateQueries']>;

  beforeEach(() => {
    vi.useFakeTimers();
    setVisibility('visible');
    queryClient = new QueryClient();
    invalidate = vi.spyOn(queryClient, 'invalidateQueries').mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.useRealTimers();
    setVisibility('visible');
  });

  it('skips static query roots', () => {
    expect(isUsageDerivedQuery({ queryKey: ['overview', 'today'] })).toBe(true);
    expect(isUsageDerivedQuery({ queryKey: ['tray_summary', 480] })).toBe(true);
    expect(isUsageDerivedQuery({ queryKey: ['preferences'] })).toBe(false);
    expect(isUsageDerivedQuery({ queryKey: ['bg-image-data-url', 'x'] })).toBe(false);
  });

  it('coalesces bursts into leading and trailing refetch waves', () => {
    const refresher = createUsageRefresher(queryClient, { throttleMs: 1_000 });
    refresher.schedule();
    expect(invalidate).toHaveBeenCalledTimes(1);

    for (let i = 0; i < 10; i += 1) refresher.schedule();
    expect(invalidate).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(1_000);
    expect(invalidate).toHaveBeenCalledTimes(2);

    vi.advanceTimersByTime(5_000);
    expect(invalidate).toHaveBeenCalledTimes(2);
    refresher.dispose();
  });

  it('defers refetch while hidden and flushes once on becoming visible', () => {
    const refresher = createUsageRefresher(queryClient, { throttleMs: 1_000 });
    setVisibility('hidden');
    refresher.schedule();
    refresher.schedule();
    vi.advanceTimersByTime(10_000);
    expect(invalidate).not.toHaveBeenCalled();

    setVisibility('visible');
    expect(invalidate).toHaveBeenCalledTimes(1);
    refresher.dispose();
  });

  it('stops after dispose', () => {
    const refresher = createUsageRefresher(queryClient, { throttleMs: 1_000 });
    refresher.schedule();
    refresher.schedule();
    refresher.dispose();
    vi.advanceTimersByTime(5_000);
    expect(invalidate).toHaveBeenCalledTimes(1);
  });
});
