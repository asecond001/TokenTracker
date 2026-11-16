import { useCallback, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  createColumnHelper,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
  type SortingState,
} from '@tanstack/react-table';
import { save } from '@tauri-apps/plugin-dialog';
import type { EChartsOption } from 'echarts';
import { ChevronDown, ChevronLeft, ChevronRight, Download } from 'lucide-react';
import { AnimatedNumber } from '../components/AnimatedNumber';
import { EChartBox } from '../components/EChartBox';
import { useChartTheme } from '../lib/chartTheme';
import { formatCost, formatExactInt, formatTokens } from '../lib/format';
import { ipc, isTauriRuntime } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type { HeatmapDayCell, MonthlyDailyLedgerRow } from '../types/ipc';

const colHelper = createColumnHelper<MonthlyDailyLedgerRow>();

export function computeHeatmapIntensityLevels(cells: HeatmapDayCell[]): Map<string, number> {
  const levels = new Map<string, number>();
  const positiveTokens = cells
    .filter((c) => c.total_tokens > 0)
    .map((c) => c.total_tokens)
    .sort((a, b) => a - b);

  const pickQuantile = (p: number): number => {
    if (positiveTokens.length === 0) return 0;
    const idx = Math.min(
      positiveTokens.length - 1,
      Math.max(0, Math.floor((positiveTokens.length - 1) * p))
    );
    return positiveTokens[idx];
  };

  const q25 = pickQuantile(0.25);
  const q50 = pickQuantile(0.5);
  const q75 = pickQuantile(0.75);
  const uniformPositive = positiveTokens.length > 0 && q75 <= q25;

  for (const c of cells) {
    if (c.total_tokens <= 0) {
      levels.set(c.date, c.requests > 0 ? 1 : 0);
      continue;
    }
    if (uniformPositive) {
      levels.set(c.date, 3);
      continue;
    }
    if (c.total_tokens <= q25) {
      levels.set(c.date, 1);
    } else if (c.total_tokens <= q50) {
      levels.set(c.date, 2);
    } else if (c.total_tokens <= q75) {
      levels.set(c.date, 3);
    } else {
      levels.set(c.date, 4);
    }
  }

  return levels;
}

