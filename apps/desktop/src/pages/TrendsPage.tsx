import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { EChartBox } from '../components/EChartBox';
import { TimeRangeControl } from '../components/TimeRangeControl';
import { useChartTheme } from '../lib/chartTheme';
import {
  TOKEN_TYPE_COLORS,
  formatCost,
  formatExactInt,
  formatTokens,
  providerAccentColor,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { computeTimeRange, useWorkspaceStore } from '../stores/workspaceStore';
import type { AnalyticsFilter, StackDimension, TrendGranularity } from '../types/ipc';

const GRANULARITIES: Array<{ id: TrendGranularity; label: string }> = [
  { id: 'auto', label: '自动' },
  { id: 'fifteen_minutes', label: '15m' },
  { id: 'hourly', label: '小时' },
  { id: 'daily', label: '天' },
  { id: 'weekly', label: '周' },
];

const GRANULARITY_LABELS: Record<TrendGranularity, string> = {
  auto: '自动',
  fifteen_minutes: '15 分钟',
  hourly: '小时',
  daily: '天',
  weekly: '周',
};

const DIMENSIONS: Array<{ id: StackDimension; label: string }> = [
  { id: 'provider', label: '按 Provider' },
  { id: 'model', label: '按模型' },
  { id: 'token_type', label: '按 Token 类型' },
];

const PALETTE = ['#0ea5e9', '#10b981', '#f59e0b', '#f43f5e', '#a855f7', '#14b8a6', '#6366f1'];

function resolveSeriesColor(dimension: StackDimension, key: string, index: number): string {
  if (dimension === 'provider') {
    return providerAccentColor(key);
  }
  if (dimension === 'token_type') {
    return TOKEN_TYPE_COLORS[key] ?? PALETTE[index % PALETTE.length];
  }
  return PALETTE[index % PALETTE.length];
}

function formatTrendBucket(ts: number, effective: TrendGranularity): string {
  const d = new Date(ts);
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  if (effective === 'fifteen_minutes' || effective === 'hourly') {
    return `${mm}-${dd} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  }
  return `${d.getFullYear()}-${mm}-${dd}`;
}

export function TrendsPage() {
  const {
    timePreset,
    customStartDate,
    customEndDate,
    providerFilter,
    deviceFilter,
    currency,
    usdToCnyRate,
    tzOffsetMinutes,
  } = useWorkspaceStore();
  const chartTheme = useChartTheme();
  const [granularity, setGranularity] = useState<TrendGranularity>('auto');
  const [dimension, setDimension] = useState<StackDimension>('provider');
  const [focusedKey, setFocusedKey] = useState<string | null>(null);

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
    queryKey: ['trends', filter, granularity, dimension],
    queryFn: () => ipc.getTrends({ filter, granularity, dimension }),
    refetchInterval: 15_000,
  });

  const isAllTime = timePreset === 'all';
  const effectiveGranularity: TrendGranularity = data?.effective_granularity ?? granularity;

  const nonZeroSeries = useMemo(
    () => (data?.series ?? []).filter((s) => s.total_tokens > 0),
    [data]
  );

  const seriesColorMap = useMemo(() => {
    const map = new Map<string, string>();
    nonZeroSeries.forEach((s, idx) => {
      map.set(s.key, resolveSeriesColor(dimension, s.key, idx));
    });
    return map;
  }, [nonZeroSeries, dimension]);

  const activeFocusedKey = useMemo(() => {
    if (!focusedKey) return null;
    return nonZeroSeries.some((s) => s.key === focusedKey) ? focusedKey : null;
  }, [nonZeroSeries, focusedKey]);

  const visibleSeries = useMemo(() => {
    if (!activeFocusedKey) return nonZeroSeries;
    return nonZeroSeries.filter((s) => s.key === activeFocusedKey);
  }, [nonZeroSeries, activeFocusedKey]);

  const chartOption = useMemo<EChartsOption>(() => {
    const buckets = data?.buckets ?? [];
    const xLabels = buckets.map((ts) => formatTrendBucket(ts, effectiveGranularity));

    return {
      backgroundColor: 'transparent',
      animationDuration: 280,
      animationDurationUpdate: 260,
      tooltip: {
        trigger: 'axis',
        backgroundColor: chartTheme.tooltipBg,
        borderColor: chartTheme.tooltipBorder,
        textStyle: { color: chartTheme.tooltipText, fontSize: 12 },
      },
      grid: { top: 20, right: 16, bottom: 26, left: 52 },
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
          formatter: (v: number) => formatTokens(v),
        },
      },
      series: visibleSeries.map((s, idx) => ({
        id: `${dimension}-${s.key}`,
        name: s.label,
        type: 'bar',
        stack: 'total',
        universalTransition: true,
        emphasis: { focus: 'series' },
        itemStyle: {
          color: seriesColorMap.get(s.key) ?? resolveSeriesColor(dimension, s.key, idx),
        },
        data: s.points.map((p) => p.tokens),
      })),
    };
  }, [data, effectiveGranularity, visibleSeries, chartTheme, dimension, seriesColorMap]);

  return (
    <div className="space-y-7">
      {/* Control Strip */}
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-[16px] font-semibold tracking-tight">多粒度用量趋势</h1>
            {data?.effective_granularity && (
              <span
                className="px-1.5 py-0.5 text-[10.5px] rounded tabular-nums"
                style={{
                  backgroundColor: 'var(--bg-hover)',
                  border: '1px solid var(--border-hairline)',
                  color: 'var(--text-secondary)',
                }}
              >
                {granularity === 'auto'
                  ? `自动粒度: ${GRANULARITY_LABELS[effectiveGranularity]}`
                  : `粒度: ${GRANULARITY_LABELS[effectiveGranularity]}`}
              </span>
            )}
          </div>
          <p className="text-[12px] mt-0.5" style={{ color: 'var(--text-muted)' }}>
            支持自动对齐全部历史到 15 分钟实时桶，点击图例或对比行可单独聚焦某一项
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2.5">
          <TimeRangeControl />

          {/* Granularity Switcher */}
          <div
            className="flex items-center p-0.5 rounded"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              border: '1px solid var(--border-hairline)',
            }}
          >
            {GRANULARITIES.map((g) => {
              const active = granularity === g.id;
              return (
                <button
                  key={g.id}
                  type="button"
                  data-active={active}
                  onClick={() => setGranularity(g.id)}
                  className="seg-pill px-2.5 py-1 text-[11.5px] rounded-sm cursor-pointer"
                  style={{
                    color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                    fontWeight: active ? 500 : 400,
                  }}
                >
                  {g.label}
                </button>
              );
            })}
          </div>

          {/* Stack Dimension Switcher */}
          <div
            className="flex items-center p-0.5 rounded"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              border: '1px solid var(--border-hairline)',
            }}
          >
            {DIMENSIONS.map((d) => {
              const active = dimension === d.id;
              return (
                <button
                  key={d.id}
                  type="button"
                  data-active={active}
                  onClick={() => {
                    setDimension(d.id);
                    setFocusedKey(null);
                  }}
                  className="seg-pill px-2.5 py-1 text-[11.5px] rounded-sm cursor-pointer"
                  style={{
                    color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                    fontWeight: active ? 500 : 400,
                  }}
                >
                  {d.label}
                </button>
              );
            })}
          </div>
        </div>
      </section>

      {/* Interactive Legend Bar */}
      {nonZeroSeries.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => setFocusedKey(null)}
            className="btn-interactive px-2 py-0.5 text-[11.5px] rounded cursor-pointer"
            style={{
              backgroundColor: activeFocusedKey === null ? 'var(--bg-hover)' : 'transparent',
              border: '1px solid var(--border-hairline)',
              color: activeFocusedKey === null ? 'var(--text-primary)' : 'var(--text-secondary)',
            }}
          >
            显示全部
          </button>
          {nonZeroSeries.map((s, idx) => {
            const active = activeFocusedKey === s.key;
            const color =
              seriesColorMap.get(s.key) ?? resolveSeriesColor(dimension, s.key, idx);
            return (
              <button
                key={s.key}
                type="button"
                onClick={() => setFocusedKey(active ? null : s.key)}
                className="btn-interactive inline-flex items-center gap-1.5 px-2 py-0.5 text-[11.5px] rounded cursor-pointer tabular-nums"
                style={{
                  backgroundColor: active ? 'var(--bg-hover)' : 'transparent',
                  border: '1px solid var(--border-hairline)',
                  color:
                    active || activeFocusedKey === null ? 'var(--text-primary)' : 'var(--text-muted)',
                }}
              >
                <span
                  className="inline-block w-2 h-2 rounded-full"
                  style={{ backgroundColor: color }}
                />
                <span>{s.label}</span>
                <span style={{ color: 'var(--text-muted)' }}>{formatTokens(s.total_tokens)}</span>
              </button>
            );
          })}
        </div>
      )}

      {/* Full-Width Chart */}
      <section
        className="pb-6"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        {isLoading && !data ? (
          <div className="h-[280px] flex items-center justify-center" style={{ color: 'var(--text-muted)' }}>
            加载趋势数据…
          </div>
        ) : (data?.buckets.length ?? 0) === 0 ? (
          <div className="h-[260px] flex items-center justify-center" style={{ color: 'var(--text-muted)' }}>
            选定区间内暂无事件数据
          </div>
        ) : (
          <EChartBox option={chartOption} height={290} />
        )}
      </section>

      {/* Period Delta / Summary Table */}
      <section>
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-[13.5px] font-semibold">
            {isAllTime ? '全周期累计分布汇总' : '区间增量变化对比'}
          </h2>
          <span className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
            {isAllTime
              ? '全部历史周期模式下展示全量汇总，不计算上周期环比'
              : '对比当前选中窗口与前一等长周期的变化幅度'}
          </span>
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
                <th className="py-2 pr-4">维度项</th>
                <th className="py-2 px-3 text-right">
                  {isAllTime ? '累计 Token' : '当前周期 Token'}
                </th>
                {!isAllTime && <th className="py-2 px-3 text-right">上一周期 Token</th>}
                {!isAllTime && <th className="py-2 px-3 text-right">环比变化</th>}
                <th className="py-2 px-3 text-right">
                  {isAllTime ? `累计折算费用 (${currency})` : `当前折算费用 (${currency})`}
                </th>
                <th className="py-2 pl-3 text-right">请求 / 步数</th>
              </tr>
            </thead>
            <tbody>
              {(data?.comparisons ?? []).map((row, idx) => {
                const isSelected = activeFocusedKey === row.key;
                const dotColor =
                  seriesColorMap.get(row.key) ?? resolveSeriesColor(dimension, row.key, idx);
                return (
                  <tr
                    key={row.key}
                    data-selected={isSelected ? 'true' : undefined}
                    onClick={() => setFocusedKey(isSelected ? null : row.key)}
                    className="row-interactive cursor-pointer"
                    style={{
                      borderBottom: '1px solid var(--border-hairline)',
                    }}
                  >
                    <td className="py-2.5 pr-4 font-medium">
                      <div className="flex items-center gap-2">
                        <span
                          className="inline-block w-2 h-2 rounded-full shrink-0"
                          style={{ backgroundColor: dotColor }}
                        />
                        <span>{row.label}</span>
                      </div>
                    </td>
                    <td className="py-2.5 px-3 text-right tabular-nums">
                      {formatTokens(row.current_tokens)}
                    </td>
                    {!isAllTime && (
                      <td
                        className="py-2.5 px-3 text-right tabular-nums"
                        style={{ color: 'var(--text-secondary)' }}
                      >
                        {formatTokens(row.previous_tokens)}
                      </td>
                    )}
                    {!isAllTime && (
                      <td className="py-2.5 px-3 text-right tabular-nums">
                        {row.delta_pct === null ? (
                          <span style={{ color: 'var(--text-muted)' }}>—</span>
                        ) : (
                          <span
                            style={{
                              color:
                                row.delta_pct >= 0
                                  ? 'var(--accent-emerald)'
                                  : 'var(--accent-rose)',
                            }}
                          >
                            {row.delta_pct >= 0 ? '+' : ''}
                            {row.delta_pct.toFixed(1)}%
                          </span>
                        )}
                      </td>
                    )}
                    <td className="py-2.5 px-3 text-right tabular-nums">
                      {formatCost(row.current_cost_micro_usd, currency, usdToCnyRate)}
                    </td>
                    <td
                      className="py-2.5 pl-3 text-right tabular-nums"
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      {formatExactInt(row.requests)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
