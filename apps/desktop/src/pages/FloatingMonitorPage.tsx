import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ExternalLink, Maximize2, Minimize2, SlidersHorizontal, X } from 'lucide-react';
import { AnimatedNumber } from '../components/AnimatedNumber';
import { InlineProgressBar } from '../components/InlineProgressBar';
import {
  computeFloatingDimensions,
  useFloatingConfig,
  type FloatingSizePreset,
  type FloatingWindowConfig,
} from '../lib/floatingConfig';
import {
  formatCost,
  formatCountdown,
  formatSessionDisplayName,
  formatTokens,
  formatTps,
  providerDisplayName,
} from '../lib/format';
import { ipc, isTauriRuntime } from '../lib/ipc';
import { syncDocumentAndNativeTheme, useWorkspaceStore } from '../stores/workspaceStore';
import type { ThemeMode } from '../types/ipc';

function normalizeThemeMode(raw: string | undefined | null): ThemeMode {
  return raw === 'light' || raw === 'dark' || raw === 'system' ? raw : 'dark';
}

export interface FloatingMonitorPageProps {
  livePulseSeq?: number;
}

const SIZE_PRESETS: ReadonlyArray<{ id: FloatingSizePreset; label: string }> = [
  { id: 'compact', label: '胶囊' },
  { id: 'sm', label: '小' },
  { id: 'md', label: '标准' },
  { id: 'lg', label: '大' },
];

const ELEMENT_TOGGLES: ReadonlyArray<{
  key: keyof Pick<
    FloatingWindowConfig,
    'showTokens' | 'showCost' | 'showSpeed' | 'showSession' | 'showSubscriptions'
  >;
  label: string;
}> = [
  { key: 'showTokens', label: 'Token 用量' },
  { key: 'showCost', label: '等效费用' },
  { key: 'showSpeed', label: '生成速率' },
  { key: 'showSession', label: '活跃会话' },
  { key: 'showSubscriptions', label: '订阅配额' },
];

