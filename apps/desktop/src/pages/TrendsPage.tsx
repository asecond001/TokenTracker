import { useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { EChartBox } from '../components/EChartBox';
import { TimeRangeControl } from '../components/TimeRangeControl';
import { useChartTheme } from '../lib/chartTheme';
import {
  TOKEN_TYPE_COLORS,
  formatBucketInTz,
  formatCost,
  formatExactInt,
  formatPercent,
  formatTokens,
  providerAccentColor,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import {
  computeTimeRange,
  formatResolvedDateSpan,
  useWorkspaceStore,
} from '../stores/workspaceStore';
import type { AnalyticsFilter, StackDimension, TrendGranularity } from '../types/ipc';

type TrendMetric = 'tokens' | 'cost' | 'cache_hit' | 'requests';
type TrendChartStyle = 'bar' | 'line';

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
  { id: 'provider', label: '按模型提供商' },
  { id: 'model', label: '按模型' },
  { id: 'token_type', label: '按 Token 类型' },
];

const METRICS: Array<{ id: TrendMetric; label: string }> = [
  { id: 'tokens', label: 'Token 消耗' },
  { id: 'cost', label: 'API 等效费用' },
  { id: 'cache_hit', label: '缓存命中率' },
  { id: 'requests', label: '请求频次' },
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

function DeltaCell({ pct }: { pct: number | null | undefined }) {
  if (pct === null || pct === undefined || !Number.isFinite(pct)) {
    return <span style={{ color: 'var(--text-muted)' }}>—</span>;
  }
  return (
    <span
      style={{
        color: pct >= 0 ? 'var(--accent-emerald)' : 'var(--accent-rose)',
      }}
    >
      {pct >= 0 ? '+' : ''}
      {pct.toFixed(1)}%
    </span>
  );
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
  const [metric, setMetric] = useState<TrendMetric>('tokens');
  const [chartStyle, setChartStyle] = useState<TrendChartStyle>('bar');
  const [focusedKey, setFocusedKey] = useState<string | null>(null);

  const resolvedSpanLabel = formatResolvedDateSpan(timePreset, {
    start: customStartDate,
    end: customEndDate,
  });

  const { data, isLoading } = useQuery({
    queryKey: [
      'trends',
      timePreset,
      customStartDate,
      customEndDate,
      providerFilter,
      deviceFilter,
      tzOffsetMinutes,
      granularity,
      dimension,
    ],
    queryFn: () => {
      const range = computeTimeRange(timePreset, {
        start: customStartDate,
        end: customEndDate,
      });
      const filter: AnalyticsFilter = {
        start_ms: range.start_ms,
        end_ms: range.end_ms,
        provider_id: providerFilter === 'all' ? null : providerFilter,
        device_id: deviceFilter === 'all' ? null : deviceFilter,
        tz_offset_minutes: tzOffsetMinutes,
        all_time: range.all_time,
      };
      return ipc.getTrends({ filter, granularity, dimension });
    },
    placeholderData: keepPreviousData,
    refetchInterval: 15_000,
  });

  const isAllTime = timePreset === 'all';
  const effectiveGranularity: TrendGranularity = data?.effective_granularity ?? granularity;

  const nonZeroSeries = useMemo(
    () =>
      (data?.series ?? []).filter(
        (s) => s.total_tokens > 0 || s.total_cost_micro_usd > 0 || s.points.some((p) => p.requests > 0)
      ),
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

  const periodSummary = useMemo(() => {
    const allSeries = data?.series ?? [];
    const totalTokens = allSeries.reduce((acc, s) => acc + s.total_tokens, 0);
    const totalCostMicro = allSeries.reduce((acc, s) => acc + s.total_cost_micro_usd, 0);
    const buckets = data?.buckets ?? [];

    let dayCount = 1;
    if (isAllTime) {
      const dayKeys = new Set(
        buckets.map((ts) => formatBucketInTz(ts, 'daily', tzOffsetMinutes))
      );
      dayCount = Math.max(1, dayKeys.size);
    } else {
      const range = computeTimeRange(timePreset, {
        start: customStartDate,
        end: customEndDate,
      });
      dayCount = Math.max(1, Math.round((range.end_ms - range.start_ms + 1) / 86_400_000));
    }

    let peakBucketLabel = '—';
    let peakBucketTokens = 0;
    buckets.forEach((ts, idx) => {
      let sum = 0;
      for (const s of allSeries) {
        sum += s.points[idx]?.tokens ?? 0;
      }
      if (sum > peakBucketTokens) {
        peakBucketTokens = sum;
        peakBucketLabel = formatBucketInTz(ts, effectiveGranularity, tzOffsetMinutes);
      }
    });

    return {
      totalTokens,
      totalCostMicro,
      avgDailyTokens: Math.round(totalTokens / dayCount),
      avgDailyCostMicro: Math.round(totalCostMicro / dayCount),
      peakBucketLabel,
      peakBucketTokens,
    };
  }, [
    data,
    isAllTime,
    timePreset,
    customStartDate,
    customEndDate,
    effectiveGranularity,
    tzOffsetMinutes,
  ]);

  const chartOption = useMemo<EChartsOption>(() => {
    const buckets = data?.buckets ?? [];
    const xLabels = buckets.map((ts) =>
      formatBucketInTz(ts, effectiveGranularity, tzOffsetMinutes)
    );
    const isLine = chartStyle === 'line';
    const isCacheHit = metric === 'cache_hit';

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
        valueFormatter: (value) => {
          const num = typeof value === 'number' ? value : Number(value ?? 0);
          if (metric === 'cost') {
            return formatCost(num, currency, usdToCnyRate);
          }
          if (metric === 'cache_hit') {
            return `${num.toFixed(1)}%`;
          }
          if (metric === 'requests') {
            return `${formatExactInt(num)} 次`;
          }
          return formatTokens(num);
        },
      },
      grid: { top: 20, right: 16, bottom: 26, left: 60 },
      xAxis: {
        type: 'category',
        data: xLabels,
        axisLine: { lineStyle: { color: chartTheme.axisLineColor } },
        axisLabel: { color: chartTheme.textMuted, fontSize: 11 },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        min: isCacheHit ? 0 : undefined,
        max: isCacheHit ? 100 : undefined,
        splitLine: { lineStyle: { color: chartTheme.splitLineColor, type: 'dashed' } },
        axisLabel: {
          color: chartTheme.textMuted,
          fontSize: 11,
          formatter: (v: number) => {
            if (metric === 'cost') {
              return formatCost(v, currency, usdToCnyRate);
            }
            if (metric === 'cache_hit') {
              return `${v.toFixed(0)}%`;
            }
            if (metric === 'requests') {
              return formatExactInt(v);
            }
            return formatTokens(v);
          },
        },
      },
      series: visibleSeries.map((s, idx) => {
        const color = seriesColorMap.get(s.key) ?? resolveSeriesColor(dimension, s.key, idx);
        const values = s.points.map((p) => {
          if (metric === 'cost') {
            return p.cost_micro_usd;
          }
          if (metric === 'cache_hit') {
            return Number(((p.cache_hit_ratio ?? 0) * 100).toFixed(2));
          }
          if (metric === 'requests') {
            return p.requests;
          }
          return p.tokens;
        });

        if (isLine) {
          return {
            id: `${dimension}-${metric}-${s.key}`,
            name: s.label,
            type: 'line' as const,
            smooth: 0.25,
            showSymbol: buckets.length <= 16,
            symbolSize: 5,
            universalTransition: true,
            emphasis: { focus: 'series' as const },
            lineStyle: { width: 2, color },
            itemStyle: { color },
            data: values,
          };
        }

        return {
          id: `${dimension}-${metric}-${s.key}`,
          name: s.label,
          type: 'bar' as const,
          stack: isCacheHit ? undefined : 'total',
          barMaxWidth: isCacheHit ? 18 : 28,
          universalTransition: true,
          emphasis: { focus: 'series' as const },
          itemStyle: { color },
          data: values,
        };
      }),
    };
  }, [
    data,
    effectiveGranularity,
    tzOffsetMinutes,
    chartStyle,
    metric,
    visibleSeries,
    chartTheme,
    dimension,
    seriesColorMap,
    currency,
    usdToCnyRate,
  ]);

  const handleMetricChange = (nextMetric: TrendMetric) => {
    setMetric(nextMetric);
    if (nextMetric === 'cache_hit' && dimension === 'token_type') {
      setDimension('provider');
      setFocusedKey(null);
    }
  };

  const handleDimensionChange = (nextDimension: StackDimension) => {
    setDimension(nextDimension);
    setFocusedKey(null);
    if (nextDimension === 'token_type' && metric === 'cache_hit') {
      setMetric('tokens');
    }
  };

  return (
    <div className="space-y-6">
      {/* Control Strip */}
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div className="flex flex-wrap items-center gap-2">
          <h1 className="text-[16px] font-semibold tracking-tight">趋势分析</h1>
          {data?.effective_granularity && (
            <span
              className="px-1.5 py-0.5 text-[10.5px] rounded-md tabular-nums"
              style={{
                backgroundColor: 'var(--bg-hover)',
                border: '1px solid var(--border-hairline)',
                color: 'var(--text-secondary)',
              }}
            >
              {granularity === 'auto'
                ? `自动: ${GRANULARITY_LABELS[effectiveGranularity]}`
                : GRANULARITY_LABELS[effectiveGranularity]}
            </span>
          )}
          <span
            data-testid="trends-date-span"
            className="px-2 py-0.5 text-[11px] rounded-md tabular-nums"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              border: '1px solid var(--border-hairline)',
              color: 'var(--text-secondary)',
            }}
          >
            {resolvedSpanLabel}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <TimeRangeControl />

          {/* Metric Switcher */}
          <div
            className="seg-group"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              borderColor: 'var(--border-hairline)',
            }}
          >
            {METRICS.map((m) => {
              const active = metric === m.id;
              return (
                <button
                  key={m.id}
                  type="button"
                  data-active={active}
                  onClick={() => handleMetricChange(m.id)}
                  className="seg-pill px-2.5 py-1 text-[11.5px] cursor-pointer whitespace-nowrap"
                  style={{
                    color: active ? 'var(--accent-tonal-text)' : 'var(--text-secondary)',
                    fontWeight: active ? 500 : 400,
                  }}
                >
                  {m.label}
                </button>
              );
            })}
          </div>

          {/* Granularity Switcher */}
          <div
            className="seg-group"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              borderColor: 'var(--border-hairline)',
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
                  className="seg-pill px-2.5 py-1 text-[11.5px] cursor-pointer whitespace-nowrap"
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
            className="seg-group"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              borderColor: 'var(--border-hairline)',
            }}
          >
            {DIMENSIONS.map((d) => {
              const active = dimension === d.id;
              const disabledForCacheHit = metric === 'cache_hit' && d.id === 'token_type';
              return (
                <button
                  key={d.id}
                  type="button"
                  data-active={active}
                  disabled={disabledForCacheHit}
                  onClick={() => handleDimensionChange(d.id)}
                  className="seg-pill px-2.5 py-1 text-[11.5px] cursor-pointer whitespace-nowrap disabled:opacity-45 disabled:cursor-not-allowed"
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

          {/* Bar / Line Toggle */}
          <div
            className="seg-group"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              borderColor: 'var(--border-hairline)',
            }}
          >
            {(
              [
                { id: 'bar', label: '柱状' },
                { id: 'line', label: '折线' },
              ] as const
            ).map((kind) => {
              const active = chartStyle === kind.id;
              return (
                <button
                  key={kind.id}
                  type="button"
                  data-active={active}
                  onClick={() => setChartStyle(kind.id)}
                  className="seg-pill px-2.5 py-1 text-[11.5px] cursor-pointer whitespace-nowrap"
                  style={{
                    color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                    fontWeight: active ? 500 : 400,
                  }}
                >
                  {kind.label}
                </button>
              );
            })}
          </div>
        </div>
      </section>

      {/* Period Summary Strip */}
      <section className="surface-tonal-band px-5 py-3.5 grid grid-cols-2 lg:grid-cols-4 gap-y-3">
        <div className="lg:pr-5">
          <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
            区间总 Token
          </div>
          <div className="mt-1 text-[20px] font-semibold tabular-nums">
            {formatTokens(periodSummary.totalTokens)}
          </div>
        </div>
        <div className="lg:px-5" style={{ borderLeft: '1px solid var(--border-hairline)' }}>
          <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
            区间等效费用 ({currency})
          </div>
          <div className="mt-1 text-[20px] font-semibold tabular-nums">
            {formatCost(periodSummary.totalCostMicro, currency, usdToCnyRate)}
          </div>
        </div>
        <div className="lg:px-5" style={{ borderLeft: '1px solid var(--border-hairline)' }}>
          <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
            日均 Token / 费用
          </div>
          <div className="mt-1 text-[18px] font-semibold tabular-nums">
            {formatTokens(periodSummary.avgDailyTokens)}{' '}
            <span className="text-[12.5px] font-normal" style={{ color: 'var(--text-muted)' }}>
              / {formatCost(periodSummary.avgDailyCostMicro, currency, usdToCnyRate)}
            </span>
          </div>
        </div>
        <div className="lg:pl-5" style={{ borderLeft: '1px solid var(--border-hairline)' }}>
          <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
            峰值时段
          </div>
          <div className="mt-1 text-[16px] font-semibold tabular-nums truncate">
            {periodSummary.peakBucketLabel}{' '}
            <span className="text-[12px] font-normal" style={{ color: 'var(--text-muted)' }}>
              {periodSummary.peakBucketTokens > 0 ? formatTokens(periodSummary.peakBucketTokens) : ''}
            </span>
          </div>
        </div>
      </section>

      {/* Interactive Legend Bar */}
      {nonZeroSeries.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => setFocusedKey(null)}
            className="btn-interactive px-2 py-0.5 text-[11.5px] rounded-lg cursor-pointer"
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
                className="btn-interactive inline-flex items-center gap-1.5 px-2 py-0.5 text-[11.5px] rounded-lg cursor-pointer tabular-nums"
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
                <span style={{ color: 'var(--text-muted)' }}>
                  {metric === 'cost'
                    ? formatCost(s.total_cost_micro_usd, currency, usdToCnyRate)
                    : formatTokens(s.total_tokens)}
                </span>
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
            当前区间等待事件数据
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
        </div>
        <div className="overflow-x-auto">
          <table className="workspace-table w-full border-collapse text-left">
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
                {!isAllTime && <th className="py-2 px-3 text-right">Token 环比</th>}
                <th className="py-2 px-3 text-right">
                  {isAllTime ? `累计折算费用 (${currency})` : `当前折算费用 (${currency})`}
                </th>
                {!isAllTime && <th className="py-2 px-3 text-right">费用环比</th>}
                <th className="py-2 px-3 text-right">缓存命中率</th>
                <th className="py-2 pl-3 pr-5 text-right">请求 / 步数</th>
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
                        <DeltaCell pct={row.delta_pct} />
                      </td>
                    )}
                    <td className="py-2.5 px-3 text-right tabular-nums">
                      {formatCost(row.current_cost_micro_usd, currency, usdToCnyRate)}
                    </td>
                    {!isAllTime && (
                      <td className="py-2.5 px-3 text-right tabular-nums">
                        <DeltaCell pct={row.cost_delta_pct} />
                      </td>
                    )}
                    <td className="py-2.5 px-3 text-right tabular-nums">
                      {row.cache_hit_ratio !== undefined
                        ? formatPercent(row.cache_hit_ratio, true)
                        : '—'}
                    </td>
                    <td
                      className="py-2.5 pl-3 pr-5 text-right tabular-nums"
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
