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
import { RefreshCw, SlidersHorizontal, X } from 'lucide-react';
import { InlineProgressBar } from '../components/InlineProgressBar';
import { TimeRangeControl } from '../components/TimeRangeControl';
import {
  formatCost,
  formatExactInt,
  formatRelativeTime,
  formatTokens,
  providerDisplayName,
} from '../lib/format';
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

const DEFAULT_PRICING_FORM: PricingFormState = {
  model_pattern: '',
  provider_id: 'claude_code',
  input_usd_per_m: '3.00',
  output_usd_per_m: '15.00',
  cache_read_usd_per_m: '0.30',
  cache_write_usd_per_m: '3.75',
  reasoning_usd_per_m: '15.00',
  recompute_history: true,
};

export function ModelsPricingPage() {
  const queryClient = useQueryClient();
  const {
    timePreset,
    customStartDate,
    customEndDate,
    providerFilter,
    deviceFilter,
    currency,
    usdToCnyRate,
    setUsdToCnyRate,
    getFilter,
  } = useWorkspaceStore();

  const [sorting, setSorting] = useState<SortingState>([
    { id: 'estimated_cost_micro_usd', desc: true },
  ]);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [rateInput, setRateInput] = useState(usdToCnyRate);
  const [form, setForm] = useState<PricingFormState>(DEFAULT_PRICING_FORM);
  const [feedback, setFeedback] = useState<{ kind: 'success' | 'error'; text: string } | null>(
    null
  );

  const filter = useMemo(
    () => getFilter(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [timePreset, customStartDate, customEndDate, providerFilter, deviceFilter]
  );

  const { data, isLoading } = useQuery({
    queryKey: ['models_pricing', filter],
    queryFn: () => ipc.getModelsPricing(filter),
  });

  const { data: prefs } = useQuery({
    queryKey: ['preferences'],
    queryFn: () => ipc.getPreferences(),
  });

  const syncedPublicModelsCount = useMemo(
    () =>
      (data?.catalog ?? []).filter(
        (c) => !c.model_pattern.startsWith('_default_') && !c.is_user_override
      ).length,
    [data]
  );
  const isExchangeRateAuto = prefs?.exchange_rate_auto ?? true;
  const latestSyncMs = prefs?.last_pricing_sync_at_ms ?? prefs?.last_rate_sync_at_ms ?? null;

  const openEditorForEntry = (
    entry: ModelPricingEntry | null,
    fallbackModel?: string,
    fallbackProv?: string
  ) => {
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
      setForm({
        ...DEFAULT_PRICING_FORM,
        model_pattern: fallbackModel,
        provider_id: fallbackProv ?? 'claude_code',
      });
    } else {
      setForm(DEFAULT_PRICING_FORM);
    }
    setRateInput(usdToCnyRate);
    setDrawerOpen(true);
  };

  const saveRateMutation = useMutation({
    mutationFn: async () => {
      const trimmedRate = rateInput.trim();
      const parsedRate = Number.parseFloat(trimmedRate);
      if (!Number.isFinite(parsedRate) || parsedRate <= 0) {
        throw new Error('USD → CNY 汇率必须为正数');
      }
      const currentPrefs = await ipc.getPreferences();
      const updated = await ipc.updatePreferences({
        ...currentPrefs,
        usd_to_cny_rate: trimmedRate,
        exchange_rate_auto: false,
      });
      setUsdToCnyRate(updated.usd_to_cny_rate);
      return updated.usd_to_cny_rate;
    },
    onSuccess: async (savedRate) => {
      setFeedback({
        kind: 'success',
        text: `已手动锁定全局汇率：1 USD = ${savedRate} CNY`,
      });
      await queryClient.invalidateQueries();
    },
    onError: (err: unknown) => {
      setFeedback({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    },
  });

  const restoreAutoRateMutation = useMutation({
    mutationFn: async () => {
      const currentPrefs = await ipc.getPreferences();
      await ipc.updatePreferences({
        ...currentPrefs,
        exchange_rate_auto: true,
      });
      return ipc.syncPricingAndRate(true);
    },
    onSuccess: async (res) => {
      setRateInput(res.usd_to_cny_rate);
      setUsdToCnyRate(res.usd_to_cny_rate);
      await queryClient.invalidateQueries({
        predicate: (query) => query.queryKey[0] !== 'bg-image-data-url',
      });
      setFeedback({
        kind: 'success',
        text: `已恢复自动汇率跟随 (1 USD = ${res.usd_to_cny_rate} CNY)`,
      });
    },
    onError: (err: unknown) => {
      setFeedback({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    },
  });

  const syncCatalogMutation = useMutation({
    mutationFn: () => ipc.syncPricingAndRate(true),
    onSuccess: async (res) => {
      if (res.rate_updated) {
        setUsdToCnyRate(res.usd_to_cny_rate);
        setRateInput(res.usd_to_cny_rate);
      }
      await queryClient.invalidateQueries({
        predicate: (query) => query.queryKey[0] !== 'bg-image-data-url',
      });
      if (res.warning && !res.rate_updated && !res.pricing_updated) {
        setFeedback({
          kind: 'error',
          text: `在线同步受限（已保留本地基线）：${res.warning}`,
        });
      } else {
        const parts = [
          res.rate_updated
            ? `汇率已同步 (1 USD = ${res.usd_to_cny_rate} CNY${res.rate_source ? ` · ${res.rate_source}` : ''})`
            : `汇率 (${res.usd_to_cny_rate}) 已最新`,
          res.pricing_updated
            ? `公开模型目录已更新 (${res.synced_models_count} 款，重算 ${res.recomputed_events_count} 条历史${res.pricing_source ? ` · ${res.pricing_source}` : ''})`
            : `公开模型目录 (${res.synced_models_count} 款) 已是最新`,
        ];
        setFeedback({ kind: 'success', text: parts.join('；') });
      }
    },
    onError: (err: unknown) => {
      setFeedback({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    },
  });

  const saveOverrideMutation = useMutation({
    mutationFn: async () => {
      const toMicro = (s: string) =>
        Math.max(0, Math.round((Number.parseFloat(s) || 0) * 1_000_000));
      const trimmedPattern = form.model_pattern.trim();
      if (!trimmedPattern) {
        throw new Error('模型匹配规则名称不能为空');
      }
      const trimmedRate = rateInput.trim();
      if (trimmedRate) {
        const parsedRate = Number.parseFloat(trimmedRate);
        if (!Number.isFinite(parsedRate) || parsedRate <= 0) {
          throw new Error('USD → CNY 汇率必须为正数');
        }
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
      if (trimmedRate && trimmedRate !== currentPrefs.usd_to_cny_rate) {
        await ipc.updatePreferences({
          ...currentPrefs,
          usd_to_cny_rate: trimmedRate,
        });
        setUsdToCnyRate(trimmedRate);
      }
      return updatedCount;
    },
    onSuccess: async (updatedCount) => {
      setFeedback({
        kind: 'success',
        text: `已保存定价覆盖（重算历史事件 ${updatedCount} 条）`,
      });
      await queryClient.invalidateQueries();
    },
    onError: (err: unknown) => {
      setFeedback({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    },
  });

  const deleteOverrideMutation = useMutation({
    mutationFn: async (pattern: string) => ipc.deleteModelPricingOverride(pattern, true),
    onSuccess: async (updatedCount) => {
      setFeedback({
        kind: 'success',
        text: `已恢复默认公开定价（重算历史事件 ${updatedCount} 条）`,
      });
      await queryClient.invalidateQueries();
    },
    onError: (err: unknown) => {
      setFeedback({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
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
                  border: '1px solid var(--border-hairline)',
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
        header: '模型提供商',
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
            className="btn-interactive text-[11.5px] px-2 py-0.5 rounded cursor-pointer"
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
          <p className="text-[12px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            {isExchangeRateAuto ? '自动汇率' : '锁定汇率'} 1 USD ={' '}
            {prefs?.usd_to_cny_rate ?? usdToCnyRate} CNY · 已同步公开模型单价{' '}
            {syncedPublicModelsCount} 款
            {latestSyncMs ? ` · 更新于 ${formatRelativeTime(latestSyncMs)}` : ''}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <TimeRangeControl />

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
            disabled={syncCatalogMutation.isPending}
            onClick={() => syncCatalogMutation.mutate()}
            className="btn-interactive bg-[var(--bg-elevated)] inline-flex items-center gap-1.5 px-2.5 py-1.5 text-[12px] rounded cursor-pointer disabled:opacity-50"
            style={{
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-primary)',
            }}
          >
            <RefreshCw
              className={`w-3.5 h-3.5 ${syncCatalogMutation.isPending ? 'animate-spin' : ''}`}
            />
            <span>{syncCatalogMutation.isPending ? '同步中…' : '立即同步'}</span>
          </button>

          <button
            type="button"
            onClick={() => openEditorForEntry(null)}
            className="btn-interactive bg-[var(--bg-elevated)] inline-flex items-center gap-1.5 px-3 py-1.5 text-[12px] rounded cursor-pointer"
            style={{
              border: '1px solid var(--border-strong)',
              color: 'var(--text-primary)',
            }}
          >
            <SlidersHorizontal className="w-3.5 h-3.5" />
            <span>汇率与定价抽屉</span>
          </button>
        </div>
      </section>

      {feedback && (
        <div
          className="px-3 py-2 text-[12px] rounded flex items-center justify-between gap-3"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            border: `1px solid ${
              feedback.kind === 'error' ? 'var(--accent-rose)' : 'var(--border-strong)'
            }`,
            color: feedback.kind === 'error' ? 'var(--accent-rose)' : 'var(--accent-emerald)',
          }}
        >
          <span>{feedback.text}</span>
          <button
            type="button"
            onClick={() => setFeedback(null)}
            className="btn-interactive p-1 rounded cursor-pointer"
            style={{ color: 'var(--text-muted)' }}
            aria-label="关闭提示"
          >
            <X className="w-3.5 h-3.5" />
          </button>
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
                        className={`py-2 px-2 cursor-pointer select-none transition-colors hover:text-[var(--text-primary)] ${
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
                    className="row-interactive"
                    style={{ borderBottom: '1px solid var(--border-hairline)' }}
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
        <div
          className="drawer-backdrop-enter fixed inset-0 z-50 flex justify-end"
          style={{ backgroundColor: 'rgba(0, 0, 0, 0.45)' }}
          onClick={(e) => {
            if (e.target === e.currentTarget) {
              setDrawerOpen(false);
            }
          }}
        >
          <div
            className="drawer-panel-enter w-full max-w-[460px] h-full flex flex-col justify-between p-5 overflow-y-auto"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              borderLeft: '1px solid var(--border-strong)',
              color: 'var(--text-primary)',
            }}
          >
            <div className="space-y-5">
              <div
                className="flex items-center justify-between pb-3"
                style={{ borderBottom: '1px solid var(--border-subtle)' }}
              >
                <div>
                  <h2 className="text-[14.5px] font-semibold">汇率与自定义模型单价</h2>
                </div>
                <button
                  type="button"
                  onClick={() => setDrawerOpen(false)}
                  className="btn-interactive p-1.5 rounded cursor-pointer"
                  style={{ color: 'var(--text-secondary)' }}
                  aria-label="关闭抽屉"
                >
                  <X className="w-4 h-4" />
                </button>
              </div>

              {/* USD -> CNY Exchange Rate */}
              <div
                className="space-y-2 pb-4"
                style={{ borderBottom: '1px solid var(--border-subtle)' }}
              >
                <div className="flex items-center justify-between">
                  <label className="block text-[12px] font-medium">USD → CNY 折算汇率</label>
                  <span
                    className="text-[11px] tabular-nums"
                    style={{
                      color: isExchangeRateAuto ? 'var(--accent-emerald)' : 'var(--accent-amber)',
                    }}
                  >
                    {isExchangeRateAuto ? '自动跟随实时汇率' : '已手动锁定汇率'}
                  </span>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <input
                    type="text"
                    inputMode="decimal"
                    value={rateInput}
                    onChange={(e) => setRateInput(e.target.value)}
                    className="w-28 px-2.5 py-1.5 text-[12.5px] rounded tabular-nums outline-none"
                    style={{
                      backgroundColor: 'var(--bg-input)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  />
                  <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                    1 USD = {rateInput} CNY
                  </span>
                  <div className="ml-auto flex items-center gap-1.5">
                    {!isExchangeRateAuto && (
                      <button
                        type="button"
                        disabled={restoreAutoRateMutation.isPending}
                        onClick={() => restoreAutoRateMutation.mutate()}
                        className="btn-interactive bg-[var(--bg-elevated)] px-2.5 py-1 text-[11.5px] rounded cursor-pointer disabled:opacity-50"
                        style={{
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--accent-emerald)',
                        }}
                      >
                        {restoreAutoRateMutation.isPending ? '同步中…' : '恢复自动汇率'}
                      </button>
                    )}
                    <button
                      type="button"
                      disabled={saveRateMutation.isPending}
                      onClick={() => saveRateMutation.mutate()}
                      className="btn-interactive bg-[var(--bg-elevated)] px-2.5 py-1 text-[11.5px] rounded cursor-pointer disabled:opacity-50"
                      style={{
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    >
                      {saveRateMutation.isPending ? '更新中…' : '锁定此汇率'}
                    </button>
                  </div>
                </div>
              </div>

              {/* Override Form */}
              <div className="space-y-3">
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                  <div>
                    <label
                      className="block text-[11.5px] mb-1"
                      style={{ color: 'var(--text-secondary)' }}
                    >
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
                    <label
                      className="block text-[11.5px] mb-1"
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      所属模型提供商
                    </label>
                    <select
                      aria-label="所属模型提供商"
                      value={form.provider_id}
                      onChange={(e) => setForm({ ...form, provider_id: e.target.value })}
                      className="w-full px-2.5 py-1.5 text-[12px] rounded outline-none cursor-pointer"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    >
                      <option value="claude_code">Claude Code</option>
                      <option value="codex">Codex</option>
                      <option value="antigravity">Antigravity</option>
                      <option value="kimi_code">Kimi Code</option>
                      <option value="workbuddy">Workbuddy</option>
                      <option value="deepseek_harness">DeepSeek Harness</option>
                      <option value="qcode">QCode</option>
                      <option value="minimax_code">MiniMax Code</option>
                    </select>
                  </div>
                </div>

                <div className="grid grid-cols-2 gap-3 tabular-nums">
                  {(
                    [
                      { key: 'input_usd_per_m', label: 'Input ($/1M)' },
                      { key: 'output_usd_per_m', label: 'Output ($/1M)' },
                      { key: 'cache_read_usd_per_m', label: 'Cache Read ($/1M)' },
                      { key: 'cache_write_usd_per_m', label: 'Cache Write ($/1M)' },
                      { key: 'reasoning_usd_per_m', label: 'Reasoning ($/1M)' },
                    ] as const
                  ).map((field) => (
                    <div key={field.key}>
                      <label
                        className="block text-[11.5px] mb-1"
                        style={{ color: 'var(--text-secondary)' }}
                      >
                        {field.label}
                      </label>
                      <input
                        type="text"
                        inputMode="decimal"
                        value={form[field.key]}
                        onChange={(e) => setForm({ ...form, [field.key]: e.target.value })}
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
                    className="btn-primary px-3.5 py-1.5 text-[12px] font-medium rounded cursor-pointer disabled:opacity-50"
                    style={{
                      backgroundColor: 'var(--accent-primary)',
                      color: '#ffffff',
                    }}
                  >
                    {saveOverrideMutation.isPending ? '保存并重算中…' : '保存并生效'}
                  </button>
                  <button
                    type="button"
                    onClick={() => setDrawerOpen(false)}
                    className="btn-interactive bg-[var(--bg-elevated)] px-3 py-1.5 text-[12px] rounded cursor-pointer"
                    style={{
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-secondary)',
                    }}
                  >
                    关闭
                  </button>
                </div>
              </div>

              {/* Built-in + Override Catalog Reference */}
              <div className="pt-4" style={{ borderTop: '1px solid var(--border-subtle)' }}>
                <div className="text-[12px] font-medium mb-2">当前生效定价目录</div>
                <div className="max-h-[260px] overflow-y-auto space-y-0.5">
                  {(data?.catalog ?? [])
                    .filter((c) => !c.model_pattern.startsWith('_default_'))
                    .map((entry) => (
                      <div
                        key={entry.model_pattern}
                        className="row-interactive flex items-center justify-between py-1.5 px-2 text-[11.5px] rounded"
                        style={{ borderBottom: '1px solid var(--border-hairline)' }}
                      >
                        <button
                          type="button"
                          onClick={() => openEditorForEntry(entry)}
                          className="text-left flex-1 cursor-pointer"
                        >
                          <span className="font-medium">{entry.model_pattern}</span>
                          {entry.is_user_override && (
                            <span
                              className="ml-1.5 text-[10px]"
                              style={{ color: 'var(--accent-primary)' }}
                            >
                              [覆盖]
                            </span>
                          )}
                          <div
                            className="text-[10.5px] tabular-nums"
                            style={{ color: 'var(--text-muted)' }}
                          >
                            In ${(entry.input_per_m_micro_usd / 1e6).toFixed(2)} · Out $
                            {(entry.output_per_m_micro_usd / 1e6).toFixed(2)} · CacheR $
                            {(entry.cache_read_per_m_micro_usd / 1e6).toFixed(3)}
                          </div>
                        </button>
                        {entry.is_user_override && (
                          <button
                            type="button"
                            disabled={deleteOverrideMutation.isPending}
                            onClick={() => deleteOverrideMutation.mutate(entry.model_pattern)}
                            className="btn-interactive text-[11px] px-2 py-0.5 rounded cursor-pointer disabled:opacity-50"
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
