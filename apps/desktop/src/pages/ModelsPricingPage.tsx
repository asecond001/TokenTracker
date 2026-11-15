import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createColumnHelper,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
  type SortingState,
} from '@tanstack/react-table';
import { SlidersHorizontal, X } from 'lucide-react';
import { InlineProgressBar } from '../components/InlineProgressBar';
import { formatCost, formatExactInt, formatTokens, providerDisplayName } from '../lib/format';
import { ipc } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type { ModelPricingEntry, ModelStatRow } from '../types/ipc';

const colHelper = createColumnHelper<ModelStatRow>();

interface PricingFormState {
  model_pattern: string;
  provider_id: string;
  input_usd_per_m: string;
  output_usd_per_m: string;
  cache_read_usd_per_m: string;
  cache_write_usd_per_m: string;
  reasoning_usd_per_m: string;
  recompute_history: boolean;
}

export function ModelsPricingPage() {
  const queryClient = useQueryClient();
  const { timePreset, customStartDate, customEndDate, providerFilter, currency, usdToCnyRate, setUsdToCnyRate, getFilter } =
    useWorkspaceStore();

  const [sorting, setSorting] = useState<SortingState>([
    { id: 'estimated_cost_micro_usd', desc: true },
  ]);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [rateInput, setRateInput] = useState(usdToCnyRate);
  const [form, setForm] = useState<PricingFormState>({
    model_pattern: '',
    provider_id: 'claude_code',
    input_usd_per_m: '3.00',
    output_usd_per_m: '15.00',
    cache_read_usd_per_m: '0.30',
    cache_write_usd_per_m: '3.75',
    reasoning_usd_per_m: '15.00',
    recompute_history: true,
  });
  const [feedback, setFeedback] = useState<string | null>(null);

  const filter = useMemo(
    () => getFilter(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [timePreset, customStartDate, customEndDate, providerFilter]
  );

  const { data, isLoading } = useQuery({
    queryKey: ['models_pricing', filter],
    queryFn: () => ipc.getModelsPricing(filter),
  });

  const openEditorForEntry = (entry: ModelPricingEntry | null, fallbackModel?: string, fallbackProv?: string) => {
    const toUsdStr = (micro: number) => (micro / 1_000_000).toFixed(3).replace(/\.?0+$/, '');
    if (entry) {
      setForm({
        model_pattern: entry.model_pattern,
        provider_id: entry.provider_id,
        input_usd_per_m: toUsdStr(entry.input_per_m_micro_usd),
        output_usd_per_m: toUsdStr(entry.output_per_m_micro_usd),
        cache_read_usd_per_m: toUsdStr(entry.cache_read_per_m_micro_usd),
        cache_write_usd_per_m: toUsdStr(entry.cache_write_per_m_micro_usd),
        reasoning_usd_per_m: toUsdStr(entry.reasoning_per_m_micro_usd),
        recompute_history: true,
      });
    } else if (fallbackModel) {
      setForm((prev) => ({
        ...prev,
        model_pattern: fallbackModel,
        provider_id: fallbackProv ?? 'claude_code',
      }));
    }
    setRateInput(usdToCnyRate);
    setDrawerOpen(true);
  };

  const saveOverrideMutation = useMutation({
    mutationFn: async () => {
      const toMicro = (s: string) => Math.max(0, Math.round((Number.parseFloat(s) || 0) * 1_000_000));
      const trimmedPattern = form.model_pattern.trim();
      if (!trimmedPattern) {
        throw new Error('模型匹配规则名称不能为空');
      }
      const entry: ModelPricingEntry = {
        model_pattern: trimmedPattern,
        provider_id: form.provider_id,
        input_per_m_micro_usd: toMicro(form.input_usd_per_m),
        output_per_m_micro_usd: toMicro(form.output_usd_per_m),
        cache_read_per_m_micro_usd: toMicro(form.cache_read_usd_per_m),
        cache_write_per_m_micro_usd: toMicro(form.cache_write_usd_per_m),
        reasoning_per_m_micro_usd: toMicro(form.reasoning_usd_per_m),
        is_user_override: true,
        effective_from: new Date().toISOString().slice(0, 10),
      };
      const updatedCount = await ipc.upsertModelPricingOverride(entry, form.recompute_history);
      const currentPrefs = await ipc.getPreferences();
      if (rateInput.trim() && rateInput.trim() !== currentPrefs.usd_to_cny_rate) {
        await ipc.updatePreferences({
          ...currentPrefs,
          usd_to_cny_rate: rateInput.trim(),
        });
        setUsdToCnyRate(rateInput.trim());
      }
      return updatedCount;
    },
    onSuccess: async (updatedCount) => {
      setFeedback(`已保存定价覆盖（重算历史事件 ${updatedCount} 条）`);
      await queryClient.invalidateQueries();
    },
    onError: (err) => {
      setFeedback(String(err));
    },
  });

  const deleteOverrideMutation = useMutation({
    mutationFn: async (pattern: string) =>
      ipc.deleteModelPricingOverride(pattern, true),
    onSuccess: async (updatedCount) => {
      setFeedback(`已恢复默认公开定价（重算历史事件 ${updatedCount} 条）`);
      await queryClient.invalidateQueries();
    },
  });

  const columns = useMemo(
    () => [
      colHelper.accessor('model_id', {
        header: '模型名',
        cell: (info) => (
          <div className="flex items-center gap-1.5">
            <span className="font-medium select-text">{info.getValue()}</span>
            {info.row.original.effective_pricing?.is_user_override && (
              <span
                className="px-1.5 py-0.5 text-[10px] rounded"
                style={{
                  backgroundColor: 'var(--bg-hover)',
                  color: 'var(--accent-primary)',
                }}
              >
                自定义单价
              </span>
            )}
          </div>
        ),
      }),
      colHelper.accessor('provider_id', {
        header: 'Provider',
        cell: (info) => (
          <span style={{ color: 'var(--text-secondary)' }}>
            {providerDisplayName(info.getValue())}
          </span>
        ),
      }),
      colHelper.accessor('requests', {
        header: '请求 / 步数',
        cell: (info) => <span className="tabular-nums">{formatExactInt(info.getValue())}</span>,
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
        header: 'Cache R / W',
        cell: (info) => (
          <span className="tabular-nums">
            {formatTokens(info.getValue())} / {formatTokens(info.row.original.cache_write_tokens)}
          </span>
        ),
      }),
      colHelper.accessor('cache_saved_micro_usd', {
        header: '缓存节省',
        cell: (info) => (
          <span className="tabular-nums" style={{ color: 'var(--accent-emerald)' }}>
            {formatCost(info.getValue(), currency, usdToCnyRate)}
          </span>
        ),
      }),
      colHelper.accessor('estimated_cost_micro_usd', {
        header: `等效总成本 (${currency})`,
        cell: (info) => (
          <span className="font-semibold tabular-nums">
            {formatCost(info.getValue(), currency, usdToCnyRate)}
          </span>
        ),
      }),
      colHelper.accessor('cost_share_ratio', {
        header: '成本占比',
        cell: (info) => (
          <div className="flex items-center gap-2 min-w-[110px]">
            <InlineProgressBar percent={info.getValue() * 100} heightPx={4} />
            <span className="text-[11px] tabular-nums w-10 text-right">
              {(info.getValue() * 100).toFixed(1)}%
            </span>
          </div>
        ),
      }),
      colHelper.display({
        id: 'actions',
        header: '',
        cell: (info) => (
          <button
            type="button"
            onClick={() =>
              openEditorForEntry(
                info.row.original.effective_pricing,
                info.row.original.model_id,
                info.row.original.provider_id
              )
            }
            className="text-[11.5px] px-2 py-0.5 rounded hover:bg-white/5 cursor-pointer"
            style={{ color: 'var(--accent-primary)' }}
          >
            改价
          </button>
        ),
      }),
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [currency, usdToCnyRate]
  );

  const table = useReactTable({
    data: data?.rows ?? [],
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  return (
    <div className="relative space-y-6">
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div>
          <h1 className="text-[16px] font-semibold tracking-tight">模型统计与定价覆盖</h1>
          <p className="text-[12px]" style={{ color: 'var(--text-muted)' }}>
            按固定点微美元核算各模型消费与提示词缓存节省，支持自定义网关单价覆盖与汇率配置
          </p>
        </div>

        <div className="flex items-center gap-4">
          <div className="text-right tabular-nums">
            <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
              累计缓存节省 / 折算总支出
            </div>
            <div className="text-[13px] font-medium">
              <span style={{ color: 'var(--accent-emerald)' }}>
                {formatCost(data?.total_cache_saved_micro_usd ?? 0, currency, usdToCnyRate)}
              </span>
              {' / '}
              <span>{formatCost(data?.total_cost_micro_usd ?? 0, currency, usdToCnyRate)}</span>
            </div>
          </div>

          <button
            type="button"
            onClick={() => openEditorForEntry(null)}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-[12px] rounded cursor-pointer hover:bg-white/5"
            style={{
              border: '1px solid var(--border-strong)',
              color: 'var(--text-primary)',
            }}
          >
            <SlidersHorizontal className="w-3.5 h-3.5" />
            汇率与定价抽屉
          </button>
        </div>
      </section>

      {feedback && (
        <div
          className="px-3 py-2 text-[12px] rounded"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            border: '1px solid var(--border-strong)',
            color: 'var(--text-primary)',
          }}
        >
          {feedback}
        </div>
      )}

      {/* Sortable High-Density Model Table */}
      <section>
        {isLoading ? (
          <div className="py-12 text-center" style={{ color: 'var(--text-muted)' }}>
            加载模型用量数据…
          </div>
        ) : (data?.rows.length ?? 0) === 0 ? (
          <div className="py-12 text-center text-[12.5px]" style={{ color: 'var(--text-muted)' }}>
            当前时间范围暂无模型调用数据
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
                          header.id === 'model_id' || header.id === 'provider_id'
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
                          cell.column.id === 'model_id' || cell.column.id === 'provider_id'
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

      {/* Flat Slide-Over Pricing & Exchange Rate Drawer */}
      {drawerOpen && (
        <div className="fixed inset-0 z-50 flex justify-end bg-black/50">
          <div
            className="w-[460px] h-full flex flex-col justify-between p-5 overflow-y-auto"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              borderLeft: '1px solid var(--border-strong)',
            }}
          >
            <div className="space-y-5">
              <div className="flex items-center justify-between pb-3" style={{ borderBottom: '1px solid var(--border-subtle)' }}>
                <div>
                  <h2 className="text-[14.5px] font-semibold">汇率与自定义模型单价</h2>
                  <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                    定价单位为 USD / 1M Tokens，用户覆盖优先级高于内置官方价格表
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => setDrawerOpen(false)}
                  className="p-1 rounded hover:bg-white/5 cursor-pointer"
                >
                  <X className="w-4 h-4" />
                </button>
              </div>

              {/* USD -> CNY Exchange Rate */}
              <div className="space-y-1.5 pb-4" style={{ borderBottom: '1px solid var(--border-subtle)' }}>
                <label className="block text-[12px] font-medium">USD → CNY 折算汇率</label>
                <div className="flex items-center gap-2">
                  <input
                    type="text"
                    value={rateInput}
                    onChange={(e) => setRateInput(e.target.value)}
                    className="w-32 px-2.5 py-1.5 text-[12.5px] rounded tabular-nums outline-none"
                    style={{
                      backgroundColor: 'var(--bg-input)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  />
                  <span className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                    1 USD = {rateInput} CNY
                  </span>
                </div>
              </div>

              {/* Override Form */}
              <div className="space-y-3">
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="block text-[11.5px] mb-1" style={{ color: 'var(--text-secondary)' }}>
                      模型匹配模式 (精确/前缀)
                    </label>
                    <input
                      type="text"
                      placeholder="例如 claude-sonnet-4-5"
                      value={form.model_pattern}
                      onChange={(e) => setForm({ ...form, model_pattern: e.target.value })}
                      className="w-full px-2.5 py-1.5 text-[12px] rounded outline-none"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    />
                  </div>
                  <div>
                    <label className="block text-[11.5px] mb-1" style={{ color: 'var(--text-secondary)' }}>
                      所属 Provider
                    </label>
                    <select
                      value={form.provider_id}
                      onChange={(e) => setForm({ ...form, provider_id: e.target.value })}
                      className="w-full px-2.5 py-1.5 text-[12px] rounded outline-none"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    >
                      <option value="claude_code">Claude Code</option>
                      <option value="codex">Codex</option>
                      <option value="antigravity">Antigravity</option>
                    </select>
                  </div>
                </div>

                <div className="grid grid-cols-2 gap-3 tabular-nums">
                  {[
                    { key: 'input_usd_per_m', label: 'Input ($/1M)' },
                    { key: 'output_usd_per_m', label: 'Output ($/1M)' },
                    { key: 'cache_read_usd_per_m', label: 'Cache Read ($/1M)' },
                    { key: 'cache_write_usd_per_m', label: 'Cache Write ($/1M)' },
                    { key: 'reasoning_usd_per_m', label: 'Reasoning ($/1M)' },
                  ].map((field) => (
                    <div key={field.key}>
                      <label className="block text-[11.5px] mb-1" style={{ color: 'var(--text-secondary)' }}>
                        {field.label}
                      </label>
                      <input
                        type="text"
                        value={form[field.key as keyof PricingFormState] as string}
                        onChange={(e) =>
                          setForm({ ...form, [field.key]: e.target.value })
                        }
                        className="w-full px-2.5 py-1.5 text-[12px] rounded tabular-nums outline-none"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                    </div>
                  ))}
                </div>

                <label className="flex items-center gap-2 text-[12px] pt-1 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={form.recompute_history}
                    onChange={(e) => setForm({ ...form, recompute_history: e.target.checked })}
                  />
                  <span>保存后立即重算历史流水的成本与聚合桶</span>
                </label>

                <div className="flex items-center gap-2 pt-2">
                  <button
                    type="button"
                    disabled={saveOverrideMutation.isPending}
                    onClick={() => saveOverrideMutation.mutate()}
                    className="px-3 py-1.5 text-[12px] font-medium rounded cursor-pointer"
                    style={{
                      backgroundColor: 'var(--accent-primary)',
                      color: '#ffffff',
                    }}
                  >
                    {saveOverrideMutation.isPending ? '保存并重算中…' : '保存并生效'}
                  </button>
                </div>
              </div>

              {/* Built-in + Override Catalog Reference */}
              <div className="pt-4" style={{ borderTop: '1px solid var(--border-subtle)' }}>
                <div className="text-[12px] font-medium mb-2">当前生效定价目录</div>
                <div className="max-h-[260px] overflow-y-auto space-y-1">
                  {(data?.catalog ?? [])
                    .filter((c) => !c.model_pattern.startsWith('_default_'))
                    .map((entry) => (
                      <div
                        key={entry.model_pattern}
                        className="flex items-center justify-between py-1.5 px-2 text-[11.5px] rounded hover:bg-white/[0.03]"
                        style={{ borderBottom: '1px solid var(--border-subtle)' }}
                      >
                        <button
                          type="button"
                          onClick={() => openEditorForEntry(entry)}
                          className="text-left flex-1 cursor-pointer"
                        >
                          <span className="font-medium">{entry.model_pattern}</span>
                          {entry.is_user_override && (
                            <span className="ml-1.5 text-[10px]" style={{ color: 'var(--accent-primary)' }}>
                              [覆盖]
                            </span>
                          )}
                          <div className="text-[10.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                            In ${(entry.input_per_m_micro_usd / 1e6).toFixed(2)} · Out $
                            {(entry.output_per_m_micro_usd / 1e6).toFixed(2)} · CacheR $
                            {(entry.cache_read_per_m_micro_usd / 1e6).toFixed(3)}
                          </div>
                        </button>
                        {entry.is_user_override && (
                          <button
                            type="button"
                            onClick={() => deleteOverrideMutation.mutate(entry.model_pattern)}
                            className="text-[11px] px-1.5 py-0.5 cursor-pointer"
                            style={{ color: 'var(--accent-rose)' }}
                          >
                            还原
                          </button>
                        )}
                      </div>
                    ))}
                </div>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