export function FloatingMonitorPage({ livePulseSeq = 0 }: FloatingMonitorPageProps) {
  const [floatingConfig, updateFloatingConfig] = useFloatingConfig();
  const [customizing, setCustomizing] = useState(false);
  const [recentDeltaTokens, setRecentDeltaTokens] = useState<number>(0);
  const [burstKey, setBurstKey] = useState(0);
  const prevTodayTokensRef = useRef<number | null>(null);
  const nonCompactPresetRef = useRef<Exclude<FloatingSizePreset, 'compact'>>(
    floatingConfig.sizePreset === 'compact' ? 'md' : floatingConfig.sizePreset
  );

  const {
    tzOffsetMinutes,
    currency: storeCurrency,
    usdToCnyRate: storeRate,
    resolvedTheme,
    setTheme,
    setCurrency,
    setUsdToCnyRate,
  } = useWorkspaceStore();

  useEffect(() => {
    if (typeof document === 'undefined') return;
    document.documentElement.dataset.floatingView = 'true';
    return () => {
      delete document.documentElement.dataset.floatingView;
    };
  }, []);

  const { data: prefs } = useQuery({
    queryKey: ['preferences'],
    queryFn: () => ipc.getPreferences(),
    refetchInterval: 5_000,
  });

  useEffect(() => {
    if (!prefs) return;
    const nextTheme = normalizeThemeMode(prefs.theme);
    setTheme(nextTheme);
    syncDocumentAndNativeTheme(nextTheme);
    setCurrency(prefs.currency);
    setUsdToCnyRate(prefs.usd_to_cny_rate);
  }, [prefs, setTheme, setCurrency, setUsdToCnyRate]);

  const { data: summary } = useQuery({
    queryKey: ['tray_summary', tzOffsetMinutes],
    queryFn: () => ipc.getTraySummary(tzOffsetMinutes),
    refetchInterval: 3_000,
  });

  const todayTokens = summary?.today_tokens ?? 0;
  useEffect(() => {
    if (prevTodayTokensRef.current === null) {
      prevTodayTokensRef.current = todayTokens;
      return;
    }
    const delta = todayTokens - prevTodayTokensRef.current;
    prevTodayTokensRef.current = todayTokens;
    if (delta > 0) {
      setRecentDeltaTokens(delta);
      setBurstKey((k) => k + 1);
    }
  }, [todayTokens, livePulseSeq]);

  useEffect(() => {
    if (recentDeltaTokens <= 0) return;
    const timer = setTimeout(() => setRecentDeltaTokens(0), 2_600);
    return () => clearTimeout(timer);
  }, [recentDeltaTokens, burstKey]);

  const compact = floatingConfig.sizePreset === 'compact';
  if (floatingConfig.sizePreset !== 'compact') {
    nonCompactPresetRef.current = floatingConfig.sizePreset;
  }

  const handleToggleCompact = () => {
    const nextCompact = !compact;
    const nextPreset: FloatingSizePreset = nextCompact ? 'compact' : nonCompactPresetRef.current;
    setCustomizing(false);
    updateFloatingConfig({ sizePreset: nextPreset });
    void ipc.setFloatingWindowCompact(nextCompact);
  };

  const handleSelectSizePreset = (preset: FloatingSizePreset) => {
    const nextCfg = updateFloatingConfig({ sizePreset: preset });
    if (preset === 'compact') {
      setCustomizing(false);
      void ipc.setFloatingWindowCompact(true);
    } else {
      const dims = computeFloatingDimensions(nextCfg);
      void ipc.setFloatingWindowCompact(false, dims.width, dims.height);
    }
  };

  const handleToggleElement = (
    key: keyof Pick<
      FloatingWindowConfig,
      'showTokens' | 'showCost' | 'showSpeed' | 'showSession' | 'showSubscriptions'
    >
  ) => {
    const nextCfg = updateFloatingConfig({ [key]: !floatingConfig[key] });
    if (nextCfg.sizePreset !== 'compact') {
      const dims = computeFloatingDimensions(nextCfg);
      void ipc.setFloatingWindowCompact(false, dims.width, dims.height);
    }
  };

  const handleClose = () => {
    void ipc.toggleFloatingWindow();
  };

  const handleDragStart = (e: React.MouseEvent) => {
    if (e.button !== 0 || !isTauriRuntime()) return;
    void getCurrentWindow().startDragging();
  };

  const handleResizeDragStart = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (e.button !== 0 || !isTauriRuntime()) return;
    const win = getCurrentWindow() as unknown as {
      startResizeDragging?: (direction: string) => Promise<void>;
    };
    if (typeof win.startResizeDragging === 'function') {
      void win.startResizeDragging('SouthEast');
    }
  };

  const currency = prefs?.currency ?? summary?.currency ?? storeCurrency;
  const rate = prefs?.usd_to_cny_rate ?? summary?.usd_to_cny_rate ?? storeRate;
  const activeSession = summary?.active_sessions?.[0] ?? null;
  const topSubs = (summary?.subscriptions ?? []).slice(0, 2);
  const effectiveTps = activeSession?.latest_output_tps ?? summary?.latest_output_tps ?? null;

  const isLight = resolvedTheme === 'light';
  const alpha = floatingConfig.opacity;
  const surfaceBg = isLight
    ? `rgba(250, 250, 252, ${alpha})`
    : `rgba(17, 19, 24, ${alpha})`;
  const borderColor = isLight
    ? 'rgba(17, 24, 39, 0.14)'
    : 'rgba(255, 255, 255, 0.12)';

  if (compact) {
    return (
      <div
        data-theme={resolvedTheme}
        className="h-screen w-screen flex items-center justify-between gap-2 px-3 select-none overflow-hidden"
        style={{
          backgroundColor: surfaceBg,
          backdropFilter: 'blur(18px) saturate(140%)',
          WebkitBackdropFilter: 'blur(18px) saturate(140%)',
          color: 'var(--text-primary)',
          border: `1px solid ${borderColor}`,
        }}
      >
        <div
          data-tauri-drag-region
          onMouseDown={handleDragStart}
          className="flex items-center gap-2 min-w-0 flex-1 h-full cursor-move"
          title="拖动悬浮窗"
        >
          <span
            className={`inline-block w-2 h-2 rounded-full shrink-0 ${
              recentDeltaTokens > 0 ? 'live-pulse-dot' : 'collector-dot-live'
            }`}
            style={{ backgroundColor: 'var(--accent-emerald)' }}
          />
          {floatingConfig.showTokens && (
            <span className="text-[13px] font-semibold tabular-nums truncate">
              <AnimatedNumber value={todayTokens} formatter={formatTokens} />
            </span>
          )}
          {floatingConfig.showCost && (
            <span
              className="text-[11px] tabular-nums shrink-0"
              style={{ color: 'var(--text-secondary)' }}
            >
              <AnimatedNumber
                value={summary?.today_cost_micro_usd ?? 0}
                formatter={(v) => formatCost(v, currency, rate)}
              />
            </span>
          )}
          {floatingConfig.showSpeed && (
            <span
              className="text-[10.5px] tabular-nums shrink-0"
              style={{ color: 'var(--accent-sky)' }}
            >
              {formatTps(effectiveTps)}
            </span>
          )}
          {recentDeltaTokens > 0 && (
            <span
              key={burstKey}
              className="hud-burst-badge text-[10px] font-medium tabular-nums shrink-0"
              style={{ color: 'var(--accent-emerald)' }}
            >
              +{formatTokens(recentDeltaTokens)}
            </span>
          )}
        </div>
        <div className="flex items-center gap-0.5 shrink-0">
          <button
            type="button"
            aria-label="展开悬浮窗"
            title="展开悬浮窗"
            onClick={handleToggleCompact}
            className="btn-interactive p-1 rounded cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <Maximize2 className="w-3 h-3" />
          </button>
          <button
            type="button"
            aria-label="关闭悬浮窗"
            title="关闭悬浮窗"
            onClick={handleClose}
            className="btn-interactive p-1 rounded cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      </div>
    );
  }

  const tokenFontClass =
    floatingConfig.sizePreset === 'lg'
      ? 'text-[23px]'
      : floatingConfig.sizePreset === 'sm'
        ? 'text-[17px]'
        : 'text-[20px]';

  return (
    <div
      data-theme={resolvedTheme}
      className="relative h-screen w-screen flex flex-col justify-between p-3 select-none overflow-hidden"
      style={{
        backgroundColor: surfaceBg,
        backdropFilter: 'blur(18px) saturate(140%)',
        WebkitBackdropFilter: 'blur(18px) saturate(140%)',
        color: 'var(--text-primary)',
        border: `1px solid ${borderColor}`,
      }}
    >
      {/* Top Drag Header */}
      <div
        className="flex items-center justify-between pb-1.5 shrink-0"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div
          data-tauri-drag-region
          onMouseDown={handleDragStart}
          className="flex items-center gap-1.5 flex-1 min-w-0 cursor-move"
          title="拖动悬浮窗"
        >
          <span
            className={`inline-block w-2 h-2 rounded-full shrink-0 ${
              recentDeltaTokens > 0 ? 'live-pulse-dot' : 'collector-dot-live'
            }`}
            style={{ backgroundColor: 'var(--accent-emerald)' }}
          />
          <span className="text-[11.5px] font-semibold tracking-tight truncate">
            TokenTracker
          </span>
        </div>
        <div className="flex items-center gap-0.5 shrink-0">
          <button
            type="button"
            aria-label="自定义悬浮窗"
            title="自定义悬浮窗元素与尺寸"
            data-active={customizing ? 'true' : undefined}
            onClick={() => setCustomizing((prev) => !prev)}
            className="btn-interactive p-1 rounded cursor-pointer"
            style={{
              color: customizing ? 'var(--accent-primary)' : 'var(--text-secondary)',
            }}
          >
            <SlidersHorizontal className="w-3 h-3" />
          </button>
          <button
            type="button"
            aria-label="切换精简胶囊模式"
            title="切换精简胶囊模式"
            onClick={handleToggleCompact}
            className="btn-interactive p-1 rounded cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <Minimize2 className="w-3 h-3" />
          </button>
          <button
            type="button"
            aria-label="打开工作台"
            title="打开主工作台"
            onClick={() => void ipc.openMainWorkspace('overview')}
            className="btn-interactive p-1 rounded cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <ExternalLink className="w-3 h-3" />
          </button>
          <button
            type="button"
            aria-label="关闭悬浮窗"
            title="关闭悬浮窗"
            onClick={handleClose}
            className="btn-interactive p-1 rounded cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      </div>

      {/* Inline Customization Panel (when toggled) */}
      {customizing ? (
        <div className="flex-1 py-2 space-y-2.5 overflow-y-auto text-[11px]">
          <div className="space-y-1">
            <div className="flex items-center justify-between" style={{ color: 'var(--text-muted)' }}>
              <span>窗口尺寸</span>
              <span>可拖拽右下角自由缩放</span>
            </div>
            <div className="grid grid-cols-4 gap-1">
              {SIZE_PRESETS.map((preset) => {
                const active = floatingConfig.sizePreset === preset.id;
                return (
                  <button
                    key={preset.id}
                    type="button"
                    data-active={active ? 'true' : undefined}
                    onClick={() => handleSelectSizePreset(preset.id)}
                    className="seg-pill py-0.5 rounded text-[10.5px] cursor-pointer"
                    style={{
                      border: active
                        ? '1px solid var(--accent-primary)'
                        : '1px solid var(--border-hairline)',
                      color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                    }}
                  >
                    {preset.label}
                  </button>
                );
              })}
            </div>
          </div>

          <div className="space-y-1">
            <div style={{ color: 'var(--text-muted)' }}>显示元素</div>
            <div className="flex flex-wrap gap-1">
              {ELEMENT_TOGGLES.map((item) => {
                const active = floatingConfig[item.key];
                return (
                  <button
                    key={item.key}
                    type="button"
                    data-active={active ? 'true' : undefined}
                    onClick={() => handleToggleElement(item.key)}
                    className="seg-pill px-2 py-0.5 rounded text-[10.5px] cursor-pointer"
                    style={{
                      border: active
                        ? '1px solid var(--accent-primary)'
                        : '1px solid var(--border-hairline)',
                      color: active ? 'var(--text-primary)' : 'var(--text-muted)',
                    }}
                  >
                    {item.label}
                  </button>
                );
              })}
            </div>
          </div>

          <label className="block space-y-1">
            <div className="flex items-center justify-between">
              <span style={{ color: 'var(--text-muted)' }}>背景不透明度</span>
              <span className="tabular-nums">{Math.round(floatingConfig.opacity * 100)}%</span>
            </div>
            <input
              type="range"
              aria-label="悬浮窗不透明度"
              min={0.55}
              max={1}
              step={0.01}
              value={floatingConfig.opacity}
              onChange={(e) => {
                const val = Number.parseFloat(e.target.value);
                if (Number.isFinite(val)) {
                  updateFloatingConfig({ opacity: val });
                }
              }}
              className="w-full cursor-pointer accent-[var(--accent-primary)]"
            />
          </label>
        </div>
      ) : (
        <div className="flex-1 flex flex-col justify-between min-h-0">
          {/* Live Token & TPS Readout */}
          {(floatingConfig.showTokens || floatingConfig.showCost || floatingConfig.showSpeed) && (
            <div className="py-1.5 flex items-baseline justify-between gap-2">
              <div className="min-w-0">
                {floatingConfig.showTokens && (
                  <div className="flex items-center gap-1.5">
                    <span className={`${tokenFontClass} font-semibold tracking-tight tabular-nums`}>
                      <AnimatedNumber value={todayTokens} formatter={formatTokens} />
                    </span>
                    {recentDeltaTokens > 0 && (
                      <span
                        key={burstKey}
                        className="hud-burst-badge text-[10.5px] font-medium px-1 py-0.5 rounded-sm tabular-nums"
                        style={{
                          backgroundColor: 'rgba(16, 185, 129, 0.14)',
                          color: 'var(--accent-emerald)',
                        }}
                      >
                        +{formatTokens(recentDeltaTokens)}
                      </span>
                    )}
                  </div>
                )}
                {floatingConfig.showCost && (
                  <div className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                    今日等效{' '}
                    <span style={{ color: 'var(--text-secondary)' }}>
                      <AnimatedNumber
                        value={summary?.today_cost_micro_usd ?? 0}
                        formatter={(v) => formatCost(v, currency, rate)}
                      />
                    </span>
                  </div>
                )}
              </div>

              {floatingConfig.showSpeed && (
                <div className="text-right shrink-0 tabular-nums">
                  <div className="text-[12.5px] font-semibold" style={{ color: 'var(--accent-sky)' }}>
                    {formatTps(effectiveTps)}
                  </div>
                  <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                    实时生成速率
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Active Session Strip */}
          {floatingConfig.showSession && (
            <div
              className="py-1.5"
              style={{ borderTop: '1px solid var(--border-hairline)' }}
            >
              {activeSession ? (
                <button
                  type="button"
                  onClick={() => void ipc.openMainWorkspace('overview', activeSession.provider_id)}
                  className="row-interactive w-full text-left px-1 py-0.5 rounded flex items-center justify-between gap-2 cursor-pointer"
                >
                  <div className="min-w-0 flex-1">
                    <div className="text-[11px] font-medium truncate">
                      {formatSessionDisplayName(activeSession)}
                    </div>
                    <div className="text-[10px] truncate" style={{ color: 'var(--text-muted)' }}>
                      {providerDisplayName(activeSession.provider_id)} · {activeSession.model_id}
                    </div>
                  </div>
                  <span
                    className="text-[10.5px] tabular-nums shrink-0"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    {formatTokens(activeSession.total_tokens)}
                  </span>
                </button>
              ) : (
                <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                  15 分钟内暂无活跃会话
                </div>
              )}
            </div>
          )}

          {/* Top Subscription Windows */}
          {floatingConfig.showSubscriptions && (
            <div
              className="pt-1.5 space-y-1.5"
              style={{ borderTop: '1px solid var(--border-hairline)' }}
            >
              {topSubs.length > 0 ? (
                topSubs.map((sub) => {
                  const win = sub.windows[0];
                  if (!win) return null;
                  return (
                    <div key={sub.plan.plan_id} className="space-y-0.5">
                      <div className="flex items-center justify-between text-[10.5px] tabular-nums">
                        <span
                          className="truncate max-w-[165px]"
                          style={{ color: 'var(--text-secondary)' }}
                        >
                          {providerDisplayName(sub.plan.provider_id)} · {win.label}
                        </span>
                        <span style={{ color: 'var(--text-primary)' }}>
                          {win.used_percent.toFixed(0)}% · {formatCountdown(win.resets_in_seconds)}
                        </span>
                      </div>
                      <InlineProgressBar
                        percent={win.used_percent}
                        alertLevel={win.alert_level}
                        heightPx={3}
                      />
                    </div>
                  );
                })
              ) : (
                <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                  暂无启用订阅配额
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* Corner Resize Grip */}
      <div
        onMouseDown={handleResizeDragStart}
        aria-label="调整悬浮窗大小"
        title="拖拽调整悬浮窗大小"
        className="absolute right-0.5 bottom-0.5 w-3 h-3 cursor-se-resize opacity-45 hover:opacity-90"
        style={{
          borderRight: '2px solid var(--text-muted)',
          borderBottom: '2px solid var(--text-muted)',
        }}
      />
    </div>
  );
}
