import { useMemo, useState } from 'react';
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
import { ChevronLeft, ChevronRight, Download } from 'lucide-react';
import { EChartBox } from '../components/EChartBox';
import { formatCost, formatExactInt, formatTokens } from '../lib/format';
import { ipc, isTauriRuntime } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type { MonthlyDailyLedgerRow } from '../types/ipc';

const colHelper = createColumnHelper<MonthlyDailyLedgerRow>();

export function MonthlyPage() {
  const now = new Date();
  const [year, setYear] = useState(now.getFullYear());
  const [month, setMonth] = useState(now.getMonth() + 1);
  const [sorting, setSorting] = useState<SortingState>([{ id: 'date', desc: true }]);
  const [exportNotice, setExportNotice] = useState<string | null>(null);

  const { providerFilter, currency, usdToCnyRate, tzOffsetMinutes } = useWorkspaceStore();

  const query = useMemo(
    () => ({
      year,
      month,
      provider_id: providerFilter === 'all' ? null : providerFilter,
      tz_offset_minutes: tzOffsetMinutes,
    }),
    [year, month, providerFilter, tzOffsetMinutes]
  );

  const { data, isLoading } = useQuery({
    queryKey: ['monthly', query],
    queryFn: () => ipc.getMonthly(query),
  });

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

    const maxTok = Math.max(1, ...cells.map((c) => c.total_tokens));
    const seriesData = cells.map((c) => [
      c.date,
      c.total_tokens,
      c.cost_micro_usd,
      c.top_model ?? '—',
      c.requests,
    ]);

    return {
      backgroundColor: 'transparent',
      tooltip: {
        position: 'top',
        backgroundColor: '#161922',
        borderColor: '#2d3343',
        textStyle: { color: '#eceff4', fontSize: 12 },
        formatter: (params: unknown) => {
          const p = params as { value?: [string, number, number, string, number] };
          if (!p.value) return '';
          const [date, tok, costMicro, topModel, reqs] = p.value;
          return `${date}<br/>Token: <b>${formatTokens(tok)}</b> (${reqs} 次)<br/>折算费用: <b>${formatCost(costMicro, currency, usdToCnyRate)}</b><br/>主用模型: ${topModel}`;
        },
      },
      visualMap: {
        min: 0,
        max: maxTok,
        show: false,
        inRange: {
          color: ['#1a1e28', '#1e3a5f', '#2563eb', '#10b981', '#34d399'],
        },
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
          color: '#141720',
          borderWidth: 2,
          borderColor: '#0c0d10',
        },
        yearLabel: { show: false },
        monthLabel: { color: '#6b7280', fontSize: 10 },
        dayLabel: { color: '#6b7280', fontSize: 10, firstDay: 1 },
      },
      series: [
        {
          type: 'heatmap',
          coordinateSystem: 'calendar',
          data: seriesData,
        },
      ],
    };
  }, [data, year, month, currency, usdToCnyRate]);

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
  const maxVal = Math.max(1, apiVal, subFee);
  const apiBarWidth = (apiVal / maxVal) * 100;
  const feeBarWidth = (subFee / maxVal) * 100;

  return (
    <div className="space-y-7">
      {/* Header + Month Picker + Export */}
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div>
          <h1 className="text-[16px] font-semibold tracking-tight">月度账单与活跃热力图</h1>
          <p className="text-[12px]" style={{ color: 'var(--text-muted)' }}>
            对比当月按量 API 公开定价等效价值与固定订阅月费支出，并支持标准报表导出
          </p>
        </div>

        <div className="flex items-center gap-3">
          <div
            className="flex items-center rounded"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            <button
              type="button"
              onClick={() => shiftMonth(-1)}
              className="p-1.5 hover:bg-white/5 cursor-pointer"
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
              className="p-1.5 hover:bg-white/5 cursor-pointer"
              aria-label="下一个月"
            >
              <ChevronRight className="w-3.5 h-3.5" />
            </button>
          </div>

          <button
            type="button"
            onClick={() => void handleExport('csv')}
            className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-[12px] rounded cursor-pointer hover:bg-white/5"
            style={{
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-primary)',
            }}
          >
            <Download className="w-3.5 h-3.5" />
            导出 CSV
          </button>
          <button
            type="button"
            onClick={() => void handleExport('json')}
            className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-[12px] rounded cursor-pointer hover:bg-white/5"
            style={{
              border: '1px solid var(--border-subtle)',
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

      {/* Monthly ROI Horizontal Comparison Bar */}
      <section
        className="pb-6 space-y-4"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div className="flex flex-wrap items-baseline justify-between gap-4">
          <div className="flex items-baseline gap-6 tabular-nums">
            <div>
              <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                当月 API 等效价值
              </div>
              <div className="text-[22px] font-semibold">
                {isLoading ? '…' : formatCost(apiVal, currency, usdToCnyRate)}
              </div>
            </div>
            <div style={{ color: 'var(--text-muted)' }}>vs</div>
            <div>
              <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                已启用订阅月费总支出
              </div>
              <div className="text-[22px] font-semibold" style={{ color: 'var(--text-secondary)' }}>
                {isLoading ? '…' : formatCost(subFee, currency, usdToCnyRate)}
              </div>
            </div>
          </div>

          <div className="text-right tabular-nums">
            <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
              订阅回本效率 / 缓存节省
            </div>
            <div className="text-[14px] font-semibold" style={{ color: 'var(--accent-emerald)' }}>
              {data?.roi_multiplier !== null && data?.roi_multiplier !== undefined
                ? `等效产出 ${data.roi_multiplier.toFixed(2)}x 订阅月费`
                : '按量计费模式'}
              {' · '}
              <span style={{ color: 'var(--text-secondary)', fontWeight: 400 }}>
                缓存节省 {formatCost(data?.month_cache_saved_micro_usd ?? 0, currency, usdToCnyRate)}
              </span>
            </div>
          </div>
        </div>

        <div className="space-y-2">
          <div className="flex items-center gap-3">
            <span className="w-24 text-[11.5px]" style={{ color: 'var(--text-secondary)' }}>
              API 等效产出
            </span>
            <div className="flex-1 h-2 rounded-full overflow-hidden" style={{ backgroundColor: 'var(--border-subtle)' }}>
              <div
                className="h-full rounded-full"
                style={{ width: `${apiBarWidth}%`, backgroundColor: 'var(--accent-emerald)' }}
              />
            </div>
          </div>
          <div className="flex items-center gap-3">
            <span className="w-24 text-[11.5px]" style={{ color: 'var(--text-secondary)' }}>
              订阅月费基准
            </span>
            <div className="flex-1 h-2 rounded-full overflow-hidden" style={{ backgroundColor: 'var(--border-subtle)' }}>
              <div
                className="h-full rounded-full"
                style={{ width: `${feeBarWidth}%`, backgroundColor: 'var(--accent-primary)' }}
              />
            </div>
          </div>
        </div>
      </section>

      {/* 365-Day Calendar Activity Heatmap */}
      <section
        className="pb-6"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div className="flex items-center justify-between mb-2">
          <h2 className="text-[13.5px] font-semibold">365 天活跃度热力图</h2>
          <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            本月累计 {formatTokens(data?.month_total_tokens ?? 0)} ·{' '}
            {(data?.month_total_requests ?? 0).toLocaleString('en-US')} 次请求
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
                      borderBottom: '1px solid var(--border-subtle)',
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
                    className="hover:bg-white/[0.02]"
                    style={{ borderBottom: '1px solid var(--border-subtle)' }}
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
    </div>
  );
}
