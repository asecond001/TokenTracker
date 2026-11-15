import { useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  Minus,
  Square,
  X,
  RefreshCw,
  Sun,
  Moon,
  Monitor,
} from 'lucide-react';
import { isTauriRuntime } from '../lib/ipc';
import { useWorkspaceStore, type NavPage, type TimePreset } from '../stores/workspaceStore';
import type { CurrencyCode, ThemeMode } from '../types/ipc';

interface TitleBarProps {
  collectorWatching: boolean;
  isScanning: boolean;
  onScanNow: () => void;
  onPersistCurrency?: (currency: CurrencyCode) => void;
  onPersistTheme?: (theme: ThemeMode) => void;
}

const PAGE_TITLES: Record<NavPage, string> = {
  overview: '概览',
  trends: '趋势分析',
  monthly: '月度统计',
  models: '模型与价格',
  subscriptions: '订阅用量',
  speed: '速度监测',
  sources: '数据源与合规',
  preferences: '偏好设置',
};

const TIME_PRESETS: Array<{ id: TimePreset; label: string }> = [
  { id: 'today', label: '今天' },
  { id: '7d', label: '7 天' },
  { id: '30d', label: '30 天' },
  { id: 'month', label: '本月' },
  { id: 'custom', label: '自定义' },
];

const PROVIDER_FILTERS = [
  { id: 'all', label: '全部源' },
  { id: 'claude_code', label: 'Claude Code' },
  { id: 'codex', label: 'Codex' },
  { id: 'antigravity', label: 'Antigravity' },
];

