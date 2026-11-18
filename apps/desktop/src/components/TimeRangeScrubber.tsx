import { useCallback, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';

export type ScrubberWindow = '24h' | '7d' | '30d';

const HOUR_MS = 3_600_000;
const DAY_MS = 24 * HOUR_MS;

export const SCRUBBER_WINDOWS: Record<ScrubberWindow, { spanMs: number; stepMs: number; label: string }> = {
  '24h': { spanMs: DAY_MS, stepMs: 5 * 60_000, label: '24 小时' },
  '7d': { spanMs: 7 * DAY_MS, stepMs: 15 * 60_000, label: '7 天' },
  '30d': { spanMs: 30 * DAY_MS, stepMs: HOUR_MS, label: '30 天' },
};

/** Picks the smallest window that still contains `startMs`. */
export function pickScrubberWindow(startMs: number, nowMs: number): ScrubberWindow {
  const span = nowMs - startMs;
  if (span <= SCRUBBER_WINDOWS['24h'].spanMs) return '24h';
  if (span <= SCRUBBER_WINDOWS['7d'].spanMs) return '7d';
  return '30d';
}

export function snapToStep(ms: number, stepMs: number): number {
  return Math.round(ms / stepMs) * stepMs;
}

type DragKind = 'start' | 'end' | 'move' | 'create';

interface DragState {
  kind: DragKind;
  pointerId: number;
  originMs: number;
  originStart: number;
  originEnd: number;
}

export interface TimeRangeScrubberProps {
  windowStartMs: number;
  windowEndMs: number;
  stepMs: number;
  startMs: number;
  endMs: number;
  /** Fired continuously while dragging. */
  onChange: (startMs: number, endMs: number) => void;
  /** Fired once when a drag / key interaction settles. */
  onCommit: (startMs: number, endMs: number) => void;
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(Math.max(v, lo), hi);
}

function pad2(n: number): string {
  return String(n).padStart(2, '0');
}

function buildTicks(windowStartMs: number, windowEndMs: number): Array<{ ms: number; label: string }> {
  const span = windowEndMs - windowStartMs;
  const ticks: Array<{ ms: number; label: string }> = [];
  if (span <= DAY_MS + 1) {
    const first = new Date(windowStartMs);
    first.setMinutes(0, 0, 0);
    if (first.getTime() < windowStartMs) {
      first.setHours(first.getHours() + 1);
    }
    while (first.getHours() % 6 !== 0) {
      first.setHours(first.getHours() + 1);
    }
    for (let t = first.getTime(); t < windowEndMs; t += 6 * HOUR_MS) {
      const d = new Date(t);
      ticks.push({ ms: t, label: `${pad2(d.getHours())}:00` });
    }
    return ticks;
  }
  const stepDays = span <= 7 * DAY_MS + 1 ? 1 : 5;
  const first = new Date(windowStartMs);
  first.setHours(0, 0, 0, 0);
  if (first.getTime() < windowStartMs) {
    first.setDate(first.getDate() + 1);
  }
  for (let d = new Date(first); d.getTime() < windowEndMs; d.setDate(d.getDate() + stepDays)) {
    ticks.push({ ms: d.getTime(), label: `${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}` });
  }
  return ticks;
}

/**
 * Continuous two-handle range scrubber: drag either handle, drag the selected
 * band to pan, or drag on the empty track to draw a new range.
 */
export function TimeRangeScrubber({
  windowStartMs,
  windowEndMs,
  stepMs,
  startMs,
  endMs,
  onChange,
  onCommit,
}: TimeRangeScrubberProps) {
  const trackRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const latestRef = useRef<{ start: number; end: number }>({ start: startMs, end: endMs });
  const [dragging, setDragging] = useState<DragKind | null>(null);

  const span = Math.max(1, windowEndMs - windowStartMs);
  const s = clamp(startMs, windowStartMs, windowEndMs);
  const e = clamp(endMs, windowStartMs, windowEndMs);
  const leftPct = ((s - windowStartMs) / span) * 100;
  const rightPct = ((e - windowStartMs) / span) * 100;

  const ticks = useMemo(() => buildTicks(windowStartMs, windowEndMs), [windowStartMs, windowEndMs]);

  const msFromClientX = useCallback(
    (clientX: number): number => {
      const rect = trackRef.current?.getBoundingClientRect();
      if (!rect || rect.width <= 0) {
        return windowStartMs;
      }
      const ratio = clamp((clientX - rect.left) / rect.width, 0, 1);
      return clamp(snapToStep(windowStartMs + ratio * span, stepMs), windowStartMs, windowEndMs);
    },
    [windowStartMs, windowEndMs, span, stepMs]
  );

  const emit = useCallback(
    (nextStart: number, nextEnd: number) => {
      const lo = Math.min(nextStart, nextEnd);
      let hi = Math.max(nextStart, nextEnd);
      if (hi - lo < stepMs) {
        hi = Math.min(windowEndMs, lo + stepMs);
      }
      const finalLo = Math.min(lo, hi - stepMs);
      latestRef.current = { start: finalLo, end: hi };
      onChange(finalLo, hi);
    },
    [onChange, stepMs, windowEndMs]
  );

  const handlePointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement;
    const handle = target.closest<HTMLElement>('[data-scrub]')?.dataset.scrub as DragKind | undefined;
    // A band spanning (nearly) the whole track cannot be panned; draw a new range instead.
    const bandFillsTrack = e - s >= span * 0.9;
    const kind: DragKind = handle === 'move' && bandFillsTrack ? 'create' : (handle ?? 'create');
    const at = msFromClientX(event.clientX);
    dragRef.current = {
      kind,
      pointerId: event.pointerId,
      originMs: at,
      originStart: s,
      originEnd: e,
    };
    latestRef.current = { start: s, end: e };
    setDragging(kind);
    event.currentTarget.setPointerCapture?.(event.pointerId);
    if (kind === 'create') {
      emit(at, at + stepMs);
    }
    event.preventDefault();
  };

  const handlePointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const at = msFromClientX(event.clientX);
    switch (drag.kind) {
      case 'start':
        emit(Math.min(at, drag.originEnd - stepMs), drag.originEnd);
        break;
      case 'end':
        emit(drag.originStart, Math.max(at, drag.originStart + stepMs));
        break;
      case 'move': {
        const width = drag.originEnd - drag.originStart;
        const delta = at - drag.originMs;
        const nextStart = clamp(drag.originStart + delta, windowStartMs, windowEndMs - width);
        emit(nextStart, nextStart + width);
        break;
      }
      case 'create':
        emit(drag.originMs, at);
        break;
    }
  };

  const finishDrag = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setDragging(null);
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    onCommit(latestRef.current.start, latestRef.current.end);
  };

  const handleKey = (kind: 'start' | 'end') => (event: KeyboardEvent<HTMLDivElement>) => {
    const big = event.shiftKey ? 12 : 1;
    let delta = 0;
    if (event.key === 'ArrowLeft' || event.key === 'ArrowDown') delta = -stepMs * big;
    if (event.key === 'ArrowRight' || event.key === 'ArrowUp') delta = stepMs * big;
    if (delta === 0) return;
    event.preventDefault();
    const nextStart = kind === 'start' ? clamp(s + delta, windowStartMs, e - stepMs) : s;
    const nextEnd = kind === 'end' ? clamp(e + delta, s + stepMs, windowEndMs) : e;
    latestRef.current = { start: nextStart, end: nextEnd };
    onChange(nextStart, nextEnd);
    onCommit(nextStart, nextEnd);
  };

  return (
    <div className="select-none">
      <div
        ref={trackRef}
        data-testid="time-range-scrubber"
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={finishDrag}
        onPointerCancel={finishDrag}
        className="relative h-8 rounded-md touch-none"
        style={{
          cursor: dragging === 'move' ? 'grabbing' : 'crosshair',
          backgroundColor: 'var(--bg-input)',
          border: '1px solid var(--border-subtle)',
        }}
      >
        {ticks.map((tick) => (
          <span
            key={tick.ms}
            aria-hidden="true"
            className="absolute top-0 bottom-0 w-px"
            style={{
              left: `${((tick.ms - windowStartMs) / span) * 100}%`,
              backgroundColor: 'var(--border-hairline)',
            }}
          />
        ))}
        <div
          data-scrub="move"
          className="absolute top-0 bottom-0"
          style={{
            left: `${leftPct}%`,
            width: `${Math.max(0.4, rightPct - leftPct)}%`,
            backgroundColor: 'var(--accent-tonal-bg)',
            borderTop: '1px solid var(--accent-primary)',
            borderBottom: '1px solid var(--accent-primary)',
            cursor: dragging === 'move' ? 'grabbing' : 'grab',
            transition: dragging ? 'none' : 'left 160ms ease, width 160ms ease',
          }}
        />
        {(['start', 'end'] as const).map((kind) => {
          const pct = kind === 'start' ? leftPct : rightPct;
          const value = kind === 'start' ? s : e;
          return (
            <div
              key={kind}
              role="slider"
              tabIndex={0}
              data-scrub={kind}
              aria-label={kind === 'start' ? '起始时间' : '结束时间'}
              aria-valuemin={windowStartMs}
              aria-valuemax={windowEndMs}
              aria-valuenow={value}
              aria-valuetext={new Date(value).toLocaleString()}
              onKeyDown={handleKey(kind)}
              className="absolute -top-0.5 -bottom-0.5 w-3 -ml-1.5 flex items-center justify-center rounded-sm outline-none focus-visible:ring-2"
              style={{
                left: `${pct}%`,
                cursor: 'ew-resize',
                transition: dragging ? 'none' : 'left 160ms ease',
              }}
            >
              <span
                className="block h-full w-1 rounded-full"
                style={{ backgroundColor: 'var(--accent-primary)' }}
              />
            </div>
          );
        })}
      </div>
      <div className="relative h-4 mt-1 text-[10px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
        {ticks.map((tick) => (
          <span
            key={tick.ms}
            className="absolute -translate-x-1/2 whitespace-nowrap"
            style={{ left: `${((tick.ms - windowStartMs) / span) * 100}%` }}
          >
            {tick.label}
          </span>
        ))}
      </div>
    </div>
  );
}
