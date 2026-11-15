import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { EChartBox } from '../components/EChartBox';
import {
  formatDurationMs,
  formatTokens,
  formatTps,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';

export function SpeedLatencyPage() {
  const { timePreset, customStartDate, customEndDate, providerFilter, getFilter } =
    useWorkspaceStore();

  const filter = useMemo(
    () => getFilter(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [timePreset, customStartDate, customEndDate, providerFilter]
  );

  const { data, isLoading } = useQuery({
    queryKey: ['speed_latency', filter],
    queryFn: () => ipc.getSpeedLatency(filter),
    refetchInterval: 15_000,
  });

  const modelSpeedOption = useMemo<EChartsOption>(() => {
    const models = (data?.models ?? []).slice(0, 10);
    return {
      backgroundColor: 'transparent',
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'shadow' },
        backgroundColor: '#161922',
        borderColor: '#2d3343',
        textStyle: { color: '#eceff4', fontSize: 12 },
      },
      legend: {
        top: 0,
        right: 0,
        textStyle: { color: '#98a2b3', fontSize: 11 },
      },
      grid: { top: 32, right: 20, bottom: 24, left: 165 },
      xAxis: {
        type: 'value',
        name: 'tok/s',
        nameTextStyle: { color: '#6b7280', fontSize: 11 },
        splitLine: { lineStyle: { color: '#1f232e', type: 'dashed' } },
        axisLabel: { color: '#6b7280', fontSize: 11 },
      },
      yAxis: {
        type: 'category',
        inverse: true,
        data: models.map((m) => m.model_id),
        axisLine: { lineStyle: { color: '#2b3040' } },
        axisLabel: { color: '#98a2b3', fontSize: 11 },
      },
      series: [
        {
          name: 'P50',
          type: 'bar',
          barWidth: 7,
          itemStyle: { color: '#38bdf8' },
          data: models.map((m) => Number(m.p50_tps.toFixed(1))),
        },
        {
          name: '均值',
          type: 'bar',
          barWidth: 7,
          itemStyle: { color: '#10b981' },
          data: models.map((m) => Number(m.mean_tps.toFixed(1))),
        },
        {
          name: 'P95',
          type: 'bar',
          barWidth: 7,
          itemStyle: { color: '#6366f1' },
          data: models.map((m) => Number(m.p95_tps.toFixed(1))),
        },
      ],
    };
  }, [data]);

  const contextDecayOption = useMemo<EChartsOption>(() => {
    const scatter = data?.scatter_points ?? [];
    const bins = data?.context_bins ?? [];

    return {
      backgroundColor: 'transparent',
      tooltip: {
        trigger: 'item',
        backgroundColor: '#161922',
        borderColor: '#2d3343',
        textStyle: { color: '#eceff4', fontSize: 12 },
        formatter: (params: unknown) => {
          const p = params as { seriesType?: string; value?: [number, number, string, number] };
          if (p.seriesType === 'scatter' && p.value) {
            return `${p.value[2]}<br/>上下文: ${formatTokens(p.value[0])}<br/>输出速度: <b>${p.value[1].toFixed(1)} tok/s</b><br/>耗时: ${formatDurationMs(p.value[3])}`;
          }
          return '';
        },
      },
      grid: { top: 24, right: 20, bottom: 28, left: 50 },
      xAxis: {
        type: 'value',
        name: '输入上下文 Tokens',
        nameTextStyle: { color: '#6b7280', fontSize: 11 },
        splitLine: { lineStyle: { color: '#1f232e', type: 'dashed' } },
        axisLabel: {
          color: '#6b7280',
          fontSize: 11,
          formatter: (v: number) => formatTokens(v),
        },
      },
      yAxis: {
        type: 'value',
        name: 'Output tok/s',
        nameTextStyle: { color: '#6b7280', fontSize: 11 },
        splitLine: { lineStyle: { color: '#1f232e', type: 'dashed' } },
        axisLabel: { color: '#6b7280', fontSize: 11 },
      },
      series: [
        {
          name: '单次请求采样',
          type: 'scatter',
          symbolSize: 6,
          data: scatter.map((pt) => ({
            value: [pt.context_tokens, Number(pt.output_tps.toFixed(1)), pt.model_id, pt.duration_ms],
            itemStyle: {
              color: providerAccentColor(pt.provider_id),
              opacity: 0.72,
            },
          })),
        },
        {
          name: '分箱均值趋势',
          type: 'line',
          smooth: true,
          symbolSize: 7,
          lineStyle: { color: '#eceff4', width: 1.5, type: 'dashed' },
          itemStyle: { color: '#eceff4' },
          data: bins
            .filter((b) => b.sample_count > 0)
            .map((b) => {
              const midpoint =
                b.bin_label === '0-8k'
                  ? 4_000
                  : b.bin_label === '8k-32k'
                    ? 20_000
                    : b.bin_label === '32k-128k'
                      ? 80_000
                      : b.bin_label === '128k-256k'
                        ? 192_000
                        : 300_000;
              return [midpoint, Number(b.avg_tps.toFixed(1))];
            }),
        },
      ],
    };
  }, [data]);

  return (
    <div className="space-y-7">
      <section
        className="pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <h1 className="text-[16px] font-semibold tracking-tight">生成速度与上下文衰减监测</h1>
        <p className="text-[12px]" style={{ color: 'var(--text-muted)' }}>
          分析各模型 Output Tokens/s 的均值、P50/P95 分位分布，以及长上下文对响应吞吐的影响拐点
        </p>
      </section>

      {isLoading ? (
        <div className="py-12 text-center" style={{ color: 'var(--text-muted)' }}>
          计算速度分布与延迟分箱…
        </div>
      ) : (data?.models.length ?? 0) === 0 ? (
        <div className="py-12 text-center text-[12.5px]" style={{ color: 'var(--text-muted)' }}>
          当前时间窗内暂无带耗时样本的速度记录
        </div>
      ) : (
        <>
          {/* Model TPS Comparison Chart */}
          <section
            className="pb-6"
            style={{ borderBottom: '1px solid var(--border-subtle)' }}
          >
            <h2 className="text-[13.5px] font-semibold mb-2">分模型 Output Tokens/s 分布对比</h2>
            <EChartBox option={modelSpeedOption} height={Math.max(200, (data?.models.length ?? 1) * 34)} />
          </section>

          {/* Context Decay Scatter + Binned Summary */}
          <section
            className="pb-6 space-y-4"
            style={{ borderBottom: '1px solid var(--border-subtle)' }}
          >
            <div>
              <h2 className="text-[13.5px] font-semibold">上下文大小 vs 生成速度衰减洞察</h2>
              <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                散点代表单次请求上下文长度与吞吐速率，虚线为 5 个上下文区间均值变化
              </p>
            </div>
            <EChartBox option={contextDecayOption} height={260} />

            {/* Flat 5-Bin Context Summary Strip */}
            <div
              className="grid grid-cols-5 pt-3"
              style={{ borderTop: '1px solid var(--border-subtle)' }}
            >
              {(data?.context_bins ?? []).map((bin, idx) => (
                <div
                  key={bin.bin_label}
                  className={idx === 0 ? 'pr-3' : 'px-3'}
                  style={
                    idx > 0 ? { borderLeft: '1px solid var(--border-subtle)' } : undefined
                  }
                >
                  <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                    上下文 {bin.bin_label}
                  </div>
                  <div className="mt-1 text-[16px] font-semibold tabular-nums">
                    {bin.sample_count > 0 ? formatTps(bin.avg_tps) : '—'}
                  </div>
                  <div className="text-[11px] tabular-nums" style={{ color: 'var(--text-secondary)' }}>
                    {bin.sample_count} 样本 · {formatDurationMs(bin.avg_duration_ms)}
                  </div>
                </div>
              ))}
            </div>
          </section>

          {/* Flat Model Latency Detail Table */}
          <section>
            <h2 className="text-[13.5px] font-semibold mb-3">模型速度与耗时明细表</h2>
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
                    <th className="py-2 pr-4">模型</th>
                    <th className="py-2 px-3">Provider</th>
                    <th className="py-2 px-3 text-right">样本数</th>
                    <th className="py-2 px-3 text-right">最新</th>
                    <th className="py-2 px-3 text-right">均值</th>
                    <th className="py-2 px-3 text-right">P50</th>
                    <th className="py-2 px-3 text-right">P95</th>
                    <th className="py-2 px-3 text-right">峰值</th>
                    <th className="py-2 pl-3 text-right">平均耗时</th>
                  </tr>
                </thead>
                <tbody>
                  {(data?.models ?? []).map((m) => (
                    <tr
                      key={`${m.provider_id}-${m.model_id}`}
                      className="hover:bg-white/[0.02]"
                      style={{ borderBottom: '1px solid var(--border-subtle)' }}
                    >
                      <td className="py-2.5 pr-4 font-medium">{m.model_id}</td>
                      <td className="py-2.5 px-3" style={{ color: 'var(--text-secondary)' }}>
                        {providerDisplayName(m.provider_id)}
                      </td>
                      <td className="py-2.5 px-3 text-right tabular-nums">{m.sample_count}</td>
                      <td className="py-2.5 px-3 text-right tabular-nums" style={{ color: 'var(--accent-emerald)' }}>
                        {formatTps(m.latest_tps)}
                      </td>
                      <td className="py-2.5 px-3 text-right tabular-nums">{formatTps(m.mean_tps)}</td>
                      <td className="py-2.5 px-3 text-right tabular-nums">{formatTps(m.p50_tps)}</td>
                      <td className="py-2.5 px-3 text-right tabular-nums">{formatTps(m.p95_tps)}</td>
                      <td className="py-2.5 px-3 text-right tabular-nums">{formatTps(m.max_tps)}</td>
                      <td className="py-2.5 pl-3 text-right tabular-nums" style={{ color: 'var(--text-secondary)' }}>
                        {formatDurationMs(m.avg_duration_ms)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        </>
      )}
    </div>
  );
}
