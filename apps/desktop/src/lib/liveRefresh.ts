import type { Query, QueryClient } from '@tanstack/react-query';

/**
 * Live refresh policy.
 *
 * The backend emits `usage-updated` after every ingest batch, cloud sync run and
 * official-API poll, so data freshness is event-driven. Polling only remains as
 * a slow clock tick for values that drift with wall time alone (rolling quota
 * windows, "today" boundaries, active-session fade-out).
 */

/** Slow clock tick for time-dependent aggregates on the main workspace. */
export const CLOCK_REFRESH_MS = 60_000;

/** Faster clock tick for the always-on floating monitor's live speed readout. */
export const LIVE_CLOCK_REFRESH_MS = 5_000;

/** Minimum spacing between two event-driven refetch waves. */
export const USAGE_REFRESH_THROTTLE_MS = 1_500;

/** Query roots that never change as a result of usage ingestion or sync. */
const STATIC_QUERY_ROOTS: ReadonlySet<string> = new Set([
  'preferences', // pushed via `preferences-updated`
  'custom-provider-rules',
  'floating-visible',
  'bg-image-data-url',
]);

export function isUsageDerivedQuery(query: Pick<Query, 'queryKey'>): boolean {
  const root = query.queryKey[0];
  return typeof root !== 'string' || !STATIC_QUERY_ROOTS.has(root);
}

export interface UsageRefresher {
  /** Request a refetch of usage-derived queries (throttled, deferred while hidden). */
  schedule: () => void;
  dispose: () => void;
}

interface RefresherOptions {
  throttleMs?: number;
  now?: () => number;
}

/**
 * Coalesce bursts of `usage-updated` events into at most one refetch wave per
 * `throttleMs` (leading + trailing edge), and defer refetches while the webview
 * is hidden until it becomes visible again.
 */
export function createUsageRefresher(
  queryClient: QueryClient,
  { throttleMs = USAGE_REFRESH_THROTTLE_MS, now = () => Date.now() }: RefresherOptions = {}
): UsageRefresher {
  let lastRunAt = Number.NEGATIVE_INFINITY;
  let trailingTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingWhileHidden = false;
  let disposed = false;

  const isHidden = () => typeof document !== 'undefined' && document.visibilityState === 'hidden';

  const run = () => {
    if (disposed) return;
    trailingTimer = null;
    lastRunAt = now();
    void queryClient.invalidateQueries({ predicate: isUsageDerivedQuery });
  };

  const schedule = () => {
    if (disposed) return;
    if (isHidden()) {
      pendingWhileHidden = true;
      return;
    }
    if (trailingTimer !== null) return;
    const wait = lastRunAt + throttleMs - now();
    if (wait <= 0) {
      run();
    } else {
      trailingTimer = setTimeout(run, wait);
    }
  };

  const onVisibilityChange = () => {
    if (!isHidden() && pendingWhileHidden) {
      pendingWhileHidden = false;
      schedule();
    }
  };

  if (typeof document !== 'undefined') {
    document.addEventListener('visibilitychange', onVisibilityChange);
  }

  return {
    schedule,
    dispose: () => {
      disposed = true;
      if (trailingTimer !== null) clearTimeout(trailingTimer);
      if (typeof document !== 'undefined') {
        document.removeEventListener('visibilitychange', onVisibilityChange);
      }
    },
  };
}