function escapeHtml(text: string): string {
  return text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

export function MonthlyPage() {
  const now = new Date();
  const [year, setYear] = useState(now.getFullYear());
  const [month, setMonth] = useState(now.getMonth() + 1);
  const [sorting, setSorting] = useState<SortingState>([{ id: 'date', desc: true }]);
  const [exportNotice, setExportNotice] = useState<string | null>(null);
  const [roiExpanded, setRoiExpanded] = useState(true);

  const { providerFilter, deviceFilter, currency, usdToCnyRate, tzOffsetMinutes } =
    useWorkspaceStore();
  const chartTheme = useChartTheme();

  const query = useMemo(
    () => ({
      year,
      month,
      provider_id: providerFilter === 'all' ? null : providerFilter,
      device_id: deviceFilter === 'all' ? null : deviceFilter,
      tz_offset_minutes: tzOffsetMinutes,
    }),
    [year, month, providerFilter, deviceFilter, tzOffsetMinutes]
  );

  const { data, isLoading } = useQuery({
    queryKey: ['monthly', query],
    queryFn: () => ipc.getMonthly(query),
  });

  const costFormatter = useCallback(
    (value: number) => formatCost(value, currency, usdToCnyRate),
    [currency, usdToCnyRate]
  );

  const shiftMonth = (delta: number) => {
    const d = new Date(year, month - 1 + delta, 1);
    setYear(d.getFullYear());
    setMonth(d.getMonth() + 1);
  };

  const handleExport = async (format: 'csv' | 'json') => {
    setExportNotice(null);
    const defaultName = `tokentracker-${year}-${String(month).padStart(2, '0')}.${format}`;
    let savePath: string | null = null;
    if (isTauriRuntime()) {
      savePath = await save({
        defaultPath: defaultName,
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      if (!savePath) return;
    }

    const res = await ipc.exportMonthlyReport(query, format, savePath);
    if (!isTauriRuntime()) {
      const blob = new Blob([res.content], {
        type: format === 'csv' ? 'text/csv;charset=utf-8' : 'application/json;charset=utf-8',
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = defaultName;
      a.click();
      URL.revokeObjectURL(url);
      setExportNotice(`已生成 ${defaultName} (${res.bytes} bytes)`);
    } else {
      setExportNotice(`已导出至 ${res.path} (${res.bytes} bytes)`);
    }
  };

  const heatmapOption = useMemo<EChartsOption>(() => {
    const cells = data?.heatmap_365d ?? [];
    const endDay = `${year}-${String(month).padStart(2, '0')}-${String(
      new Date(year, month, 0).getDate()
    ).padStart(2, '0')}`;
    const startDateObj = new Date(year, month, 0);
    startDateObj.setDate(startDateObj.getDate() - 364);
    const startDay = `${startDateObj.getFullYear()}-${String(
      startDateObj.getMonth() + 1
    ).padStart(2, '0')}-${String(startDateObj.getDate()).padStart(2, '0')}`;

    const intensityLevels = computeHeatmapIntensityLevels(cells);
    const seriesData = cells.map((c) => [
      c.date,
      intensityLevels.get(c.date) ?? 0,
      c.total_tokens,
      c.cost_micro_usd,
      c.top_model ?? '—',
      c.requests,
    ]);

    return {
      backgroundColor: 'transparent',
      animationDuration: 280,
      animationDurationUpdate: 260,
      tooltip: {
        position: 'top',
        backgroundColor: chartTheme.tooltipBg,
        borderColor: chartTheme.tooltipBorder,
        textStyle: { color: chartTheme.tooltipText, fontSize: 12 },
        formatter: (params: unknown) => {
          const p = params as {
            value?: [string, number, number, number, string, number];
          };
          if (!p.value) return '';
          const [date, , tok, costMicro, topModel, reqs] = p.value;
          const tokenText =
            tok === 0 && reqs > 0 ? '0 (仅可观测步数)' : formatTokens(tok);
          return `${escapeHtml(date)}<br/>Token: <b>${tokenText}</b> (${formatExactInt(reqs)} 次)<br/>折算费用: <b>${formatCost(costMicro, currency, usdToCnyRate)}</b><br/>主用模型: ${escapeHtml(topModel)}`;
        },
      },
      visualMap: {
        type: 'piecewise',
        dimension: 1,
        show: false,
        pieces: [
          { value: 0, color: chartTheme.heatmapEmptyColor },
          { value: 1, color: chartTheme.heatmapRamp[0] },
          { value: 2, color: chartTheme.heatmapRamp[1] },
          { value: 3, color: chartTheme.heatmapRamp[2] },
          { value: 4, color: chartTheme.heatmapRamp[3] },
        ],
      },
      calendar: {
        top: 24,
        left: 28,
        right: 12,
        bottom: 8,
        cellSize: ['auto', 13],
        range: [startDay, endDay],
        splitLine: { show: false },
        itemStyle: {
          color: chartTheme.heatmapEmptyColor,
          borderWidth: 2,
          borderColor: chartTheme.heatmapBorderColor,
        },
        yearLabel: { show: false },
        monthLabel: { color: chartTheme.textMuted, fontSize: 10 },
        dayLabel: { color: chartTheme.textMuted, fontSize: 10, firstDay: 1 },
      },
      series: [
        {
          id: 'monthly-heatmap-365d',
          type: 'heatmap',
          coordinateSystem: 'calendar',
          data: seriesData,
        },
      ],
    };
  }, [data, year, month, currency, usdToCnyRate, chartTheme]);

  const columns = useMemo(
    () => [
      colHelper.accessor('date', {
        header: '日期',
        cell: (info) => <span className="font-medium tabular-nums">{info.getValue()}</span>,
      }),
      colHelper.accessor('requests', {
        header: '请求/步数',
        cell: (info) => (
          <span className="tabular-nums">
            {formatExactInt(info.getValue())}
            {info.row.original.observable_steps > 0
              ? ` (${info.row.original.observable_steps} 步)`
              : ''}
          </span>
        ),
      }),
      colHelper.accessor('input_tokens', {
        header: 'Input',
        cell: (info) => <span className="tabular-nums">{formatTokens(info.getValue())}</span>,
      }),
      colHelper.accessor('output_tokens', {
        header: 'Output',
        cell: (info) => (
          <span className="tabular-nums">
            {formatTokens(info.getValue() + info.row.original.reasoning_tokens)}
          </span>
        ),
      }),
      colHelper.accessor('cache_read_tokens', {
        header: 'Cache Read',
        cell: (info) => <span className="tabular-nums">{formatTokens(info.getValue())}</span>,
      }),
      colHelper.accessor('total_tokens', {
        header: '总 Token',
        cell: (info) => (
          <span className="font-medium tabular-nums">{formatTokens(info.getValue())}</span>
        ),
      }),
      colHelper.accessor('top_model', {
        header: '主用模型',
        cell: (info) => (
          <span className="text-[12px]" style={{ color: 'var(--text-secondary)' }}>
            {info.getValue() ?? '—'}
          </span>
        ),
      }),
      colHelper.accessor('cost_micro_usd', {
        header: `折算费用 (${currency})`,
        cell: (info) => (
          <span className="font-semibold tabular-nums">
            {formatCost(info.getValue(), currency, usdToCnyRate)}
          </span>
        ),
      }),
    ],
    [currency, usdToCnyRate]
  );

  const table = useReactTable({
    data: data?.daily_ledger ?? [],
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  const apiVal = data?.api_equivalent_cost_micro_usd ?? 0;
  const subFee = data?.subscription_monthly_fee_micro_usd ?? 0;
  const hasUserSubscriptionFee = subFee > 0;
  const maxVal = Math.max(1, apiVal, subFee);
  const apiBarWidth = (apiVal / maxVal) * 100;
  const feeBarWidth = (subFee / maxVal) * 100;

  return (
    <div className="space-y-7">
      {/* Header + Month Picker + Export */}
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div>
          <h1 className="text-[16px] font-semibold tracking-tight">月度账单与活跃热力图</h1>
        </div>

        <div className="flex items-center gap-3">
          <div
            className="flex items-center rounded"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              border: '1px solid var(--border-hairline)',
            }}
          >
            <button
              type="button"
              onClick={() => shiftMonth(-1)}
              className="btn-interactive p-1.5 rounded-l cursor-pointer"
              aria-label="上一个月"
            >
              <ChevronLeft className="w-3.5 h-3.5" />
            </button>
            <span className="px-3 text-[12.5px] font-medium tabular-nums">
              {year}-{String(month).padStart(2, '0')}
            </span>
            <button
              type="button"
              onClick={() => shiftMonth(1)}
              className="btn-interactive p-1.5 rounded-r cursor-pointer"
              aria-label="下一个月"
            >
              <ChevronRight className="w-3.5 h-3.5" />
            </button>
          </div>

          <button
            type="button"
            onClick={() => void handleExport('csv')}
            className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1.5 text-[12px] rounded cursor-pointer"
            style={{
              border: '1px solid var(--border-hairline)',
              color: 'var(--text-primary)',
            }}
          >
            <Download className="w-3.5 h-3.5" />
            导出 CSV
          </button>
          <button
            type="button"
            onClick={() => void handleExport('json')}
            className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1.5 text-[12px] rounded cursor-pointer"
            style={{
              border: '1px solid var(--border-hairline)',
              color: 'var(--text-primary)',
            }}
          >
            <Download className="w-3.5 h-3.5" />
            导出 JSON
          </button>
        </div>
      </section>

      {exportNotice && (
        <div
          className="px-3 py-2 text-[12px] rounded"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            border: '1px solid var(--accent-emerald)',
            color: 'var(--accent-emerald)',
          }}
        >
          {exportNotice}
        </div>
      )}

      {/* Objective Top 4-Metric Strip (Zero-Assumption: No Subscription Fee / ROI Prominence Here) */}
      <section
        className="grid grid-cols-4 pb-6"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div className="pr-5">
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            当月按量 API 等效价值 ({currency})
          </div>
          <div className="mt-1.5 text-[24px] font-semibold tracking-tight tabular-nums">
            {isLoading && !data ? (
              '…'
            ) : (
              <AnimatedNumber value={apiVal} formatter={costFormatter} />
            )}
          </div>
        </div>

        <div
          className="px-5"
          style={{ borderLeft: '1px solid var(--border-hairline)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            当月总 Token
          </div>
          <div className="mt-1.5 text-[24px] font-semibold tracking-tight tabular-nums">
            {isLoading && !data ? (
              '…'
            ) : (
              <AnimatedNumber value={data?.month_total_tokens ?? 0} formatter={formatTokens} />
            )}
          </div>
        </div>

        <div
          className="px-5"
          style={{ borderLeft: '1px solid var(--border-hairline)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            缓存节省金额 ({currency})
          </div>
          <div
            className="mt-1.5 text-[24px] font-semibold tracking-tight tabular-nums"
            style={{ color: 'var(--accent-emerald)' }}
          >
            {isLoading && !data ? (
              '…'
            ) : (
              <AnimatedNumber
                value={data?.month_cache_saved_micro_usd ?? 0}
                formatter={costFormatter}
              />
            )}
          </div>
        </div>

        <div
          className="pl-5"
          style={{ borderLeft: '1px solid var(--border-hairline)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            有效请求数
          </div>
          <div className="mt-1.5 text-[24px] font-semibold tracking-tight tabular-nums">
            {isLoading && !data ? (
              '…'
            ) : (
              <AnimatedNumber
                value={data?.month_total_requests ?? 0}
                formatter={formatExactInt}
              />
            )}
          </div>
          <div className="mt-1 text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            {data?.daily_ledger.length ?? 0} 个活跃记账日
          </div>
        </div>
      </section>

      {/* 365-Day Calendar Activity Heatmap */}
      <section
        className="pb-6"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div className="flex flex-wrap items-center justify-between gap-2 mb-2">
          <div className="flex items-center gap-3">
            <h2 className="text-[13.5px] font-semibold">365 天活跃度热力图</h2>
            <div
              data-testid="heatmap-legend"
              className="inline-flex items-center gap-1 text-[10.5px]"
              style={{ color: 'var(--text-muted)' }}
              aria-label="热力图活跃等级图例"
            >
              <span>少</span>
              {[chartTheme.heatmapEmptyColor, ...chartTheme.heatmapRamp].map((color, idx) => (
                <span
                  key={idx}
                  className="inline-block w-2.5 h-2.5 rounded-[2px]"
                  style={{
                    backgroundColor: color,
                    border: '1px solid var(--border-hairline)',
                  }}
                />
              ))}
              <span>多</span>
            </div>
          </div>
          <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            本月累计 {formatTokens(data?.month_total_tokens ?? 0)} ·{' '}
            {formatExactInt(data?.month_total_requests ?? 0)} 次请求
          </span>
        </div>
        <EChartBox option={heatmapOption} height={155} />
      </section>

      {/* Daily Ledger TanStack Table */}
      <section>
        <h2 className="text-[13.5px] font-semibold mb-3">按日账单明细</h2>
        {(data?.daily_ledger.length ?? 0) === 0 ? (
          <div className="py-10 text-center text-[12.5px]" style={{ color: 'var(--text-muted)' }}>
            {year} 年 {month} 月暂无用量记录
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full border-collapse text-left">
              <thead>
                {table.getHeaderGroups().map((hg) => (
                  <tr
                    key={hg.id}
                    className="text-[11px] font-medium uppercase tracking-wider"
                    style={{
                      color: 'var(--text-muted)',
                      borderBottom: '1px solid var(--border-hairline)',
                    }}
                  >
                    {hg.headers.map((header) => (
                      <th
                        key={header.id}
                        onClick={header.column.getToggleSortingHandler()}
                        className={`py-2 px-2 cursor-pointer select-none ${
                          header.id === 'date' || header.id === 'top_model'
                            ? 'text-left'
                            : 'text-right'
                        }`}
                      >
                        {flexRender(header.column.columnDef.header, header.getContext())}
                        {header.column.getIsSorted() === 'asc'
                          ? ' ↑'
                          : header.column.getIsSorted() === 'desc'
                            ? ' ↓'
                            : ''}
                      </th>
                    ))}
                  </tr>
                ))}
              </thead>
              <tbody>
                {table.getRowModel().rows.map((row) => (
                  <tr
                    key={row.id}
                    className="row-interactive"
                    style={{ borderBottom: '1px solid var(--border-hairline)' }}
                  >
                    {row.getVisibleCells().map((cell) => (
                      <td
                        key={cell.id}
                        className={`py-2.5 px-2 ${
                          cell.column.id === 'date' || cell.column.id === 'top_model'
                            ? 'text-left'
                            : 'text-right'
                        }`}
                      >
                        {flexRender(cell.column.columnDef.cell, cell.getContext())}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {/* Secondary Collapsible Subscription ROI Reference (Only When User Configured Monthly Fee > 0) */}
      {hasUserSubscriptionFee && (
        <section
          className="pt-5"
          style={{ borderTop: '1px solid var(--border-hairline)' }}
        >
          <button
            type="button"
            onClick={() => setRoiExpanded((prev) => !prev)}
            className="row-interactive w-full flex items-center justify-between py-2 px-1 rounded cursor-pointer text-left"
          >
            <div className="flex items-center gap-2">
              {roiExpanded ? (
                <ChevronDown className="w-3.5 h-3.5" style={{ color: 'var(--text-muted)' }} />
              ) : (
                <ChevronRight className="w-3.5 h-3.5" style={{ color: 'var(--text-muted)' }} />
              )}
              <span className="text-[12.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
                订阅回本参考
              </span>
            </div>

            <div className="flex items-center gap-4 text-[12px] tabular-nums">
              <span style={{ color: 'var(--text-muted)' }}>
                已录入月费 {formatCost(subFee, currency, usdToCnyRate)}
              </span>
              {data?.roi_multiplier !== null && data?.roi_multiplier !== undefined && (
                <span className="font-medium" style={{ color: 'var(--accent-emerald)' }}>
                  等效产出 {data.roi_multiplier.toFixed(2)}x
                </span>
              )}
            </div>
          </button>

          {roiExpanded && (
            <div className="mt-3 pl-6 pr-1 space-y-2.5">
              <div className="flex items-center gap-3">
                <span className="w-28 text-[11.5px]" style={{ color: 'var(--text-secondary)' }}>
                  当月 API 等效产出
                </span>
                <div
                  className="flex-1 h-1.5 rounded-full overflow-hidden"
                  style={{ backgroundColor: 'var(--border-hairline)' }}
                >
                  <div
                    className="h-full rounded-full"
                    style={{
                      width: `${apiBarWidth}%`,
                      backgroundColor: 'var(--accent-emerald)',
                      transition: 'width 280ms cubic-bezier(0.16, 1, 0.3, 1)',
                    }}
                  />
                </div>
                <span
                  className="w-24 text-right text-[11.5px] tabular-nums"
                  style={{ color: 'var(--text-primary)' }}
                >
                  {formatCost(apiVal, currency, usdToCnyRate)}
                </span>
              </div>

              <div className="flex items-center gap-3">
                <span className="w-28 text-[11.5px]" style={{ color: 'var(--text-secondary)' }}>
                  订阅月费基准
                </span>
                <div
                  className="flex-1 h-1.5 rounded-full overflow-hidden"
                  style={{ backgroundColor: 'var(--border-hairline)' }}
                >
                  <div
                    className="h-full rounded-full"
                    style={{
                      width: `${feeBarWidth}%`,
                      backgroundColor: 'var(--accent-primary)',
                      transition: 'width 280ms cubic-bezier(0.16, 1, 0.3, 1)',
                    }}
                  />
                </div>
                <span
                  className="w-24 text-right text-[11.5px] tabular-nums"
                  style={{ color: 'var(--text-secondary)' }}
                >
                  {formatCost(subFee, currency, usdToCnyRate)}
                </span>
              </div>
            </div>
          )}
        </section>
      )}
    </div>
  );
}
