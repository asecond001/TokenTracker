import { useState } from 'react';
import { useWorkspaceStore, type TimePreset } from '../stores/workspaceStore';

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

  return (
    <div
      data-testid="time-range-control"
      aria-label="时间范围选择"
      className={`relative inline-flex items-center p-0.5 rounded ${className}`.trim()}
      style={{
        backgroundColor: 'var(--bg-sidebar)',
        border: '1px solid var(--border-hairline)',
      }}
    >
      {TIME_PRESETS.map((preset) => {
        const active = timePreset === preset.id;
        return (
          <button
            key={preset.id}
            type="button"
            data-active={active ? 'true' : undefined}
            onClick={() => {
              if (preset.id === 'custom') {
                setShowCustomPopover((v) => !v);
                setTimePreset('custom');
              } else {
                setShowCustomPopover(false);
                setTimePreset(preset.id);
              }
            }}
            className="seg-pill px-2 py-0.5 text-[11.5px] rounded-sm cursor-pointer whitespace-nowrap"
            style={{
              color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
              fontWeight: active ? 500 : 400,
            }}
          >
            {preset.label}
          </button>
        );
      })}

      {showCustomPopover && (
        <div
          className="absolute right-0 top-8 z-50 flex items-center gap-2 p-2.5 rounded shadow-lg"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            border: '1px solid var(--border-strong)',
          }}
        >
          <input
            type="date"
            aria-label="开始日期"
            value={customStartDate}
            onChange={(e) => setCustomDateRange(e.target.value, customEndDate)}
            className="px-2 py-1 text-[11.5px] rounded tabular-nums"
            style={{
              backgroundColor: 'var(--bg-input)',
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-primary)',
            }}
          />
          <span style={{ color: 'var(--text-muted)' }}>至</span>
          <input
            type="date"
            aria-label="结束日期"
            value={customEndDate}
            onChange={(e) => setCustomDateRange(customStartDate, e.target.value)}
            className="px-2 py-1 text-[11.5px] rounded tabular-nums"
            style={{
              backgroundColor: 'var(--bg-input)',
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-primary)',
            }}
          />
          <button
            type="button"
            onClick={() => setShowCustomPopover(false)}
            className="btn-interactive px-2 py-1 text-[11.5px] rounded cursor-pointer"
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
