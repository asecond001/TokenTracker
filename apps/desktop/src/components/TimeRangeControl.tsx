import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  computeTimeRange,
  formatRangeInstant,
  getPresetDateInputs,
  selectCustomRange,
  useWorkspaceStore,
  type TimePreset,
} from '../stores/workspaceStore';
import {
  SCRUBBER_WINDOWS,
  TimeRangeScrubber,
  pickScrubberWindow,
  snapToStep,
  type ScrubberWindow,
} from './TimeRangeScrubber';

const TIME_PRESETS: Array<{ id: TimePreset; label: string }> = [
  { id: 'today', label: '今天' },
  { id: '7d', label: '7 天' },
  { id: '30d', label: '30 天' },
  { id: 'month', label: '本月' },
  { id: 'all', label: '全部' },
  { id: 'custom', label: '自定义' },
];

/** Minimum interval between live store updates while dragging (keeps queries smooth). */
const LIVE_COMMIT_INTERVAL_MS = 120;
const POPOVER_WIDTH_PX = 500;
const EDGE_PX = 8;

type RangeSnapshot = Pick<
  ReturnType<typeof useWorkspaceStore.getState>,
  'timePreset' | 'customStartDate' | 'customEndDate' | 'customPrecise' | 'customStartMs' | 'customEndMs'
>;

function takeSnapshot(): RangeSnapshot {
  const s = useWorkspaceStore.getState();
  return {
    timePreset: s.timePreset,
    customStartDate: s.customStartDate,
    customEndDate: s.customEndDate,
    customPrecise: s.customPrecise,
    customStartMs: s.customStartMs,
    customEndMs: s.customEndMs,
  };
}

export interface TimeRangeControlProps {
  className?: string;
}

