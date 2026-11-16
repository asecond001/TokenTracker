import { useEffect, useRef, useState } from 'react';
import {
  getPresetDateInputs,
  useWorkspaceStore,
  type TimePreset,
} from '../stores/workspaceStore';

const TIME_PRESETS: Array<{ id: TimePreset; label: string }> = [
  { id: 'today', label: '今天' },
  { id: '7d', label: '7 天' },
  { id: '30d', label: '30 天' },
  { id: 'month', label: '本月' },
  { id: 'all', label: '全部' },
  { id: 'custom', label: '自定义' },
];

export interface TimeRangeControlProps {
  className?: string;
}

export function TimeRangeControl({ className = '' }: TimeRangeControlProps) {
  const {
    timePreset,
    customStartDate,
    customEndDate,
    setTimePreset,
    setCustomDateRange,
  } = useWorkspaceStore();

  const [showCustomPopover, setShowCustomPopover] = useState(false);
  const [draftStart, setDraftStart] = useState(customStartDate);
  const [draftEnd, setDraftEnd] = useState(customEndDate);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!showCustomPopover) {
      return;
    }
    const handlePointerDown = (event: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setShowCustomPopover(false);
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setShowCustomPopover(false);
      }
    };
    document.addEventListener('mousedown', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [showCustomPopover]);

  const openOrToggleCustomPopover = () => {
    if (showCustomPopover) {
      setShowCustomPopover(false);
      return;
    }
    const initial = getPresetDateInputs(timePreset, {
      start: customStartDate,
      end: customEndDate,
    });
    setDraftStart(initial.start);
    setDraftEnd(initial.end);
    setShowCustomPopover(true);
  };

  const commitDraftRange = () => {
    const start = draftStart || customStartDate;
    const end = draftEnd || customEndDate;
    if (start && end && start > end) {
      setCustomDateRange(end, start);
    } else {
      setCustomDateRange(start, end);
    }
    setShowCustomPopover(false);
  };

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
            aria-expanded={preset.id === 'custom' ? showCustomPopover : undefined}
            onClick={() => {
              if (preset.id === 'custom') {
                openOrToggleCustomPopover();
              } else {
                setShowCustomPopover(false);
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

      {showCustomPopover && (
        <div
          role="dialog"
          aria-label="自定义日期范围"
          className="liquid-popover absolute right-0 top-9 z-50 flex items-center gap-2 p-2.5 min-w-max"
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
          <button
            type="button"
            onClick={() => setShowCustomPopover(false)}
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
      )}
    </div>
  );
}
