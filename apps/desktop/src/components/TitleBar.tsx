import { useContext, useEffect, useMemo, useState } from 'react';
import { QueryClient, QueryClientContext, useQuery } from '@tanstack/react-query';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  Minus,
  Square,
  X,
  RefreshCw,
  Sun,
  Moon,
  Monitor,
  PictureInPicture2,
  Download,
} from 'lucide-react';
import { ipc, isTauriRuntime } from '../lib/ipc';
import { downloadAndInstallUpdate, useUpdaterStore } from '../lib/updater';
import { useWorkspaceStore, type NavPage } from '../stores/workspaceStore';
import type { CurrencyCode, ThemeMode } from '../types/ipc';

export interface TitleBarProps {
  collectorWatching: boolean;
  isScanning: boolean;
  livePulseSeq?: number;
  hasBackdrop?: boolean;
  floatingEnabled?: boolean;
  onScanNow: () => void;
  onPersistCurrency?: (currency: CurrencyCode) => void;
  onPersistTheme?: (theme: ThemeMode) => void;
}

const PAGE_TITLES: Record<NavPage, string> = {
  overview: '概览',
  trends: '趋势分析',
  monthly: '月度统计',
  models: '模型与价格',
  subscriptions: '用量',
  speed: '速度监测',
  sources: '数据源与合规',
  preferences: '偏好设置',
};

const BUILTIN_PROVIDER_FILTERS = [
  { id: 'all', label: '全部源' },
  { id: 'claude_code', label: 'Claude Code' },
  { id: 'codex', label: 'Codex' },
  { id: 'antigravity', label: 'Antigravity' },
  { id: 'kimi_code', label: 'Kimi Code' },
  { id: 'workbuddy', label: 'Workbuddy' },
  { id: 'deepseek_harness', label: 'DeepSeek Harness' },
  { id: 'qcode', label: 'QCode' },
  { id: 'minimax_code', label: 'MiniMax Code' },
];

const THEME_LABELS: Record<ThemeMode, string> = {
  dark: '深色',
  light: '浅色',
  system: '跟随系统',
};