export function TitleBar({
  collectorWatching,
  isScanning,
  onScanNow,
  onPersistCurrency,
  onPersistTheme,
}: TitleBarProps) {
  const {
    activePage,
    timePreset,
    customStartDate,
    customEndDate,
    providerFilter,
    currency,
    theme,
    setTimePreset,
    setCustomDateRange,
    setProviderFilter,
    setCurrency,
    setTheme,
  } = useWorkspaceStore();

  const [showCustomPopover, setShowCustomPopover] = useState(false);

  const handleMinimize = async () => {
    if (isTauriRuntime()) {
      await getCurrentWindow().minimize();
    }
  };

  const handleToggleMaximize = async () => {
    if (isTauriRuntime()) {
      await getCurrentWindow().toggleMaximize();
    }
  };

  const handleClose = async () => {
    if (isTauriRuntime()) {
      await getCurrentWindow().close();
    }
  };

  const cycleTheme = () => {
    const order: ThemeMode[] = ['dark', 'light', 'system'];
    const idx = order.indexOf(theme);
    const next = order[(idx + 1) % order.length];
    setTheme(next);
    onPersistTheme?.(next);
  };

  const handleCurrencyToggle = (next: CurrencyCode) => {
    setCurrency(next);
    onPersistCurrency?.(next);
  };

  return (
    <header
      className="relative flex items-center justify-between px-3 shrink-0 select-none"
      style={{
        height: '38px',
        backgroundColor: 'var(--bg-sidebar)',
        borderBottom: '1px solid var(--border-subtle)',
      }}
    >
      {/* Left: Brand & Active Section Title (Draggable) */}
      <div
        data-tauri-drag-region
        className="flex items-center gap-2 h-full min-w-[196px]"
      >
        <span
          className="inline-flex items-center justify-center w-5 h-5 rounded text-[11px] font-semibold tracking-tight"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            border: '1px solid var(--border-strong)',
            color: 'var(--text-primary)',
          }}
        >
          TT
        </span>
        <span className="text-[12.5px] font-semibold tracking-tight" style={{ color: 'var(--text-primary)' }}>
          TokenTracker
        </span>
        <span style={{ color: 'var(--text-muted)' }}>/</span>
        <span className="text-[12.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
          {PAGE_TITLES[activePage]}
        </span>
      </div>

      {/* Draggable Spacer */}
      <div data-tauri-drag-region className="flex-1 h-full min-w-[16px]" />

      {/* Center/Right Controls */}
      <div className="flex items-center gap-2">
        {/* Global Time Range Segmented Control */}
        <div
          className="relative flex items-center p-0.5 rounded"
          style={{
            backgroundColor: 'var(--bg-canvas)',
            border: '1px solid var(--border-subtle)',
          }}
        >
          {TIME_PRESETS.map((preset) => {
            const active = timePreset === preset.id;
            return (
              <button
                key={preset.id}
                type="button"
                onClick={() => {
                  if (preset.id === 'custom') {
                    setShowCustomPopover((v) => !v);
                    setTimePreset('custom');
                  } else {
                    setShowCustomPopover(false);
                    setTimePreset(preset.id);
                  }
                }}
                className="px-2 py-0.5 text-[11.5px] rounded-sm transition-colors cursor-pointer"
                style={{
                  backgroundColor: active ? 'var(--bg-hover)' : 'transparent',
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
                className="px-2 py-1 text-[11.5px] rounded cursor-pointer"
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

        {/* Provider Quick Filter */}
        <select
          aria-label="Provider 筛选"
          value={providerFilter}
          onChange={(e) => setProviderFilter(e.target.value)}
          className="px-2 py-1 text-[11.5px] rounded outline-none cursor-pointer"
          style={{
            backgroundColor: 'var(--bg-canvas)',
            border: '1px solid var(--border-subtle)',
            color: 'var(--text-primary)',
          }}
        >
          {PROVIDER_FILTERS.map((pf) => (
            <option key={pf.id} value={pf.id}>
              {pf.label}
            </option>
          ))}
        </select>

        {/* Currency Switcher */}
        <div
          className="flex items-center p-0.5 rounded"
          style={{
            backgroundColor: 'var(--bg-canvas)',
            border: '1px solid var(--border-subtle)',
          }}
        >
          {(['USD', 'CNY'] as const).map((code) => {
            const active = currency === code;
            return (
              <button
                key={code}
                type="button"
                onClick={() => handleCurrencyToggle(code)}
                className="px-2 py-0.5 text-[11px] font-medium rounded-sm tabular-nums cursor-pointer"
                style={{
                  backgroundColor: active ? 'var(--bg-hover)' : 'transparent',
                  color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                }}
              >
                {code}
              </button>
            );
          })}
        </div>

        {/* Collector Live Status Dot + Manual Refresh */}
        <div className="flex items-center gap-1.5 pl-1 pr-1.5">
          <span
            title={collectorWatching ? '正在实时只读监听本地日志' : '增量扫描就绪'}
            className={`inline-block w-2 h-2 rounded-full ${collectorWatching ? 'collector-dot-live' : ''}`}
            style={{
              backgroundColor: collectorWatching
                ? 'var(--accent-emerald)'
                : 'var(--accent-amber)',
            }}
          />
          <button
            type="button"
            title="立即增量扫描"
            onClick={onScanNow}
            className="p-1 rounded hover:opacity-80 cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isScanning ? 'animate-spin' : ''}`} />
          </button>
          <button
            type="button"
            title={`切换主题（当前: ${theme}）`}
            onClick={cycleTheme}
            className="p-1 rounded hover:opacity-80 cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            {theme === 'light' ? (
              <Sun className="w-3.5 h-3.5" />
            ) : theme === 'dark' ? (
              <Moon className="w-3.5 h-3.5" />
            ) : (
              <Monitor className="w-3.5 h-3.5" />
            )}
          </button>
        </div>

        {/* Hairline Divider & Window Controls */}
        <div
          className="h-4 w-px mx-0.5"
          style={{ backgroundColor: 'var(--border-subtle)' }}
        />

        <div className="flex items-center">
          <button
            type="button"
            aria-label="最小化"
            onClick={handleMinimize}
            className="inline-flex items-center justify-center w-7 h-7 rounded hover:bg-white/5 cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <Minus className="w-3.5 h-3.5" />
          </button>
          <button
            type="button"
            aria-label="最大化或还原"
            onClick={handleToggleMaximize}
            className="inline-flex items-center justify-center w-7 h-7 rounded hover:bg-white/5 cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <Square className="w-3 h-3" />
          </button>
          <button
            type="button"
            aria-label="关闭窗口"
            onClick={handleClose}
            className="inline-flex items-center justify-center w-7 h-7 rounded hover:bg-rose-500/80 hover:text-white cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </header>
  );
}
