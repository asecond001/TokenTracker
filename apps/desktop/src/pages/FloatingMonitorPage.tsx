import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ExternalLink, Maximize2, Minimize2, X } from 'lucide-react';
import { AnimatedNumber } from '../components/AnimatedNumber';
import { InlineProgressBar } from '../components/InlineProgressBar';
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

export function FloatingMonitorPage({ livePulseSeq = 0 }: FloatingMonitorPageProps) {
  const [compact, setCompact] = useState(false);
  const [recentDeltaTokens, setRecentDeltaTokens] = useState<number>(0);
  const [burstKey, setBurstKey] = useState(0);
  const prevTodayTokensRef = useRef<number | null>(null);

  const {
    tzOffsetMinutes,
    currency: storeCurrency,
    usdToCnyRate: storeRate,
    setTheme,
    setCurrency,
    setUsdToCnyRate,
  } = useWorkspaceStore();

  const { data: prefs } = useQuery({
    queryKey: ['preferences'],
    queryFn: () => ipc.getPreferences(),
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

  const handleToggleCompact = () => {
    const next = !compact;
    setCompact(next);
    void ipc.setFloatingWindowCompact(next);
  };

  const handleClose = () => {
    void ipc.toggleFloatingWindow();
  };

  const handleDragStart = (e: React.MouseEvent) => {
    if (e.button !== 0 || !isTauriRuntime()) return;
    void getCurrentWindow().startDragging();
  };

  const currency = prefs?.currency ?? summary?.currency ?? storeCurrency;
  const rate = prefs?.usd_to_cny_rate ?? summary?.usd_to_cny_rate ?? storeRate;
  const activeSession = summary?.active_sessions?.[0] ?? null;
  const topSubs = (summary?.subscriptions ?? []).slice(0, 2);
  const effectiveTps = activeSession?.latest_output_tps ?? summary?.latest_output_tps ?? null;

  if (compact) {
    return (
      <div
        className="h-screen w-screen flex items-center justify-between gap-2 px-3 select-none overflow-hidden"
        style={{
          backgroundColor: 'var(--bg-sidebar)',
          color: 'var(--text-primary)',
          border: '1px solid var(--border-strong)',
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
          <span className="text-[13px] font-semibold tabular-nums truncate">
            <AnimatedNumber value={todayTokens} formatter={formatTokens} />
          </span>
          <span
            className="text-[11px] tabular-nums shrink-0"
            style={{ color: 'var(--text-secondary)' }}
          >
            <AnimatedNumber
              value={summary?.today_cost_micro_usd ?? 0}
              formatter={(v) => formatCost(v, currency, rate)}
            />
          </span>
          <span
            className="text-[10.5px] tabular-nums shrink-0"
            style={{ color: 'var(--accent-sky)' }}
          >
            {formatTps(effectiveTps)}
          </span>
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

  return (
    <div
      className="h-screen w-screen flex flex-col justify-between p-3 select-none overflow-hidden"
      style={{
        backgroundColor: 'var(--bg-canvas)',
        color: 'var(--text-primary)',
        border: '1px solid var(--border-strong)',
      }}
    >
      {/* Top Drag Header */}
      <div
        className="flex items-center justify-between pb-1.5"
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
            TokenTracker 实时悬浮窗
          </span>
        </div>
        <div className="flex items-center gap-0.5 shrink-0">
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

      {/* Live Token & TPS Readout */}
      <div className="py-1.5 flex items-baseline justify-between gap-2">
        <div className="min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-[20px] font-semibold tracking-tight tabular-nums">
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
          <div className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            今日等效{' '}
            <span style={{ color: 'var(--text-secondary)' }}>
              <AnimatedNumber
                value={summary?.today_cost_micro_usd ?? 0}
                formatter={(v) => formatCost(v, currency, rate)}
              />
            </span>
          </div>
        </div>

        <div className="text-right shrink-0 tabular-nums">
          <div className="text-[12.5px] font-semibold" style={{ color: 'var(--accent-sky)' }}>
            {formatTps(effectiveTps)}
          </div>
          <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
            实时生成速率
          </div>
        </div>
      </div>

      {/* Active Session Strip */}
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

      {/* Top Subscription Windows */}
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
                  <span className="truncate max-w-[155px]" style={{ color: 'var(--text-secondary)' }}>
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
    </div>
  );
}