export function TimeRangeControl({ className = '' }: TimeRangeControlProps) {
  const timePreset = useWorkspaceStore((s) => s.timePreset);
  const customStartDate = useWorkspaceStore((s) => s.customStartDate);
  const customEndDate = useWorkspaceStore((s) => s.customEndDate);
  const setTimePreset = useWorkspaceStore((s) => s.setTimePreset);
  const setCustomDateRange = useWorkspaceStore((s) => s.setCustomDateRange);
  const setCustomPreciseRange = useWorkspaceStore((s) => s.setCustomPreciseRange);

  const [open, setOpen] = useState(false);
  const [draftStart, setDraftStart] = useState(customStartDate);
  const [draftEnd, setDraftEnd] = useState(customEndDate);
  const [windowId, setWindowId] = useState<ScrubberWindow>('24h');
  const [windowEndMs, setWindowEndMs] = useState(() => Date.now());
  const [scrub, setScrub] = useState<{ start: number; end: number } | null>(null);
  const [position, setPosition] = useState<{ top: number; left: number; width: number } | null>(
    null
  );

  const rootRef = useRef<HTMLDivElement | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);
  const snapshotRef = useRef<RangeSnapshot | null>(null);
  const liveTimerRef = useRef<number | null>(null);
  const lastLiveCommitRef = useRef(0);
  const pendingLiveRef = useRef<{ start: number; end: number } | null>(null);

  const close = useCallback(() => {
    setOpen(false);
    setScrub(null);
    snapshotRef.current = null;
  }, []);

  useEffect(() => {
    if (!open) {
      return;
    }
    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (rootRef.current?.contains(target) || popoverRef.current?.contains(target)) {
        return;
      }
      close();
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        close();
      }
    };
    document.addEventListener('mousedown', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [open, close]);

  useEffect(
    () => () => {
      if (liveTimerRef.current !== null) {
        window.clearTimeout(liveTimerRef.current);
      }
    },
    []
  );

  // Keep the popover inside the window: anchor under the control, clamp horizontally.
  const reposition = useCallback(() => {
    const root = rootRef.current;
    if (!root) return;
    const rect = root.getBoundingClientRect();
    const viewportW = window.innerWidth || POPOVER_WIDTH_PX + EDGE_PX * 2;
    const width = Math.min(POPOVER_WIDTH_PX, viewportW - EDGE_PX * 2);
    // Align with the control's left edge; flip to right alignment near the window edge.
    const preferredLeft =
      rect.left + width <= viewportW - EDGE_PX ? rect.left : rect.right - width;
    const left = Math.min(Math.max(EDGE_PX, preferredLeft), viewportW - width - EDGE_PX);
    setPosition({ top: rect.bottom + 6, left, width });
  }, []);

  useLayoutEffect(() => {
    if (!open) return;
    reposition();
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    return () => {
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
    };
  }, [open, reposition]);

  const openPopover = () => {
    const state = useWorkspaceStore.getState();
    snapshotRef.current = takeSnapshot();
    const initial = getPresetDateInputs(timePreset, {
      start: customStartDate,
      end: customEndDate,
    });
    setDraftStart(initial.start);
    setDraftEnd(initial.end);

    const now = Date.now();
    const range = computeTimeRange(state.timePreset, selectCustomRange(state), new Date(now));
    const startMs = range.all_time ? now - SCRUBBER_WINDOWS['7d'].spanMs : range.start_ms;
    const nextWindow = pickScrubberWindow(startMs, now);
    setWindowId(nextWindow);
    setWindowEndMs(now);
    setScrub({ start: startMs, end: Math.min(range.end_ms, now) });
    setOpen(true);
  };

  const flushLive = useCallback(
    (range: { start: number; end: number }) => {
      const stepMs = SCRUBBER_WINDOWS[windowId].stepMs;
      // Anchoring the end handle to the right edge keeps the range following "now".
      const live = windowEndMs - range.end < stepMs;
      lastLiveCommitRef.current = Date.now();
      pendingLiveRef.current = null;
      setCustomPreciseRange(range.start, live ? null : range.end);
      const after = useWorkspaceStore.getState();
      setDraftStart(after.customStartDate);
      setDraftEnd(after.customEndDate);
    },
    [setCustomPreciseRange, windowEndMs, windowId]
  );

  const handleScrubChange = (start: number, end: number) => {
    setScrub({ start, end });
    pendingLiveRef.current = { start, end };
    const elapsed = Date.now() - lastLiveCommitRef.current;
    if (elapsed >= LIVE_COMMIT_INTERVAL_MS) {
      flushLive({ start, end });
      return;
    }
    if (liveTimerRef.current === null) {
      liveTimerRef.current = window.setTimeout(() => {
        liveTimerRef.current = null;
        if (pendingLiveRef.current) {
          flushLive(pendingLiveRef.current);
        }
      }, LIVE_COMMIT_INTERVAL_MS - elapsed);
    }
  };

  const handleScrubCommit = (start: number, end: number) => {
    if (liveTimerRef.current !== null) {
      window.clearTimeout(liveTimerRef.current);
      liveTimerRef.current = null;
    }
    setScrub({ start, end });
    flushLive({ start, end });
  };

  const changeWindow = (next: ScrubberWindow) => {
    const now = Date.now();
    const { spanMs, stepMs } = SCRUBBER_WINDOWS[next];
    const windowStart = now - spanMs;
    setWindowId(next);
    setWindowEndMs(now);
    if (!scrub) return;
    const start = Math.max(windowStart, snapToStep(scrub.start, stepMs));
    const end = Math.max(start + stepMs, Math.min(now, scrub.end));
    setScrub({ start, end });
    // Apply the clamped range so the label and the charts stay in sync.
    lastLiveCommitRef.current = Date.now();
    setCustomPreciseRange(start, now - end < stepMs ? null : end);
  };

  const cancel = () => {
    const snapshot = snapshotRef.current;
    if (snapshot) {
      useWorkspaceStore.setState(snapshot);
    }
    close();
  };

  const commitDraftRange = () => {
    const start = draftStart || customStartDate;
    const end = draftEnd || customEndDate;
    if (start && end && start > end) {
      setCustomDateRange(end, start);
    } else {
      setCustomDateRange(start, end);
    }
    close();
  };

  const win = SCRUBBER_WINDOWS[windowId];
  const windowStartMs = windowEndMs - win.spanMs;
  const scrubLive = scrub ? windowEndMs - scrub.end < win.stepMs : false;

  const popover =
    open && typeof document !== 'undefined'
      ? createPortal(
          <div
            ref={popoverRef}
            role="dialog"
            aria-label="自定义日期范围"
            className="liquid-popover fixed z-[70] p-3 space-y-3"
            style={{
              top: position?.top ?? 0,
              left: position?.left ?? 0,
              width: position?.width ?? POPOVER_WIDTH_PX,
              visibility: position ? 'visible' : 'hidden',
            }}
          >
            <div className="flex items-center justify-between gap-3">
              <div className="seg-group" role="group" aria-label="拖选跨度">
                {(Object.keys(SCRUBBER_WINDOWS) as ScrubberWindow[]).map((id) => (
                  <button
                    key={id}
                    type="button"
                    data-active={windowId === id ? 'true' : undefined}
                    onClick={() => changeWindow(id)}
                    className="seg-pill px-2 py-0.5 text-[11px] cursor-pointer whitespace-nowrap"
                    style={{
                      color: windowId === id ? 'var(--accent-tonal-text)' : 'var(--text-secondary)',
                      fontWeight: windowId === id ? 500 : 400,
                    }}
                  >
                    {SCRUBBER_WINDOWS[id].label}
                  </button>
                ))}
              </div>
              {scrub && (
                <span
                  data-testid="scrubber-range-label"
                  className="text-[11.5px] font-medium tabular-nums whitespace-nowrap"
                  style={{ color: 'var(--text-primary)' }}
                >
                  {formatRangeInstant(scrub.start)} ~ {scrubLive ? '现在' : formatRangeInstant(scrub.end)}
                </span>
              )}
            </div>

            {scrub && (
              <TimeRangeScrubber
                windowStartMs={windowStartMs}
                windowEndMs={windowEndMs}
                stepMs={win.stepMs}
                startMs={scrub.start}
                endMs={scrub.end}
                onChange={handleScrubChange}
                onCommit={handleScrubCommit}
              />
            )}

            <div
              className="flex flex-wrap items-center gap-2 pt-2.5"
              style={{ borderTop: '1px solid var(--border-hairline)' }}
            >
              <input
                type="date"
                aria-label="开始日期"
                value={draftStart}
                onChange={(e) => setDraftStart(e.target.value)}
                className="px-2 py-1 text-[11.5px] rounded-lg tabular-nums shrink-0"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
              <span className="text-[11.5px] shrink-0" style={{ color: 'var(--text-muted)' }}>
                至
              </span>
              <input
                type="date"
                aria-label="结束日期"
                value={draftEnd}
                onChange={(e) => setDraftEnd(e.target.value)}
                className="px-2 py-1 text-[11.5px] rounded-lg tabular-nums shrink-0"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
              <div className="ml-auto flex items-center gap-2">
                <button
                  type="button"
                  onClick={cancel}
                  className="btn-interactive px-2.5 py-1 text-[11.5px] rounded-lg cursor-pointer whitespace-nowrap shrink-0"
                  style={{
                    backgroundColor: 'var(--bg-input)',
                    border: '1px solid var(--border-subtle)',
                    color: 'var(--text-secondary)',
                  }}
                >
                  取消
                </button>
                <button
                  type="button"
                  onClick={commitDraftRange}
                  className="btn-primary px-2.5 py-1 text-[11.5px] rounded-lg cursor-pointer whitespace-nowrap shrink-0"
                  style={{
                    backgroundColor: 'var(--accent-primary)',
                    color: '#ffffff',
                  }}
                >
                  确定
                </button>
              </div>
            </div>
          </div>,
          document.body
        )
      : null;

  return (
    <div
      ref={rootRef}
      data-testid="time-range-control"
      aria-label="时间范围选择"
      className={`seg-group relative ${className}`.trim()}
      style={{
        backgroundColor: 'var(--surface-glass)',
        borderColor: 'var(--border-hairline)',
        boxShadow: 'var(--border-specular)',
      }}
    >
      {TIME_PRESETS.map((preset) => {
        const active = timePreset === preset.id;
        return (
          <button
            key={preset.id}
            type="button"
            data-active={active ? 'true' : undefined}
            aria-expanded={preset.id === 'custom' ? open : undefined}
            onClick={() => {
              if (preset.id === 'custom') {
                if (open) {
                  close();
                } else {
                  openPopover();
                }
              } else {
                close();
                setTimePreset(preset.id);
              }
            }}
            className="seg-pill px-2.5 py-0.5 text-[11.5px] cursor-pointer whitespace-nowrap shrink-0"
            style={{
              color: active ? 'var(--accent-tonal-text)' : 'var(--text-secondary)',
              fontWeight: active ? 500 : 400,
            }}
          >
            {preset.label}
          </button>
        );
      })}
      {popover}
    </div>
  );
}
