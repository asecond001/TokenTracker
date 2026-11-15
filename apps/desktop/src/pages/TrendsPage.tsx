import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { EChartBox } from '../components/EChartBox';
import { formatCost, formatTokens } from '../lib/format';
import { ipc } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type { StackDimension, TrendGranularity } from '../types/ipc';

const GRANULARITIES: Array<{ id: TrendGranularity; label: string }> = [
  { id: 'fifteen_minutes', label: '15 分钟' },
  { id: 'hourly', label: '小时' },
  { id: 'daily', label: '天' },
  { id: 'weekly', label: '周' },
];

const DIMENSIONS: Array<{ id: StackDimension; label: string }> = [
  { id: 'provider', label: '按 Provider' },
  { id: 'model', label: '按模型' },
  { id: 'token_type', label: '按 Token 类型' },
];

const PALETTE = ['#6366f1', '#10b981', '#38bdf8', '#f59e0b', '#f43f5e', '#a855f7', '#14b8a6'];

export function TrendsPage() {
  const { timePreset, customStartDate, customEndDate, providerFilter, currency, usdToCnyRate, getFilter } =
    useWorkspaceStore();
  const [granularity, setGranularity] = useState<TrendGranularity>('hourly');
  const [dimension, setDimension] = useState<StackDimension>('token_type');
  const [focusedKey, setFocusedKey] = useState<string | null>(null);

  const filter = useMemo(
    () => getFilter(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [timePreset, customStartDate, customEndDate, providerFilter]
  );

  const { data, isLoading } = useQuery({
    queryKey: ['trends', filter, granularity, dimension],
    queryFn: () => ipc.getTrends({ filter, granularity, dimension }),
    refetchInterval: 15_000,
  });

  const visibleSeries = useMemo(() => {
    const all = data?.series ?? [];
    if (!focusedKey) return all;
    const filtered = all.filter((s) => s.key === focusedKey);
    return filtered.length > 0 ? filtered : all;
  }, [data, focusedKey]);

  const chartOption = useMemo<EChartsOption>(() => {
    const buckets = data?.buckets ?? [];
    const xLabels = buckets.map((ts) => {
      const d = new Date(ts);
      if (granularity === 'fifteen_minutes' || granularity === 'hourly') {
        return `${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
      }
      return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
    });

    return {
      backgroundColor: 'transparent',
      tooltip: {
        trigger: 'axis',
        backgroundColor: '#161922',
        borderColor: '#2d3343',
        textStyle: { color: '#eceff4', fontSize: 12 },
      },
      grid: { top: 20, right: 16, bottom: 26, left: 52 },
      xAxis: {
        type: 'category',
        data: xLabels,
        axisLine: { lineStyle: { color: '#2b3040' } },
        axisLabel: { color: '#6b7280', fontSize: 11 },
      },
      yAxis: {
        type: 'value',
        splitLine: { lineStyle: { color: '#1f232e', type: 'dashed' } },
        axisLabel: {
          color: '#6b7280',
          fontSize: 11,
          formatter: (v: number) => formatTokens(v),
        },
      },
      series: visibleSeries.map((s, idx) => ({
        name: s.label,
        type: 'bar',
        stack: 'total',
        emphasis: { focus: 'series' },
        itemStyle: { color: PALETTE[idx % PALETTE.length] },
        data: s.points.map((p) => p.tokens),
      })),
    };
  }, [data, granularity, visibleSeries]);

  return (
    <div className="space-y-7">
      {/* Control Strip */}
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div>
          <h1 className="text-[16px] font-semibold tracking-tight">多粒度用量趋势</h1>
          <p className="text-[12px]" style={{ color: 'var(--text-muted)' }}>
            支持 15 分钟实时到周级跨度，点击下方图例或对比行可单独聚焦某一项
          </p>
        </div>

        <div className="flex items-center gap-3">
          {/* Granularity Switcher */}
          <div
            className="flex items-center p-0.5 rounded"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            {GRANULARITIES.map((g) => (
              <button
                key={g.id}
                type="button"
                onClick={() => setGranularity(g.id)}
                className="px-2.5 py-1 text-[11.5px] rounded-sm cursor-pointer"
                style={{
                  backgroundColor: granularity === g.id ? 'var(--bg-hover)' : 'transparent',
                  color: granularity === g.id ? 'var(--text-primary)' : 'var(--text-secondary)',
                }}
              >
                {g.label}
              </button>
            ))}
          </div>

          {/* Stack Dimension Switcher */}
          <div
            className="flex items-center p-0.5 rounded"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            {DIMENSIONS.map((d) => (
              <button
                key={d.id}
                type="button"
                onClick={() => {
                  setDimension(d.id);
                  setFocusedKey(null);
                }}
                className="px-2.5 py-1 text-[11.5px] rounded-sm cursor-pointer"
                style={{
                  backgroundColor: dimension === d.id ? 'var(--bg-hover)' : 'transparent',
                  color: dimension === d.id ? 'var(--text-primary)' : 'var(--text-secondary)',
                }}
              >
                {d.label}
              </button>
            ))}
          </div>
        </div>
      </section>

      {/* Interactive Legend Bar */}
      {(data?.series.length ?? 0) > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => setFocusedKey(null)}
            className="px-2 py-0.5 text-[11.5px] rounded cursor-pointer"
            style={{
              backgroundColor: focusedKey === null ? 'var(--bg-hover)' : 'transparent',
              border: '1px solid var(--border-subtle)',
              color: focusedKey === null ? 'var(--text-primary)' : 'var(--text-secondary)',
            }}
          >
            显示全部
          </button>
          {(data?.series ?? []).map((s, idx) => {
            const active = focusedKey === s.key;
            return (
              <button
                key={s.key}
                type="button"
                onClick={() => setFocusedKey(active ? null : s.key)}
                className="inline-flex items-center gap-1.5 px-2 py-0.5 text-[11.5px] rounded cursor-pointer tabular-nums"
                style={{
                  backgroundColor: active ? 'var(--bg-hover)' : 'transparent',
                  border: '1px solid var(--border-subtle)',
                  color: active || focusedKey === null ? 'var(--text-primary)' : 'var(--text-muted)',
                }}
              >
                <span
                  className="inline-block w-2 h-2 rounded-full"
                  style={{ backgroundColor: PALETTE[idx % PALETTE.length] }}
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
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        {isLoading ? (
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

      {/* Period Delta Comparison Table */}
      <section>
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-[13.5px] font-semibold">区间增量变化对比</h2>
          <span className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
            对比当前选中窗口与前一等长周期的变化幅度
          </span>
        </div>
        <div className="overflow-x-auto">
          <table className="w-full border-collapse text-left">
            <thead>
              <tr
                className="text-[11px] font-medium uppercase tracking-wider"
                style={{
                  color: 'var(--text-muted)',
                  borderBottom: '1px solid var(--border-subtle)',
                }}
              >
                <th className="py-2 pr-4">维度项</th>
                <th className="py-2 px-3 text-right">当前周期 Token</th>
                <th className="py-2 px-3 text-right">上一周期 Token</th>
                <th className="py-2 px-3 text-right">环比变化</th>
                <th className="py-2 px-3 text-right">当前折算费用</th>
                <th className="py-2 pl-3 text-right">请求 / 步数</th>
              </tr>
            </thead>
            <tbody>
              {(data?.comparisons ?? []).map((row) => {
                const isSelected = focusedKey === row.key;
                return (
                  <tr
                    key={row.key}
                    onClick={() => setFocusedKey(isSelected ? null : row.key)}
                    className="cursor-pointer transition-colors hover:bg-white/[0.02]"
                    style={{
                      borderBottom: '1px solid var(--border-subtle)',
                      backgroundColor: isSelected ? 'var(--bg-hover)' : 'transparent',
                    }}
                  >
                    <td className="py-2.5 pr-4 font-medium">{row.label}</td>
                    <td className="py-2.5 px-3 text-right tabular-nums">
                      {formatTokens(row.current_tokens)}
                    </td>
                    <td className="py-2.5 px-3 text-right tabular-nums" style={{ color: 'var(--text-secondary)' }}>
                      {formatTokens(row.previous_tokens)}
                    </td>
                    <td className="py-2.5 px-3 text-right tabular-nums">
                      {row.delta_pct === null ? (
                        <span style={{ color: 'var(--text-muted)' }}>—</span>
                      ) : (
                        <span
                          style={{
                            color:
                              row.delta_pct >= 0 ? 'var(--accent-emerald)' : 'var(--accent-rose)',
                          }}
                        >
                          {row.delta_pct >= 0 ? '+' : ''}
                          {row.delta_pct.toFixed(1)}%
                        </span>
                      )}
                    </td>
                    <td className="py-2.5 px-3 text-right tabular-nums">
                      {formatCost(row.current_cost_micro_usd, currency, usdToCnyRate)}
                    </td>
                    <td className="py-2.5 pl-3 text-right tabular-nums" style={{ color: 'var(--text-secondary)' }}>
                      {row.requests.toLocaleString('en-US')}
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