export function TitleBar({
  collectorWatching,
  isScanning,
  livePulseSeq = 0,
  hasBackdrop = false,
  floatingEnabled,
  onScanNow,
  onPersistCurrency,
  onPersistTheme,
}: TitleBarProps) {
  const {
    activePage,
    providerFilter,
    deviceFilter,
    currency,
    theme,
    setProviderFilter,
    setDeviceFilter,
    setCurrency,
    setTheme,
  } = useWorkspaceStore();

  const parentQueryClient = useContext(QueryClientContext);
  const fallbackQueryClient = useMemo(
    () => new QueryClient({ defaultOptions: { queries: { retry: false } } }),
    []
  );
  const queryClient = parentQueryClient ?? fallbackQueryClient;

  const { data: customRules = [] } = useQuery(
    {
      queryKey: ['custom-provider-rules'],
      queryFn: () => ipc.listCustomProviderRules(),
      staleTime: 30_000,
    },
    queryClient
  );

  const { data: devices = [] } = useQuery(
    {
      queryKey: ['devices'],
      queryFn: () => ipc.listDevices(),
      staleTime: 30_000,
    },
    queryClient
  );

  const { data: queriedFloatingVisible } = useQuery(
    {
      queryKey: ['floating-visible'],
      queryFn: () => ipc.getFloatingWindowVisible(),
      staleTime: 5_000,
    },
    queryClient
  );
  const isFloatingActive = floatingEnabled ?? queriedFloatingVisible ?? false;

  const handleToggleFloating = async () => {
    const next = await ipc.toggleFloatingWindow();
    queryClient.setQueryData(['floating-visible'], next);
    void queryClient.invalidateQueries({ queryKey: ['preferences'] });
  };

  const safeDevices = useMemo(() => (Array.isArray(devices) ? devices : []), [devices]);

  useEffect(() => {
    if (
      deviceFilter !== 'all' &&
      safeDevices.length > 0 &&
      !safeDevices.some((dev) => dev.device_id === deviceFilter)
    ) {
      setDeviceFilter('all');
    }
  }, [deviceFilter, safeDevices, setDeviceFilter]);

  const providerOptions = useMemo(() => {
    const safeRules = Array.isArray(customRules) ? customRules : [];
    const customEntries = safeRules
      .filter((rule) => rule.enabled)
      .map((rule) => {
        const overrideId = rule.mapping_spec.provider_id_override?.trim();
        const id = overrideId
          ? overrideId
          : rule.rule_id.startsWith('custom_')
            ? rule.rule_id
            : `custom_${rule.rule_id}`;
        return { id, label: rule.display_name };
      });
    return [...BUILTIN_PROVIDER_FILTERS, ...customEntries];
  }, [customRules]);

  const [livePulseActive, setLivePulseActive] = useState(false);
  const updater = useUpdaterStore();
  const updateDownloadPercent =
    updater.totalBytes && updater.totalBytes > 0
      ? Math.min(100, (updater.downloadedBytes / updater.totalBytes) * 100)
      : updater.status === 'installing'
        ? 100
        : 0;

  useEffect(() => {
    if (livePulseSeq <= 0) {
      return;
    }
    setLivePulseActive(true);
    const timer = window.setTimeout(() => {
      setLivePulseActive(false);
    }, 2400);
    return () => {
      window.clearTimeout(timer);
    };
  }, [livePulseSeq]);

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
    if (onPersistTheme) {
      onPersistTheme(next);
    } else {
      setTheme(next);
    }
  };

  const handleCurrencyToggle = (next: CurrencyCode) => {
    if (onPersistCurrency) {
      onPersistCurrency(next);
    } else {
      setCurrency(next);
    }
  };

  const borderToken = hasBackdrop ? 'var(--border-hairline)' : 'var(--border-subtle)';
  const controlBgToken = hasBackdrop ? 'var(--bg-input)' : 'var(--bg-canvas)';

  const dotClass = livePulseActive
    ? 'live-pulse-dot'
    : collectorWatching
      ? 'collector-dot-live'
      : '';
  const dotColor =
    livePulseActive || collectorWatching ? 'var(--accent-emerald)' : 'var(--accent-amber)';
  const dotTitle = livePulseActive
    ? '刚接收实时用量更新'
    : collectorWatching
      ? '正在实时只读监听本地日志'
      : '增量扫描就绪';

  return (
    <header
      className="liquid-chrome relative z-20 flex items-center justify-between px-3 shrink-0 select-none"
      style={{
        height: '40px',
        backgroundColor: hasBackdrop ? 'var(--bg-translucent-chrome)' : 'var(--surface-glass-chrome)',
        borderBottom: `1px solid ${borderToken}`,
      }}
    >
      {/* Left: Brand & Active Section Title (Draggable) */}
      <div
        data-tauri-drag-region
        className="flex items-center gap-2 h-full min-w-[196px]"
      >
        <span
          className="inline-flex items-center justify-center w-5 h-5 rounded-lg text-[10.5px] font-semibold tracking-tight"
          style={{
            backgroundColor: 'var(--accent-tonal-bg)',
            boxShadow: 'var(--border-specular)',
            border: '1px solid var(--border-hairline)',
            color: 'var(--accent-tonal-text)',
          }}
        >
          TT
        </span>
        <span
          className="text-[12.5px] font-semibold tracking-tight"
          style={{ color: 'var(--text-primary)' }}
        >
          TokenVane
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
        {/* Multi-Device Quick Filter (only when >1 devices synced) */}
        {safeDevices.length > 1 && (
          <select
            aria-label="设备筛选"
            value={deviceFilter}
            onChange={(e) => setDeviceFilter(e.target.value)}
            className="px-2 py-1 text-[11.5px] rounded-lg outline-none cursor-pointer"
            style={{
              backgroundColor: controlBgToken,
              border: `1px solid ${borderToken}`,
              color: 'var(--text-primary)',
            }}
          >
            <option value="all">全部设备</option>
            {safeDevices.map((dev, index) => (
              <option key={dev.device_id} value={dev.device_id}>
                {index === 0 ? `本机 · ${dev.device_name}` : dev.device_name}
              </option>
            ))}
          </select>
        )}

        {/* Provider Quick Filter */}
        <select
          aria-label="模型提供商筛选"
          value={providerFilter}
          onChange={(e) => setProviderFilter(e.target.value)}
          className="px-2 py-1 text-[11.5px] rounded-lg outline-none cursor-pointer"
          style={{
            backgroundColor: controlBgToken,
            border: `1px solid ${borderToken}`,
            color: 'var(--text-primary)',
          }}
        >
          {providerOptions.map((pf) => (
            <option key={pf.id} value={pf.id}>
              {pf.label}
            </option>
          ))}
        </select>

        {/* Currency Switcher */}
        <div
          className="seg-group"
          style={{
            backgroundColor: controlBgToken,
            borderColor: borderToken,
            boxShadow: 'var(--border-specular)',
          }}
        >
          {(['USD', 'CNY'] as const).map((code) => {
            const active = currency === code;
            return (
              <button
                key={code}
                type="button"
                data-active={active ? 'true' : undefined}
                onClick={() => handleCurrencyToggle(code)}
                className="seg-pill px-2.5 py-0.5 text-[11px] font-medium tabular-nums cursor-pointer"
                style={{
                  color: active ? 'var(--accent-tonal-text)' : 'var(--text-secondary)',
                }}
              >
                {code}
              </button>
            );
          })}
        </div>

        {/* Collector Live Status Dot + Manual Refresh + Floating Toggle + Theme Cycle + Update Button */}
        <div className="flex items-center gap-1.5 pl-1 pr-1.5">
          {updater.availableUpdate && (
            <button
              type="button"
              aria-label="立即更新客户端"
              title={`发现新版本 v${updater.availableUpdate.version}，点击立即更新`}
              disabled={updater.status === 'downloading' || updater.status === 'installing'}
              onClick={() => void downloadAndInstallUpdate()}
              className="btn-interactive inline-flex items-center gap-1 px-2 py-0.5 rounded-lg text-[11px] font-medium tabular-nums cursor-pointer disabled:opacity-60"
              style={{
                backgroundColor: 'rgba(16, 185, 129, 0.14)',
                border: '1px solid var(--accent-emerald)',
                color: 'var(--accent-emerald)',
              }}
            >
              <Download className="w-3 h-3" />
              <span>
                {updater.status === 'downloading'
                  ? `下载中 ${updateDownloadPercent.toFixed(0)}%`
                  : updater.status === 'installing'
                    ? '安装中…'
                    : `更新 v${updater.availableUpdate.version}`}
              </span>
            </button>
          )}
          <span
            data-testid="titlebar-live-dot"
            title={dotTitle}
            className={`inline-block w-2 h-2 rounded-full ${dotClass}`}
            style={{ backgroundColor: dotColor }}
          />
          <button
            type="button"
            aria-label="立即增量扫描"
            title="立即增量扫描"
            onClick={onScanNow}
            className="btn-interactive p-1 rounded-lg cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isScanning ? 'animate-spin' : ''}`} />
          </button>
          <button
            type="button"
            aria-label="切换桌面悬浮窗"
            aria-pressed={isFloatingActive}
            data-active={isFloatingActive ? 'true' : undefined}
            title={isFloatingActive ? '隐藏实时桌面悬浮窗' : '开启实时桌面悬浮窗'}
            onClick={() => void handleToggleFloating()}
            className="btn-interactive p-1 rounded-lg cursor-pointer"
            style={{
              backgroundColor: isFloatingActive ? 'var(--accent-tonal-bg)' : undefined,
              color: isFloatingActive ? 'var(--accent-tonal-text)' : 'var(--text-secondary)',
            }}
          >
            <PictureInPicture2 className="w-3.5 h-3.5" />
          </button>
          <button
            type="button"
            aria-label="切换主题"
            title={`切换主题（当前: ${THEME_LABELS[theme]}）`}
            onClick={cycleTheme}
            className="btn-interactive p-1 rounded-lg cursor-pointer"
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
          style={{ backgroundColor: borderToken }}
        />

        <div className="flex items-center">
          <button
            type="button"
            aria-label="最小化"
            onClick={handleMinimize}
            className="btn-interactive inline-flex items-center justify-center w-7 h-7 rounded-lg cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <Minus className="w-3.5 h-3.5" />
          </button>
          <button
            type="button"
            aria-label="最大化或还原"
            onClick={handleToggleMaximize}
            className="btn-interactive inline-flex items-center justify-center w-7 h-7 rounded-lg cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <Square className="w-3 h-3" />
          </button>
          <button
            type="button"
            aria-label="关闭窗口"
            onClick={handleClose}
            className="inline-flex items-center justify-center w-7 h-7 rounded-lg text-[var(--text-secondary)] hover:bg-rose-500/85 hover:text-white transition-colors cursor-pointer"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </header>
  );
}
