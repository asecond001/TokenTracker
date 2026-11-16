import { useCallback, useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { AnimatedNumber } from '../components/AnimatedNumber';
import { EChartBox } from '../components/EChartBox';
import { InlineProgressBar } from '../components/InlineProgressBar';
import { LiveSpeedBadge } from '../components/LiveSpeedBadge';
import { TimeRangeControl } from '../components/TimeRangeControl';
import { useChartTheme } from '../lib/chartTheme';
import {
  TOKEN_TYPE_COLORS,
  formatCost,
  formatExactInt,
  formatPercent,
  formatRelativeTime,
  formatSessionDisplayName,
  formatTokens,
  formatTps,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { computeTimeRange, useWorkspaceStore } from '../stores/workspaceStore';
import type { AnalyticsFilter } from '../types/ipc';

type OverviewChartDimension = 'provider' | 'token_type';

function DeltaBadge({ pct }: { pct: number | null }) {
  if (pct === null || !Number.isFinite(pct)) {
    return (
      <span className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
        环比 —
      </span>
    );
  }
  const positive = pct >= 0;
  const sign = positive ? '+' : '';
  return (
    <span
      className="text-[11px] font-medium tabular-nums"
      style={{ color: positive ? 'var(--accent-emerald)' : 'var(--accent-rose)' }}
    >
      {sign}
      {pct.toFixed(1)}%
    </span>
  );
}

function formatBucketLabel(tsMs: number, granularity: string | undefined): string {
  const d = new Date(tsMs);
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  if (granularity === 'hourly') {
    return `${mm}-${dd} ${String(d.getHours()).padStart(2, '0')}:00`;
  }
  return `${d.getFullYear()}-${mm}-${dd}`;
}

export function OverviewPage() {
  const {
    timePreset,
    customStartDate,
    customEndDate,
    providerFilter,
    deviceFilter,
    currency,
    usdToCnyRate,
    tzOffsetMinutes,
    setActivePage,
  } = useWorkspaceStore();
  const chartTheme = useChartTheme();
  const [chartDimension, setChartDimension] = useState<OverviewChartDimension>('provider');
  const [chartType, setChartType] = useState<'bar' | 'line'>('bar');
  const [focusedSeriesId, setFocusedSeriesId] = useState<string | null>(null);

  const filter = useMemo<AnalyticsFilter>(() => {
    const range = computeTimeRange(timePreset, {
      start: customStartDate,
      end: customEndDate,
    });
    return {
      start_ms: range.start_ms,
      end_ms: range.end_ms,
      provider_id: providerFilter === 'all' ? null : providerFilter,
      device_id: deviceFilter === 'all' ? null : deviceFilter,
      tz_offset_minutes: tzOffsetMinutes,
      all_time: range.all_time,
    };
  }, [timePreset, customStartDate, customEndDate, providerFilter, deviceFilter, tzOffsetMinutes]);

  const { data, isLoading } = useQuery({
    queryKey: ['overview', filter],
    queryFn: () => ipc.getOverview(filter),
    placeholderData: keepPreviousData,
    refetchInterval: 10_000,
  });

  const costFormatter = useCallback(
    (value: number) => formatCost(value, currency, usdToCnyRate),
    [currency, usdToCnyRate]
  );

  const percentFormatter = useCallback((value: number) => formatPercent(value, true), []);

  const showDeltas = timePreset !== 'all';
  const activeProviderSeries = useMemo(
    () =>
      (data?.provider_series ?? []).filter(
        (ps) => ps.total_tokens > 0 && ps.points.some((pt) => pt.tokens > 0)
      ),
    [data]
  );

  const hasChartData =
    chartDimension === 'provider'
      ? activeProviderSeries.length > 0
      : (data?.series.length ?? 0) > 0 && (data?.total_tokens ?? 0) > 0;

  const chartOption = useMemo<EChartsOption>(() => {
    const bucketSeries = data?.series ?? [];
    const bucketTimestamps =
      bucketSeries.length > 0
        ? bucketSeries.map((pt) => pt.bucket_start_ms)
        : (activeProviderSeries[0]?.points.map((pt) => pt.bucket_start_ms) ?? []);
    const xLabels = bucketTimestamps.map((ts) =>
      formatBucketLabel(ts, data?.series_granularity)
    );
    const isLine = chartType === 'line';

    const tokenTypeCandidates = [
      {
        id: 'token-input',
        name: 'Input',
        color: TOKEN_TYPE_COLORS.input,
        values: bucketSeries.map((p) => p.input_tokens),
      },
      {
        id: 'token-output',
        name: 'Output',
        color: TOKEN_TYPE_COLORS.output,
        values: bucketSeries.map((p) => p.output_tokens + p.reasoning_tokens),
      },
      {
        id: 'token-cache-read',
        name: 'Cache Read',
        color: TOKEN_TYPE_COLORS.cache_read,
        values: bucketSeries.map((p) => p.cache_read_tokens),
      },
      {
        id: 'token-cache-write',
        name: 'Cache Write',
        color: TOKEN_TYPE_COLORS.cache_write,
        values: bucketSeries.map((p) => p.cache_write_tokens),
      },
    ].filter((entry) => entry.values.some((v) => v > 0));

    const buildSeriesEntry = (
      id: string,
      name: string,
      color: string,
      values: number[]
    ) => {
      const hasFocus = focusedSeriesId !== null;
      const isFocused = focusedSeriesId === id;
      const opacity = hasFocus ? (isFocused ? 1 : 0.22) : 1;

      if (isLine) {
        return {
          id,
          name,
          type: 'line' as const,
          smooth: 0.25,
          showSymbol: false,
          symbolSize: 5,
          universalTransition: true,
          emphasis: { focus: 'series' as const },
          lineStyle: {
            width: isFocused ? 2.5 : 1.75,
            color,
            opacity,
          },
          areaStyle: {
            color,
            opacity: hasFocus ? (isFocused ? 0.14 : 0.015) : chartTheme.isLight ? 0.07 : 0.05,
          },
          itemStyle: { color, opacity },
          data: values,
        };
      }

      return {
        id,
        name,
        type: 'bar' as const,
        stack: 'tokens',
        barMaxWidth: 28,
        barCategoryGap: '28%',
        universalTransition: true,
        emphasis: { focus: 'series' as const },
        itemStyle: {
          color,
          opacity,
          borderColor: chartTheme.canvasBg,
          borderWidth: 1.5,
          borderRadius: [2, 2, 0, 0],
        },
        data: values,
      };
    };

    const seriesConfig: EChartsOption['series'] =
      chartDimension === 'provider'
        ? activeProviderSeries.map((ps) =>
            buildSeriesEntry(
              `provider-${ps.key}`,
              ps.label || providerDisplayName(ps.key),
              providerAccentColor(ps.key),
              ps.points.map((pt) => pt.tokens)
            )
          )
        : tokenTypeCandidates.map((entry) =>
            buildSeriesEntry(entry.id, entry.name, entry.color, entry.values)
          );

    return {
      backgroundColor: 'transparent',
      animationDuration: 280,
      animationDurationUpdate: 260,
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: isLine ? 'line' : 'shadow' },
        backgroundColor: chartTheme.tooltipBg,
        borderColor: chartTheme.tooltipBorder,
        textStyle: { color: chartTheme.tooltipText, fontSize: 12 },
      },
      legend: {
        top: 0,
        right: 0,
        textStyle: { color: chartTheme.textSecondary, fontSize: 11 },
        itemWidth: 10,
        itemHeight: 6,
      },
      grid: { top: 32, right: 12, bottom: 24, left: 52, containLabel: false },
      xAxis: {
        type: 'category',
        data: xLabels,
        axisLine: { lineStyle: { color: chartTheme.axisLineColor } },
        axisLabel: { color: chartTheme.textMuted, fontSize: 11 },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        splitLine: { lineStyle: { color: chartTheme.splitLineColor, type: 'dashed' } },
        axisLabel: {
          color: chartTheme.textMuted,
          fontSize: 11,
          formatter: (val: number) => formatTokens(val),
        },
      },
      series: seriesConfig,
    };
  }, [data, activeProviderSeries, chartDimension, chartType, chartTheme, focusedSeriesId]);

  const seriesPills = useMemo(() => {
    if (chartDimension === 'provider') {
      return activeProviderSeries.map((ps) => ({
        id: `provider-${ps.key}`,
        label: ps.label || providerDisplayName(ps.key),
        color: providerAccentColor(ps.key),
      }));
    }
    const bucketSeries = data?.series ?? [];
    return [
      { id: 'token-input', label: 'Input', color: TOKEN_TYPE_COLORS.input, has: bucketSeries.some((p) => p.input_tokens > 0) },
      { id: 'token-output', label: 'Output', color: TOKEN_TYPE_COLORS.output, has: bucketSeries.some((p) => p.output_tokens + p.reasoning_tokens > 0) },
      { id: 'token-cache-read', label: 'Cache Read', color: TOKEN_TYPE_COLORS.cache_read, has: bucketSeries.some((p) => p.cache_read_tokens > 0) },
      { id: 'token-cache-write', label: 'Cache Write', color: TOKEN_TYPE_COLORS.cache_write, has: bucketSeries.some((p) => p.cache_write_tokens > 0) },
    ]
      .filter((e) => e.has)
      .map(({ id, label, color }) => ({ id, label, color }));
  }, [chartDimension, activeProviderSeries, data]);

  const liveSessions = data?.active_sessions ?? [];
  const activeCount = liveSessions.filter((s) => s.activity_state === 'active').length;
  const recentCount = liveSessions.length - activeCount;

  return (
    <div className="space-y-7">
      {/* Top 2-Column Live Pulse & Active/Recent Sessions Strip (Flat Hairline Layout) */}
      <section
        className="grid grid-cols-1 lg:grid-cols-12 gap-6 pb-6"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        {/* Left: Today Spend + Provider Share Bar + Top Models */}
        <div
          className="lg:col-span-5 lg:pr-6 space-y-4 lg:border-r"
          style={{ borderColor: 'var(--border-hairline)' }}
        >
          <div className="flex items-baseline justify-between gap-3">
            <div>
              <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
                今日实时用量
              </div>
              <div className="mt-1 flex items-baseline gap-2.5 tabular-nums">
                <span className="text-[22px] font-semibold tracking-tight">
                  <AnimatedNumber
                    value={data?.today_cost_micro_usd ?? 0}
                    formatter={(v) => formatCost(v, currency, usdToCnyRate)}
                  />
                </span>
                <span className="text-[12px]" style={{ color: 'var(--text-muted)' }}>
                  ·{' '}
                  <AnimatedNumber
                    value={data?.today_tokens ?? 0}
                    formatter={formatTokens}
                  />{' '}
                  Tokens
                </span>
              </div>
            </div>
          </div>

          {/* Stacked Provider Share Bar */}
          <div className="space-y-2">
            <div
              className="flex items-center justify-between text-[11px]"
              style={{ color: 'var(--text-secondary)' }}
            >
              <span>今日模型提供商占比</span>
              <span className="tabular-nums" style={{ color: 'var(--text-muted)' }}>
                {(data?.provider_shares ?? []).length} 个活跃源
              </span>
            </div>
            {(data?.provider_shares ?? []).length > 0 ? (
              <>
                <div
                  className="h-2 w-full rounded-sm overflow-hidden flex"
                  style={{ backgroundColor: 'var(--bg-hover)' }}
                >
                  {(data?.provider_shares ?? []).map((share) => (
                    <div
                      key={share.key}
                      style={{
                        width: `${Math.max(1.5, share.token_share_percent)}%`,
                        backgroundColor: providerAccentColor(share.key),
                      }}
                      title={`${providerDisplayName(share.key)}: ${formatTokens(share.tokens)} (${share.token_share_percent.toFixed(1)}%)`}
                    />
                  ))}
                </div>
                <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-[11px] tabular-nums">
                  {(data?.provider_shares ?? []).slice(0, 6).map((share) => (
                    <div key={share.key} className="flex items-center justify-between gap-2 min-w-0">
                      <span className="inline-flex items-center gap-1.5 min-w-0">
                        <span
                          className="inline-block w-2 h-2 rounded-full shrink-0"
                          style={{ backgroundColor: providerAccentColor(share.key) }}
                        />
                        <span className="truncate" style={{ color: 'var(--text-secondary)' }}>
                          {providerDisplayName(share.key)}
                        </span>
                      </span>
                      <span style={{ color: 'var(--text-muted)' }}>
                        {share.token_share_percent.toFixed(1)}% ·{' '}
                        {formatCost(share.cost_micro_usd, currency, usdToCnyRate)}
                      </span>
                    </div>
                  ))}
                </div>
              </>
            ) : (
              <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                今日暂无用量占比数据
              </div>
            )}
          </div>

          {/* Top 3 Model Share Breakdown */}
          <div
            className="pt-3 space-y-2"
            style={{ borderTop: '1px solid var(--border-subtle)' }}
          >
            <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
              今日 Top 3 模型消耗占比
            </div>
            {(data?.top_model_shares ?? []).slice(0, 3).map((model) => (
              <div key={model.key} className="space-y-1 text-[11px] tabular-nums">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-mono truncate" style={{ color: 'var(--text-primary)' }}>
                    {model.label}
                  </span>
                  <span className="shrink-0" style={{ color: 'var(--text-muted)' }}>
                    {model.token_share_percent.toFixed(1)}% · {formatTokens(model.tokens)}
                  </span>
                </div>
                <InlineProgressBar
                  percent={model.token_share_percent}
                  accentColor="var(--accent-sky)"
                  heightPx={3}
                />
              </div>
            ))}
            {(data?.top_model_shares ?? []).length === 0 && (
              <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                暂无模型分布记录
              </div>
            )}
          </div>
        </div>

        {/* Right: Active & Recent Sessions (Last 24h) */}
        <div className="lg:col-span-7 flex flex-col min-w-0">
          <div className="flex items-center justify-between gap-3 pb-2.5">
            <div className="flex items-center gap-2">
              <span className="text-[12px] font-medium">活跃与近期会话（15m）</span>
              <span
                className="px-1.5 py-0.5 rounded text-[10.5px] tabular-nums"
                style={{
                  backgroundColor: 'rgba(16, 185, 129, 0.12)',
                  color: 'var(--accent-emerald)',
                }}
              >
                {activeCount} 活跃
              </span>
              {recentCount > 0 && (
                <span
                  className="px-1.5 py-0.5 rounded text-[10.5px] tabular-nums"
                  style={{
                    backgroundColor: 'var(--bg-hover)',
                    color: 'var(--text-secondary)',
                  }}
                >
                  {recentCount} 近期
                </span>
              )}
            </div>

            <LiveSpeedBadge
              activeSessions={data?.active_sessions}
              liveModelSpeeds={data?.live_model_speeds}
              fallbackTps={data?.latest_output_tps}
              align="right"
              placement="bottom"
              className="tabular-nums cursor-default"
            >
              {(breakdown) => (
                <span
                  className="text-[11.5px] font-medium"
                  style={{ color: 'var(--accent-sky)' }}
                >
                  实时速率 {formatTps(breakdown.primaryTps)}
                  {breakdown.isMultiModel ? ` (${breakdown.items.length} 模型均值)` : ''}
                </span>
              )}
            </LiveSpeedBadge>
          </div>

          {liveSessions.length > 0 ? (
            <div
              className="divide-y max-h-[228px] overflow-y-auto pr-1"
              style={{ borderTop: '1px solid var(--border-subtle)' }}
            >
              {liveSessions.map((session) => {
                const isLive = session.activity_state === 'active';
                return (
                  <div
                    key={`${session.provider_id}-${session.session_key}`}
                    className="py-2 flex items-center justify-between gap-3 text-[12px]"
                    style={{ borderColor: 'var(--border-subtle)' }}
                  >
                    <div className="min-w-0 flex items-start gap-2.5">
                      <span
                        className={`mt-1.5 inline-block w-2 h-2 rounded-full shrink-0 ${isLive ? 'collector-dot-live' : ''}`}
                        style={{
                          backgroundColor: isLive ? 'var(--accent-emerald)' : 'var(--text-muted)',
                        }}
                      />
                      <div className="min-w-0">
                        <div className="flex items-center gap-2 min-w-0">
                          <span className="font-medium truncate">
                            {formatSessionDisplayName(session)}
                          </span>
                          <span
                            className="text-[10px] px-1.5 py-0.2 rounded shrink-0"
                            style={{
                              backgroundColor: isLive
                                ? 'rgba(16, 185, 129, 0.14)'
                                : 'var(--bg-hover)',
                              color: isLive
                                ? 'var(--accent-emerald)'
                                : 'var(--text-muted)',
                            }}
                          >
                            {isLive ? '活跃中' : '近期'}
                          </span>
                        </div>
                        <div
                          className="text-[11px] flex items-center gap-2 truncate tabular-nums"
                          style={{ color: 'var(--text-muted)' }}
                        >
                          <span className="truncate">
                            {providerDisplayName(session.provider_id)} · {session.model_id}
                          </span>
                          {session.project_label && (
                            <>
                              <span>·</span>
                              <span className="truncate">{session.project_label}</span>
                            </>
                          )}
                          <span>·</span>
                          <span>{session.request_count} 次请求</span>
                        </div>
                      </div>
                    </div>

                    <div className="shrink-0 text-right tabular-nums">
                      <div className="text-[12px] font-medium">
                        {formatTokens(session.total_tokens)}{' '}
                        <span style={{ color: 'var(--text-muted)' }}>
                          ({formatCost(session.estimated_cost_micro_usd, currency, usdToCnyRate)})
                        </span>
                      </div>
                      <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                        {formatTps(session.latest_output_tps)} ·{' '}
                        {formatRelativeTime(session.last_active_at_ms)}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : (
            <div
              className="py-8 text-center text-[12px]"
              style={{
                borderTop: '1px solid var(--border-subtle)',
                color: 'var(--text-muted)',
              }}
            >
              过去 15 分钟内暂无本地或远端活跃会话；运行受支持的 AI 编程工具后将在此实时显示。
            </div>
          )}
        </div>
      </section>

      {/* Top Flat Single-Row 4-Metric Strip (Separated by 1px Vertical Hairlines — No Bento Cards) */}
      <section
        className="grid grid-cols-4 pb-6"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div className="pr-5">
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            总 Token 消耗
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[24px] font-semibold tracking-tight tabular-nums">
              {isLoading && !data ? (
                '…'
              ) : (
                <AnimatedNumber value={data?.total_tokens ?? 0} formatter={formatTokens} />
              )}
            </span>
            {showDeltas && <DeltaBadge pct={data?.tokens_delta_pct ?? null} />}
          </div>
          <div className="mt-1 text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            {formatExactInt(data?.total_requests ?? 0)} 次请求 / 步
          </div>
        </div>

        <div
          className="px-5"
          style={{ borderLeft: '1px solid var(--border-hairline)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            API 等效成本 ({currency})
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[24px] font-semibold tracking-tight tabular-nums">
              {isLoading && !data ? (
                '…'
              ) : (
                <AnimatedNumber value={data?.total_cost_micro_usd ?? 0} formatter={costFormatter} />
              )}
            </span>
            {showDeltas && <DeltaBadge pct={data?.cost_delta_pct ?? null} />}
          </div>
        </div>

        <div
          className="px-5"
          style={{ borderLeft: '1px solid var(--border-hairline)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            平均缓存命中率
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[24px] font-semibold tracking-tight tabular-nums">
              {isLoading && !data ? (
                '…'
              ) : (
                <AnimatedNumber value={data?.cache_hit_ratio ?? 0} formatter={percentFormatter} />
              )}
            </span>
            {showDeltas && <DeltaBadge pct={data?.cache_delta_pct ?? null} />}
          </div>
        </div>

        <div
          className="pl-5"
          style={{ borderLeft: '1px solid var(--border-hairline)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            平均生成速度
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[24px] font-semibold tracking-tight tabular-nums">
              {isLoading && !data ? (
                '…'
              ) : (
                <AnimatedNumber value={data?.avg_output_tps ?? 0} formatter={formatTps} />
              )}
            </span>
            {showDeltas && <DeltaBadge pct={data?.tps_delta_pct ?? null} />}
          </div>
          <div className="mt-1 text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            <LiveSpeedBadge
              activeSessions={data?.active_sessions}
              liveModelSpeeds={data?.live_model_speeds}
              fallbackTps={data?.latest_output_tps}
              align="right"
              placement="bottom"
              className="cursor-default"
            >
              {(breakdown) => (
                <span>
                  当前{' '}
                  <span style={{ color: 'var(--accent-sky)' }}>
                    {formatTps(breakdown.primaryTps)}
                  </span>
                  {breakdown.isMultiModel ? ` (${breakdown.items.length} 模型均值)` : ''}
                </span>
              )}
            </LiveSpeedBadge>
            {' · '}峰值 {formatTps(data?.peak_output_tps ?? 0)}
          </div>
        </div>
      </section>

      {/* Mid Full-Width Borderless Consumption Trend Chart */}
      <section
        className="pb-6"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div className="flex flex-wrap items-center justify-between gap-3 mb-3">
          <div className="flex items-center gap-2.5">
            <h2 className="text-[13.5px] font-semibold">Token 构成趋势</h2>
            <span
              className="px-1.5 py-0.5 text-[10.5px] rounded tabular-nums"
              style={{
                backgroundColor: 'var(--bg-sidebar)',
                border: '1px solid var(--border-hairline)',
                color: 'var(--text-muted)',
              }}
            >
              {data?.series_granularity === 'hourly' ? '小时桶' : '日桶'}
            </span>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <TimeRangeControl />

            {/* Dimension Toggle */}
            <div
              className="flex items-center p-0.5 rounded"
              style={{
                backgroundColor: 'var(--bg-sidebar)',
                border: '1px solid var(--border-hairline)',
              }}
            >
              {(
                [
                  { id: 'provider', label: '按模型提供商' },
                  { id: 'token_type', label: '按 Token 类型' },
                ] as const
              ).map((dim) => {
                const active = chartDimension === dim.id;
                return (
                  <button
                    key={dim.id}
                    type="button"
                    data-active={active}
                    onClick={() => {
                      setChartDimension(dim.id);
                      setFocusedSeriesId(null);
                    }}
                    className="seg-pill px-2.5 py-0.5 text-[11.5px] rounded-sm cursor-pointer"
                    style={{
                      color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                      fontWeight: active ? 500 : 400,
                    }}
                  >
                    {dim.label}
                  </button>
                );
              })}
            </div>

            {/* Chart Type Toggle */}
            <div
              className="flex items-center p-0.5 rounded"
              style={{
                backgroundColor: 'var(--bg-sidebar)',
                border: '1px solid var(--border-hairline)',
              }}
            >
              {(['bar', 'line'] as const).map((kind) => {
                const active = chartType === kind;
                return (
                  <button
                    key={kind}
                    type="button"
                    data-active={active}
                    onClick={() => setChartType(kind)}
                    className="seg-pill px-2.5 py-0.5 text-[11.5px] rounded-sm cursor-pointer"
                    style={{
                      color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                      fontWeight: active ? 500 : 400,
                    }}
                  >
                    {kind === 'bar' ? '柱状图' : '流向曲线'}
                  </button>
                );
              })}
            </div>
          </div>
        </div>

        {seriesPills.length > 1 && hasChartData && (
          <div className="flex flex-wrap items-center gap-1.5 mb-2">
            {seriesPills.map((pill) => {
              const focused = focusedSeriesId === pill.id;
              const dimmed = focusedSeriesId !== null && !focused;
              return (
                <button
                  key={pill.id}
                  type="button"
                  data-active={focused ? 'true' : undefined}
                  onClick={() =>
                    setFocusedSeriesId((prev) => (prev === pill.id ? null : pill.id))
                  }
                  className="seg-pill inline-flex items-center gap-1.5 px-2 py-0.5 rounded text-[11px] cursor-pointer"
                  style={{
                    color: dimmed ? 'var(--text-muted)' : 'var(--text-secondary)',
                    opacity: dimmed ? 0.55 : 1,
                    border: '1px solid var(--border-hairline)',
                  }}
                >
                  <span
                    className="inline-block w-2 h-2 rounded-full"
                    style={{ backgroundColor: pill.color }}
                  />
                  <span>{pill.label}</span>
                </button>
              );
            })}
          </div>
        )}

        {!hasChartData ? (
          <div
            className="h-[240px] flex flex-col items-center justify-center text-[12.5px] gap-2"
            style={{ color: 'var(--text-muted)' }}
          >
            <span>当前时间窗暂无可观测用量流水</span>
            <button
              type="button"
              onClick={() => setActivePage('sources')}
              className="btn-interactive px-2.5 py-1 rounded text-[12px] underline cursor-pointer"
              style={{ color: 'var(--accent-primary)' }}
            >
              前往数据源与合规检查路径或触发扫描
            </button>
          </div>
        ) : (
          <EChartBox option={chartOption} height={260} />
        )}
      </section>

      {/* Bottom Flat Provider Summary Table */}
      <section>
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-[13.5px] font-semibold">模型提供商运行与消耗</h2>
        </div>

        <div className="overflow-x-auto">
          <table className="w-full border-collapse text-left">
            <thead>
              <tr
                className="text-[11px] font-medium uppercase tracking-wider"
                style={{
                  color: 'var(--text-muted)',
                  borderBottom: '1px solid var(--border-hairline)',
                }}
              >
                <th className="py-2 pr-4">模型提供商</th>
                <th className="py-2 px-3 text-right">今日用量 / 费用</th>
                <th className="py-2 px-3 text-right">本月用量 / 费用</th>
                <th className="py-2 px-3 text-right">缓存命中率</th>
                <th className="py-2 px-3 w-[220px]">主订阅窗口 / 限额</th>
                <th className="py-2 pl-3 text-right">最近活跃</th>
              </tr>
            </thead>
            <tbody>
              {(data?.providers ?? []).map((row) => (
                <tr
                  key={row.provider_id}
                  className="row-interactive"
                  style={{ borderBottom: '1px solid var(--border-hairline)' }}
                >
                  <td className="py-3 pr-4">
                    <div className="flex items-center gap-2">
                      <span
                        className="inline-block w-2 h-2 rounded-full shrink-0"
                        style={{ backgroundColor: providerAccentColor(row.provider_id) }}
                      />
                      <span className="font-medium">{providerDisplayName(row.provider_id)}</span>
                      {!row.enabled && (
                        <span
                          className="px-1.5 py-0.5 text-[10px] rounded"
                          style={{
                            backgroundColor: 'var(--bg-hover)',
                            color: 'var(--text-muted)',
                          }}
                        >
                          已停用
                        </span>
                      )}
                      {row.observable_steps > 0 && (
                        <span
                          className="px-1.5 py-0.5 text-[10px] rounded tabular-nums"
                          style={{
                            backgroundColor: 'var(--bg-hover)',
                            color: 'var(--text-secondary)',
                          }}
                          title="本地公开日志未包含 Token 计数字段的纯可观测步数"
                        >
                          +{row.observable_steps} 步追踪
                        </span>
                      )}
                    </div>
                  </td>
                  <td className="py-3 px-3 text-right tabular-nums">
                    <div>{formatTokens(row.today_tokens)}</div>
                    <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                      {formatCost(row.today_cost_micro_usd, currency, usdToCnyRate)}
                    </div>
                  </td>
                  <td className="py-3 px-3 text-right tabular-nums">
                    <div>{formatTokens(row.month_tokens)}</div>
                    <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                      {formatCost(row.month_cost_micro_usd, currency, usdToCnyRate)}
                    </div>
                  </td>
                  <td className="py-3 px-3 text-right tabular-nums">
                    {formatPercent(row.cache_hit_ratio, true)}
                  </td>
                  <td className="py-3 px-3">
                    {row.active_window_used_percent !== null ? (
                      <div className="space-y-1">
                        <div className="flex items-center justify-between text-[11px] tabular-nums">
                          <span style={{ color: 'var(--text-secondary)' }}>
                            {row.active_window_label ?? '滚动窗口'}
                          </span>
                          <span>{row.active_window_used_percent.toFixed(1)}%</span>
                        </div>
                        <InlineProgressBar percent={row.active_window_used_percent} heightPx={4} />
                      </div>
                    ) : (
                      <span className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                        未配置订阅规则
                      </span>
                    )}
                  </td>
                  <td
                    className="py-3 pl-3 text-right text-[12px] tabular-nums"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    <div>{formatRelativeTime(row.last_sync_at_ms)}</div>
                    {row.last_error && (
                      <div
                        className="text-[10.5px] truncate max-w-[160px]"
                        style={{ color: 'var(--accent-rose)' }}
                      >
                        {row.last_error}
                      </div>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
