import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { EChartBox } from '../components/EChartBox';
import { InlineProgressBar } from '../components/InlineProgressBar';
import {
  formatCost,
  formatPercent,
  formatRelativeTime,
  formatTokens,
  formatTps,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';

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

export function OverviewPage() {
  const { timePreset, customStartDate, customEndDate, providerFilter, currency, usdToCnyRate, getFilter, setActivePage } =
    useWorkspaceStore();
  const [chartType, setChartType] = useState<'area' | 'bar'>('area');

  const filter = useMemo(
    () => getFilter(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [timePreset, customStartDate, customEndDate, providerFilter]
  );

  const { data, isLoading } = useQuery({
    queryKey: ['overview', filter],
    queryFn: () => ipc.getOverview(filter),
    refetchInterval: 10_000,
  });

  const chartOption = useMemo<EChartsOption>(() => {
    const series = data?.series ?? [];
    const xLabels = series.map((pt) => {
      const d = new Date(pt.bucket_start_ms);
      if (data?.series_granularity === 'hourly') {
        return `${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${String(d.getHours()).padStart(2, '0')}:00`;
      }
      return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
    });

    const isBar = chartType === 'bar';

    return {
      backgroundColor: 'transparent',
      tooltip: {
        trigger: 'axis',
        backgroundColor: '#161922',
        borderColor: '#2d3343',
        textStyle: { color: '#eceff4', fontSize: 12 },
      },
      legend: {
        top: 0,
        right: 0,
        textStyle: { color: '#98a2b3', fontSize: 11 },
        itemWidth: 10,
        itemHeight: 6,
      },
      grid: { top: 32, right: 12, bottom: 24, left: 48, containLabel: false },
      xAxis: {
        type: 'category',
        data: xLabels,
        axisLine: { lineStyle: { color: '#2b3040' } },
        axisLabel: { color: '#6b7280', fontSize: 11 },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        splitLine: { lineStyle: { color: '#1f232e', type: 'dashed' } },
        axisLabel: {
          color: '#6b7280',
          fontSize: 11,
          formatter: (val: number) => formatTokens(val),
        },
      },
      series: [
        {
          name: 'Input',
          type: isBar ? 'bar' : 'line',
          stack: 'tokens',
          smooth: !isBar,
          showSymbol: false,
          areaStyle: isBar ? undefined : { opacity: 0.28 },
          itemStyle: { color: '#6366f1' },
          data: series.map((p) => p.input_tokens),
        },
        {
          name: 'Output',
          type: isBar ? 'bar' : 'line',
          stack: 'tokens',
          smooth: !isBar,
          showSymbol: false,
          areaStyle: isBar ? undefined : { opacity: 0.28 },
          itemStyle: { color: '#10b981' },
          data: series.map((p) => p.output_tokens + p.reasoning_tokens),
        },
        {
          name: 'Cache Read',
          type: isBar ? 'bar' : 'line',
          stack: 'tokens',
          smooth: !isBar,
          showSymbol: false,
          areaStyle: isBar ? undefined : { opacity: 0.22 },
          itemStyle: { color: '#38bdf8' },
          data: series.map((p) => p.cache_read_tokens),
        },
        {
          name: 'Cache Write',
          type: isBar ? 'bar' : 'line',
          stack: 'tokens',
          smooth: !isBar,
          showSymbol: false,
          areaStyle: isBar ? undefined : { opacity: 0.22 },
          itemStyle: { color: '#f59e0b' },
          data: series.map((p) => p.cache_write_tokens),
        },
      ],
    };
  }, [data, chartType]);

  return (
    <div className="space-y-7">
      {/* Top Flat Single-Row 4-Metric Strip (Separated by 1px Vertical Hairlines — No Bento Cards) */}
      <section
        className="grid grid-cols-4 pb-6"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div className="pr-5">
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            总 Token 消耗
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[24px] font-semibold tracking-tight tabular-nums">
              {isLoading ? '…' : formatTokens(data?.total_tokens ?? 0)}
            </span>
            <DeltaBadge pct={data?.tokens_delta_pct ?? null} />
          </div>
          <div className="mt-1 text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            {(data?.total_requests ?? 0).toLocaleString('en-US')} 次请求 / 步
          </div>
        </div>

        <div
          className="px-5"
          style={{ borderLeft: '1px solid var(--border-subtle)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            API 等效成本 ({currency})
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[24px] font-semibold tracking-tight tabular-nums">
              {isLoading
                ? '…'
                : formatCost(data?.total_cost_micro_usd ?? 0, currency, usdToCnyRate)}
            </span>
            <DeltaBadge pct={data?.cost_delta_pct ?? null} />
          </div>
          <div className="mt-1 text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            按官方公开百万 Token 定价折算
          </div>
        </div>

        <div
          className="px-5"
          style={{ borderLeft: '1px solid var(--border-subtle)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            平均缓存命中率
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[24px] font-semibold tracking-tight tabular-nums">
              {isLoading ? '…' : formatPercent(data?.cache_hit_ratio ?? 0, true)}
            </span>
            <DeltaBadge pct={data?.cache_delta_pct ?? null} />
          </div>
          <div className="mt-1 text-[11px]" style={{ color: 'var(--text-muted)' }}>
            Cache Read / (Uncached + Cache)
          </div>
        </div>

        <div
          className="pl-5"
          style={{ borderLeft: '1px solid var(--border-subtle)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            当前 / 今日峰值速度
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[24px] font-semibold tracking-tight tabular-nums">
              {isLoading
                ? '…'
                : formatTps(data?.latest_output_tps ?? data?.avg_output_tps ?? null)}
            </span>
            <DeltaBadge pct={data?.tps_delta_pct ?? null} />
          </div>
          <div className="mt-1 text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            峰值 {formatTps(data?.peak_output_tps ?? 0)}
          </div>
        </div>
      </section>

      {/* Mid Full-Width Borderless Consumption Trend Chart */}
      <section
        className="pb-6"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div className="flex items-center justify-between mb-3">
          <div>
            <h2 className="text-[13.5px] font-semibold">Token 构成趋势</h2>
            <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              {data?.series_granularity === 'hourly' ? '按小时聚合桶' : '按天聚合桶'} ·
              展示 Input、Output、Cache Read 与 Cache Write 分布
            </p>
          </div>
          <div
            className="flex items-center p-0.5 rounded"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            {(['area', 'bar'] as const).map((kind) => (
              <button
                key={kind}
                type="button"
                onClick={() => setChartType(kind)}
                className="px-2.5 py-0.5 text-[11.5px] rounded-sm cursor-pointer"
                style={{
                  backgroundColor: chartType === kind ? 'var(--bg-hover)' : 'transparent',
                  color: chartType === kind ? 'var(--text-primary)' : 'var(--text-secondary)',
                }}
              >
                {kind === 'area' ? '面积图' : '柱状图'}
              </button>
            ))}
          </div>
        </div>

        {(data?.series.length ?? 0) === 0 ? (
          <div
            className="h-[240px] flex flex-col items-center justify-center text-[12.5px] gap-2"
            style={{ color: 'var(--text-muted)' }}
          >
            <span>当前时间窗暂无可观测用量流水</span>
            <button
              type="button"
              onClick={() => setActivePage('sources')}
              className="text-[12px] underline cursor-pointer"
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
          <h2 className="text-[13.5px] font-semibold">Provider 运行与消耗汇总</h2>
          <span className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
            含今日、本月折算费用、缓存占比及主订阅窗口配额
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
                <th className="py-2 pr-4">Provider</th>
                <th className="py-2 px-3 text-right">今日用量 / 费用</th>
                <th className="py-2 px-3 text-right">本月用量 / 费用</th>
                <th className="py-2 px-3 text-right">缓存命中率</th>
                <th className="py-2 px-3 w-[220px]">主订阅窗口配额</th>
                <th className="py-2 pl-3 text-right">最近活跃</th>
              </tr>
            </thead>
            <tbody>
              {(data?.providers ?? []).map((row) => (
                <tr
                  key={row.provider_id}
                  className="hover:bg-white/[0.02] transition-colors"
                  style={{ borderBottom: '1px solid var(--border-subtle)' }}
                >
                  <td className="py-3 pr-4">
                    <div className="flex items-center gap-2">
                      <span
                        className="inline-block w-2 h-2 rounded-full"
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
                        未启用订阅规则
                      </span>
                    )}
                  </td>
                  <td className="py-3 pl-3 text-right text-[12px] tabular-nums" style={{ color: 'var(--text-secondary)' }}>
                    <div>{formatRelativeTime(row.last_sync_at_ms)}</div>
                    {row.last_error && (
                      <div className="text-[10.5px] truncate max-w-[160px]" style={{ color: 'var(--accent-rose)' }}>
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
