import { useEffect, useMemo, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ExternalLink, Maximize2, Minimize2, SlidersHorizontal, X } from 'lucide-react';
import { AnimatedNumber } from '../components/AnimatedNumber';
import { InlineProgressBar } from '../components/InlineProgressBar';
import { LiveSpeedBadge } from '../components/LiveSpeedBadge';
import {
  computeFloatingDimensions,
  useFloatingConfig,
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
  { key: 'showSubscriptions', label: '配额窗口' },
];

export function FloatingMonitorPage({ livePulseSeq = 0 }: FloatingMonitorPageProps) {
  const [floatingConfig, updateFloatingConfig] = useFloatingConfig();
  const [customizing, setCustomizing] = useState(false);
  const [recentDeltaTokens, setRecentDeltaTokens] = useState<number>(0);
  const [burstKey, setBurstKey] = useState(0);
  const prevTodayTokensRef = useRef<number | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [viewport, setViewport] = useState<{ width: number; height: number }>(() => ({
    width: typeof window !== 'undefined' && window.innerWidth > 0 ? window.innerWidth : 304,
    height: typeof window !== 'undefined' && window.innerHeight > 0 ? window.innerHeight : 248,
  }));

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

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const syncSize = () => {
      const w = rootRef.current?.clientWidth || window.innerWidth || 304;
      const h = rootRef.current?.clientHeight || window.innerHeight || 248;
      setViewport((prev) => (prev.width === w && prev.height === h ? prev : { width: w, height: h }));
    };
    syncSize();
    window.addEventListener('resize', syncSize);
    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver !== 'undefined' && rootRef.current) {
      observer = new ResizeObserver(() => syncSize());
      observer.observe(rootRef.current);
    }
    return () => {
      window.removeEventListener('resize', syncSize);
      observer?.disconnect();
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

  const isCompactViewport = floatingConfig.compact || viewport.height <= 68;

  const handleToggleCompact = () => {
    const nextCompact = !floatingConfig.compact;
    setCustomizing(false);
    const nextCfg = updateFloatingConfig({ compact: nextCompact });
    if (nextCompact) {
      void ipc.setFloatingWindowCompact(true);
    } else {
      const dims = computeFloatingDimensions(nextCfg, viewport.width);
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
    if (!nextCfg.compact) {
      const dims = computeFloatingDimensions(nextCfg, viewport.width);
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

  // Dynamically determine how many sessions and subscription windows fit the current window height
  const maxSessionRows =
    viewport.height >= 320 ? 3 : viewport.height >= 255 ? 2 : 1;
  const visibleSessions = (summary?.active_sessions ?? []).slice(0, maxSessionRows);

  const flattenedSubWindows = useMemo(() => {
    const list: Array<{
      key: string;
      providerId: string;
      planName: string;
      win: NonNullable<typeof summary>['subscriptions'][number]['windows'][number];
    }> = [];
    const subs = summary?.subscriptions ?? [];
    // First pass: primary window of each enabled subscription
    for (const sub of subs) {
      const first = sub.windows[0];
      if (first) {
        list.push({
          key: `${sub.plan.plan_id}-${first.window_id}`,
          providerId: sub.plan.provider_id,
          planName: sub.plan.plan_name,
          win: first,
        });
      }
    }
    // Second pass: additional windows (e.g. 7d / 30d) when window is tall enough
    for (const sub of subs) {
      for (let i = 1; i < sub.windows.length; i += 1) {
        const w = sub.windows[i];
        list.push({
          key: `${sub.plan.plan_id}-${w.window_id}`,
          providerId: sub.plan.provider_id,
          planName: sub.plan.plan_name,
          win: w,
        });
      }
    }
    return list;
  }, [summary]);

  const maxSubRows =
    viewport.height >= 330
      ? 4
      : viewport.height >= 265
        ? 3
        : viewport.height < 180
          ? 1
          : 2;
  const visibleSubWindows = flattenedSubWindows.slice(0, maxSubRows);

  const isLight = resolvedTheme === 'light';
  const alpha = floatingConfig.opacity;
  const lowAlpha = Math.max(0.5, alpha - 0.1);
  const surfaceGradient = isLight
    ? `linear-gradient(155deg, rgba(255, 255, 255, ${alpha}) 0%, rgba(242, 245, 252, ${lowAlpha}) 100%)`
    : `linear-gradient(155deg, rgba(26, 30, 44, ${alpha}) 0%, rgba(13, 16, 25, ${lowAlpha}) 100%)`;
  const borderColor = isLight
    ? 'rgba(15, 23, 42, 0.14)'
    : 'rgba(255, 255, 255, 0.14)';
  const specularRim = isLight
    ? 'inset 0 1px 0 rgba(255, 255, 255, 0.92), 0 12px 32px -10px rgba(15, 23, 42, 0.22)'
    : 'inset 0 1px 0 rgba(255, 255, 255, 0.18), 0 14px 36px -10px rgba(0, 0, 0, 0.65)';

  if (isCompactViewport) {
    return (
      <div
        ref={rootRef}
        data-theme={resolvedTheme}
        className="h-screen w-screen flex items-center justify-between gap-2 px-3.5 select-none overflow-visible rounded-full"
        style={{
          backgroundImage: surfaceGradient,
          backdropFilter: 'blur(24px) saturate(180%)',
          WebkitBackdropFilter: 'blur(24px) saturate(180%)',
          color: 'var(--text-primary)',
          border: `1px solid ${borderColor}`,
          boxShadow: specularRim,
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
          {floatingConfig.showCost && viewport.width >= 190 && (
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
          {floatingConfig.showSpeed && viewport.width >= 235 && (
            <LiveSpeedBadge
              activeSessions={summary?.active_sessions}
              liveModelSpeeds={summary?.live_model_speeds}
              fallbackTps={summary?.latest_output_tps}
              align="left"
              placement="bottom"
              className="shrink-0"
            >
              {(breakdown) => (
                <span
                  className="text-[10.5px] tabular-nums cursor-default"
                  style={{ color: 'var(--accent-sky)' }}
                >
                  {formatTps(breakdown.primaryTps)}
                </span>
              )}
            </LiveSpeedBadge>
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
            className="btn-interactive p-1 rounded-lg cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <Maximize2 className="w-3 h-3" />
          </button>
          <button
            type="button"
            aria-label="关闭悬浮窗"
            title="关闭悬浮窗"
            onClick={handleClose}
            className="btn-interactive p-1 rounded-lg cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      </div>
    );
  }

  const isWide = viewport.width >= 350;
  const isNarrow = viewport.width < 255;
  const isShort = viewport.height < 185;
  const tokenFontClass =
    isWide && viewport.height >= 240
      ? 'text-[23px]'
      : isNarrow || isShort
        ? 'text-[17px]'
        : 'text-[20px]';
  const containerPaddingClass = isShort || isNarrow ? 'p-2.5' : 'p-3';

  return (
    <div
      ref={rootRef}
      data-theme={resolvedTheme}
      className={`relative h-screen w-screen flex flex-col justify-between ${containerPaddingClass} select-none overflow-hidden rounded-2xl`}
      style={{
        backgroundImage: surfaceGradient,
        backdropFilter: 'blur(24px) saturate(180%)',
        WebkitBackdropFilter: 'blur(24px) saturate(180%)',
        color: 'var(--text-primary)',
        border: `1px solid ${borderColor}`,
        boxShadow: specularRim,
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
            title="自定义悬浮窗显示元素"
            data-active={customizing ? 'true' : undefined}
            onClick={() => setCustomizing((prev) => !prev)}
            className="btn-interactive p-1 rounded-lg cursor-pointer"
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
            className="btn-interactive p-1 rounded-lg cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <Minimize2 className="w-3 h-3" />
          </button>
          <button
            type="button"
            aria-label="打开工作台"
            title="打开主工作台"
            onClick={() => void ipc.openMainWorkspace('overview')}
            className="btn-interactive p-1 rounded-lg cursor-pointer"
            style={{ color: 'var(--text-secondary)' }}
          >
            <ExternalLink className="w-3 h-3" />
          </button>
          <button
            type="button"
            aria-label="关闭悬浮窗"
            title="关闭悬浮窗"
            onClick={handleClose}
            className="btn-interactive p-1 rounded-lg cursor-pointer"
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
                    className="seg-pill px-2 py-0.5 rounded-lg text-[10.5px] cursor-pointer"
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
                        className="hud-burst-badge text-[10.5px] font-medium px-1 py-0.5 rounded-md tabular-nums"
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
                <LiveSpeedBadge
                  activeSessions={summary?.active_sessions}
                  liveModelSpeeds={summary?.live_model_speeds}
                  fallbackTps={summary?.latest_output_tps}
                  align="right"
                  placement="bottom"
                  className="text-right shrink-0 tabular-nums cursor-default"
                >
                  {(breakdown) => (
                    <>
                      <div
                        className="text-[12.5px] font-semibold"
                        style={{ color: 'var(--accent-sky)' }}
                      >
                        {formatTps(breakdown.primaryTps)}
                      </div>
                      <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                        {breakdown.isMultiModel
                          ? `实时生成速率 (${breakdown.items.length} 模型均值)`
                          : '实时生成速率'}
                      </div>
                    </>
                  )}
                </LiveSpeedBadge>
              )}
            </div>
          )}

          {/* Active Session Strip */}
          {floatingConfig.showSession && (
            <div
              className="py-1.5 space-y-1"
              style={{ borderTop: '1px solid var(--border-hairline)' }}
            >
              {visibleSessions.length > 0 ? (
                visibleSessions.map((sess) => (
                  <button
                    key={`${sess.provider_id}-${sess.session_key}`}
                    type="button"
                    onClick={() => void ipc.openMainWorkspace('overview', sess.provider_id)}
                    className="row-interactive w-full text-left px-1 py-0.5 rounded-lg flex items-center justify-between gap-2 cursor-pointer"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="text-[11px] font-medium truncate">
                        {formatSessionDisplayName(sess)}
                      </div>
                      {!isShort && (
                        <div
                          className="text-[10px] truncate"
                          style={{ color: 'var(--text-muted)' }}
                        >
                          {providerDisplayName(sess.provider_id)} · {sess.model_id}
                        </div>
                      )}
                    </div>
                    <span
                      className="text-[10.5px] tabular-nums shrink-0"
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      {formatTokens(sess.total_tokens)}
                    </span>
                  </button>
                ))
              ) : (
                <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                  等待活跃会话
                </div>
              )}
            </div>
          )}

          {/* Subscription Windows */}
          {floatingConfig.showSubscriptions && (
            <div
              className="pt-1.5 space-y-1.5"
              style={{ borderTop: '1px solid var(--border-hairline)' }}
            >
              {visibleSubWindows.length > 0 ? (
                visibleSubWindows.map(({ key, providerId, win }) => (
                  <div key={key} className="space-y-0.5">
                    <div className="flex items-center justify-between text-[10.5px] tabular-nums">
                      <span
                        className="truncate max-w-[65%]"
                        style={{ color: 'var(--text-secondary)' }}
                      >
                        {providerDisplayName(providerId)} · {win.label}
                      </span>
                      <span style={{ color: 'var(--text-primary)' }}>
                        已用 {win.used_percent.toFixed(0)}% · {formatCountdown(win.resets_in_seconds)}
                      </span>
                    </div>
                    <InlineProgressBar
                      percent={win.used_percent}
                      alertLevel={win.alert_level}
                      heightPx={3}
                    />
                  </div>
                ))
              ) : (
                <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                  等待官方配额同步
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
