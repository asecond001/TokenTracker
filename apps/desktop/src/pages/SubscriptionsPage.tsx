import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, Info, Pencil, Plus, Trash2, X } from 'lucide-react';
import { InlineProgressBar } from '../components/InlineProgressBar';
import {
  formatCost,
  formatCountdown,
  formatRelativeTime,
  formatTokens,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type {
  CurrencyCode,
  ObservedRateLimitWindow,
  QuotaUnit,
  RemoteSubscriptionSnapshot,
  SubscriptionPlan,
  SubscriptionPlanStatus,
  SubscriptionPlanTemplate,
  SubscriptionWindowRule,
} from '../types/ipc';

const PROVIDERS: Array<{ id: string; label: string }> = [
  { id: 'claude_code', label: 'Claude Code' },
  { id: 'codex', label: 'Codex' },
  { id: 'antigravity', label: 'Antigravity' },
  { id: 'kimi_code', label: 'Kimi Code' },
  { id: 'workbuddy', label: 'Workbuddy' },
  { id: 'deepseek_harness', label: 'DeepSeek Harness' },
  { id: 'qcode', label: 'QCode' },
  { id: 'minimax_code', label: 'MiniMax Code' },
];

interface WindowFormDraft {
  window_id: string;
  label: string;
  duration_hours: string;
  rolling: boolean;
  unit: QuotaUnit;
  limit_input: string;
  warning_percent: string;
  critical_percent: string;
}

interface PlanFormDraft {
  mode: 'create' | 'edit';
  plan_id: string;
  provider_id: string;
  plan_name: string;
  enabled: boolean;
  monthly_fee_input: string;
  windows: WindowFormDraft[];
}

function getSafeExchangeRate(rateStr: string): number {
  const rate = Number.parseFloat(rateStr);
  return Number.isFinite(rate) && rate > 0 ? rate : 7.25;
}

function microUsdToCurrencyInput(
  microUsd: number,
  currency: CurrencyCode,
  usdToCnyRate: string
): string {
  if (!Number.isFinite(microUsd) || microUsd <= 0) {
    return '0';
  }
  const usd = microUsd / 1_000_000;
  const converted = currency === 'CNY' ? usd * getSafeExchangeRate(usdToCnyRate) : usd;
  return Number(converted.toFixed(2)).toString();
}

function currencyInputToMicroUsd(
  input: string,
  currency: CurrencyCode,
  usdToCnyRate: string
): number | null {
  const trimmed = input.trim();
  if (!trimmed) {
    return 0;
  }
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return null;
  }
  const usd = currency === 'CNY' ? parsed / getSafeExchangeRate(usdToCnyRate) : parsed;
  return Math.max(0, Math.round(usd * 1_000_000));
}

function buildDefaultWindowRules(): SubscriptionWindowRule[] {
  return [
    {
      window_id: '5h',
      label: '5h 滚动窗口',
      duration_hours: 5,
      rolling: true,
      unit: 'micro_usd',
      limit_value: 20_000_000,
      warning_percent: 80,
      critical_percent: 95,
    },
    {
      window_id: '7d',
      label: '7d 周额度',
      duration_hours: 168,
      rolling: true,
      unit: 'micro_usd',
      limit_value: 100_000_000,
      warning_percent: 80,
      critical_percent: 95,
    },
  ];
}

function windowsToDrafts(
  windows: SubscriptionWindowRule[],
  currency: CurrencyCode,
  usdToCnyRate: string
): WindowFormDraft[] {
  return windows.map((w) => ({
    window_id: w.window_id,
    label: w.label,
    duration_hours: String(w.duration_hours),
    rolling: w.rolling,
    unit: w.unit,
    limit_input:
      w.unit === 'micro_usd'
        ? microUsdToCurrencyInput(w.limit_value, currency, usdToCnyRate)
        : String(w.limit_value),
    warning_percent: String(w.warning_percent),
    critical_percent: String(w.critical_percent),
  }));
}

function formatObservedWindowLabel(obs: ObservedRateLimitWindow): string {
  if (obs.window_minutes !== null && Number.isFinite(obs.window_minutes) && obs.window_minutes > 0) {
    if (obs.window_minutes % 1440 === 0) {
      return `${obs.window_minutes / 1440}d 官方限额`;
    }
    if (obs.window_minutes % 60 === 0) {
      return `${obs.window_minutes / 60}h 官方限额`;
    }
    return `${obs.window_minutes}m 官方限额`;
  }
  const lower = obs.window_name.toLowerCase();
  if (lower.includes('primary') || lower.includes('5h')) {
    return '5h 官方限额';
  }
  if (lower.includes('secondary') || lower.includes('7d')) {
    return '7d 官方限额';
  }
  return `${obs.window_name} 官方限额`;
}

function observedWindowSortKey(obs: ObservedRateLimitWindow): number {
  if (obs.window_minutes !== null && Number.isFinite(obs.window_minutes)) {
    return obs.window_minutes;
  }
  const lower = obs.window_name.toLowerCase();
  if (lower.includes('primary') || lower.includes('5h')) return 300;
  if (lower.includes('secondary') || lower.includes('7d')) return 10080;
  return 999999;
}

export function SubscriptionsPage() {
  const queryClient = useQueryClient();
  const { currency, usdToCnyRate } = useWorkspaceStore();

  const [draft, setDraft] = useState<PlanFormDraft | null>(null);
  const [selectedTemplateId, setSelectedTemplateId] = useState<string>('custom');
  const [templateProviderFilter, setTemplateProviderFilter] = useState<string>('all');
  const [formError, setFormError] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const { data: plans = [], isLoading } = useQuery({
    queryKey: ['subscriptions'],
    queryFn: () => ipc.getSubscriptions(),
    refetchInterval: 10_000,
  });

  const { data: observedRateLimits = [] } = useQuery({
    queryKey: ['observed_rate_limits'],
    queryFn: () => ipc.getObservedRateLimits(),
    refetchInterval: 10_000,
  });

  const { data: templates = [] } = useQuery({
    queryKey: ['subscription_templates'],
    queryFn: () => ipc.getSubscriptionTemplates(),
    staleTime: Infinity,
  });

  const { data: remoteSnapshots = [] } = useQuery({
    queryKey: ['remote-subscriptions'],
    queryFn: () => ipc.getRemoteSubscriptionSnapshots(),
    refetchInterval: 15_000,
  });

  const remoteDeviceGroups = useMemo(() => {
    const safeSnapshots = Array.isArray(remoteSnapshots) ? remoteSnapshots : [];
    const grouped = new Map<
      string,
      { device_id: string; device_display_name: string; items: RemoteSubscriptionSnapshot[] }
    >();
    for (const snap of safeSnapshots) {
      const existing = grouped.get(snap.device_id);
      if (existing) {
        existing.items.push(snap);
      } else {
        grouped.set(snap.device_id, {
          device_id: snap.device_id,
          device_display_name: snap.device_display_name || snap.device_id,
          items: [snap],
        });
      }
    }
    return Array.from(grouped.values());
  }, [remoteSnapshots]);

  const configuredPlans = useMemo(
    () => plans.filter((s) => s.plan.is_user_configured),
    [plans]
  );

  const enabledConfiguredProviders = useMemo(
    () =>
      new Set(
        configuredPlans
          .filter((s) => s.plan.enabled)
          .map((s) => s.plan.provider_id.toLowerCase())
      ),
    [configuredPlans]
  );

  const unboundObservedGroups = useMemo(() => {
    const grouped = new Map<string, ObservedRateLimitWindow[]>();
    for (const obs of observedRateLimits) {
      if (enabledConfiguredProviders.has(obs.provider_id.toLowerCase())) {
        continue;
      }
      const list = grouped.get(obs.provider_id) ?? [];
      list.push(obs);
      grouped.set(obs.provider_id, list);
    }
    return Array.from(grouped.entries()).map(([providerId, windows]) => ({
      provider_id: providerId,
      windows: [...windows].sort(
        (a, b) => observedWindowSortKey(a) - observedWindowSortKey(b)
      ),
    }));
  }, [observedRateLimits, enabledConfiguredProviders]);

  const invalidateSubscriptionQueries = async () => {
    await queryClient.invalidateQueries({ queryKey: ['subscriptions'] });
    await queryClient.invalidateQueries({ queryKey: ['monthly'] });
    await queryClient.invalidateQueries({ queryKey: ['overview'] });
    await queryClient.invalidateQueries({ queryKey: ['tray_summary'] });
    await queryClient.invalidateQueries({ queryKey: ['tray-summary'] });
    await queryClient.invalidateQueries({ queryKey: ['observed_rate_limits'] });
  };

  const savePlanMutation = useMutation({
    mutationFn: (plan: SubscriptionPlan) => ipc.upsertSubscriptionPlan(plan),
    onSuccess: async () => {
      setDraft(null);
      setFormError(null);
      await invalidateSubscriptionQueries();
    },
    onError: (err: unknown) => {
      setFormError(err instanceof Error ? err.message : String(err));
    },
  });

  const deletePlanMutation = useMutation({
    mutationFn: (planId: string) => ipc.deleteSubscriptionPlan(planId),
    onSuccess: async () => {
      setConfirmDeleteId(null);
      await invalidateSubscriptionQueries();
    },
  });

  const openCreateDrawer = (preselectedProviderId?: string) => {
    const providerId = preselectedProviderId ?? 'claude_code';
    setTemplateProviderFilter(preselectedProviderId ?? 'all');
    setSelectedTemplateId('custom');
    setFormError(null);
    setDraft({
      mode: 'create',
      plan_id: `sub_${providerId}_${Date.now()}`,
      provider_id: providerId,
      plan_name: '',
      enabled: true,
      monthly_fee_input: '0',
      windows: windowsToDrafts(buildDefaultWindowRules(), currency, usdToCnyRate),
    });
  };

  const openEditDrawer = (plan: SubscriptionPlan) => {
    setTemplateProviderFilter('all');
    setSelectedTemplateId('custom');
    setFormError(null);
    setDraft({
      mode: 'edit',
      plan_id: plan.plan_id,
      provider_id: plan.provider_id,
      plan_name: plan.plan_name,
      enabled: plan.enabled,
      monthly_fee_input: microUsdToCurrencyInput(
        plan.monthly_fee_micro_usd,
        currency,
        usdToCnyRate
      ),
      windows: windowsToDrafts(
        plan.windows.length > 0 ? plan.windows : buildDefaultWindowRules(),
        currency,
        usdToCnyRate
      ),
    });
  };

  const applyTemplateToDraft = (tpl: SubscriptionPlanTemplate) => {
    if (!draft) return;
    setSelectedTemplateId(tpl.template_id);
    setFormError(null);
    setDraft({
      ...draft,
      plan_id:
        draft.mode === 'create'
          ? `sub_${tpl.provider_id}_${tpl.template_id}_${Date.now()}`
          : draft.plan_id,
      provider_id: tpl.provider_id,
      plan_name: tpl.plan_name,
      monthly_fee_input: microUsdToCurrencyInput(
        tpl.monthly_fee_micro_usd,
        currency,
        usdToCnyRate
      ),
      windows: windowsToDrafts(tpl.windows, currency, usdToCnyRate),
    });
  };

  const applyCustomTemplateToDraft = () => {
    if (!draft) return;
    setSelectedTemplateId('custom');
    setFormError(null);
    const activeProvider =
      templateProviderFilter !== 'all' ? templateProviderFilter : draft.provider_id;
    setDraft({
      ...draft,
      provider_id: activeProvider,
      plan_name: draft.mode === 'create' ? '' : draft.plan_name,
      monthly_fee_input: draft.mode === 'create' ? '0' : draft.monthly_fee_input,
      windows:
        draft.mode === 'create'
          ? windowsToDrafts(buildDefaultWindowRules(), currency, usdToCnyRate)
          : draft.windows,
    });
  };

  const toggleEnabled = (status: SubscriptionPlanStatus) => {
    savePlanMutation.mutate({
      ...status.plan,
      enabled: !status.plan.enabled,
      is_user_configured: true,
    });
  };

  const handleSaveDraft = () => {
    if (!draft) return;
    const trimmedName = draft.plan_name.trim();
    if (!trimmedName) {
      setFormError('请输入订阅套餐名称（例如 Claude Max 5x 或 自定义订阅）');
      return;
    }
    const monthlyFeeMicroUsd = currencyInputToMicroUsd(
      draft.monthly_fee_input,
      currency,
      usdToCnyRate
    );
    if (monthlyFeeMicroUsd === null) {
      setFormError('订阅月费格式无效，请输入大于或等于 0 的数字');
      return;
    }

    const parsedWindows: SubscriptionWindowRule[] = [];
    for (let i = 0; i < draft.windows.length; i += 1) {
      const w = draft.windows[i];
      const durationHours = Number.parseInt(w.duration_hours, 10);
      if (!Number.isFinite(durationHours) || durationHours < 1) {
        setFormError(`窗口 #${i + 1} 周期时长必须为至少 1 小时`);
        return;
      }

      let limitValue: number;
      if (w.unit === 'micro_usd') {
        const parsedLimit = currencyInputToMicroUsd(w.limit_input, currency, usdToCnyRate);
        if (parsedLimit === null || parsedLimit < 1) {
          setFormError(`窗口 #${i + 1} 估算等效额度必须大于 0`);
          return;
        }
        limitValue = parsedLimit;
      } else {
        const parsedTokens = Number.parseInt(w.limit_input, 10);
        if (!Number.isFinite(parsedTokens) || parsedTokens < 1) {
          setFormError(`窗口 #${i + 1} Token 额度必须大于 0`);
          return;
        }
        limitValue = parsedTokens;
      }

      const warnPct = Number.parseInt(w.warning_percent, 10);
      const critPct = Number.parseInt(w.critical_percent, 10);
      if (
        !Number.isFinite(warnPct) ||
        warnPct < 1 ||
        warnPct > 99 ||
        !Number.isFinite(critPct) ||
        critPct < 1 ||
        critPct > 100
      ) {
        setFormError(`窗口 #${i + 1} 预警阈值需在 1%–100% 范围内`);
        return;
      }

      parsedWindows.push({
        window_id: w.window_id,
        label: w.label.trim() || `${durationHours}h 滚动窗口`,
        duration_hours: durationHours,
        rolling: w.rolling,
        unit: w.unit,
        limit_value: limitValue,
        warning_percent: Math.min(warnPct, critPct),
        critical_percent: Math.max(warnPct, critPct),
      });
    }

    setFormError(null);
    savePlanMutation.mutate({
      plan_id: draft.plan_id,
      provider_id: draft.provider_id,
      plan_name: trimmedName,
      enabled: draft.enabled,
      monthly_fee_micro_usd: monthlyFeeMicroUsd,
      windows: parsedWindows,
      is_user_configured: true,
      updated_at_ms: Date.now(),
    });
  };

  const visibleTemplates = useMemo(() => {
    if (templateProviderFilter === 'all') {
      return templates;
    }
    return templates.filter((t) => t.provider_id === templateProviderFilter);
  }, [templates, templateProviderFilter]);

  const draftFeeMicroPreview = draft
    ? currencyInputToMicroUsd(draft.monthly_fee_input, currency, usdToCnyRate)
    : null;

  return (
    <div className="space-y-7">
      {/* Header */}
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div>
          <h1 className="text-[16px] font-semibold tracking-tight">订阅配额与滚动窗口预警</h1>
          <p className="text-[12px]" style={{ color: 'var(--text-muted)' }}>
            严守零假设原则：绝不猜测未配置套餐的档位或月费；仅核算您主动录入的订阅与官方日志实际记录的限额快照
          </p>
        </div>

        <button
          type="button"
          onClick={() => openCreateDrawer()}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded text-[12px] font-medium cursor-pointer transition-opacity hover:opacity-90 active:scale-[0.985]"
          style={{
            backgroundColor: 'var(--accent-primary)',
            color: '#ffffff',
          }}
        >
          <Plus className="w-3.5 h-3.5" />
          <span>添加我的订阅</span>
        </button>
      </section>

      {isLoading ? (
        <div className="py-12 text-center" style={{ color: 'var(--text-muted)' }}>
          计算滚动窗口配额中…
        </div>
      ) : (
        <>
          {/* Section 1: User-Configured Subscriptions (or Clean Empty State) */}
          <section className="space-y-2">
            <div className="flex items-center justify-between">
              <h2
                className="text-[11px] font-semibold uppercase tracking-wider"
                style={{ color: 'var(--text-muted)' }}
              >
                我的订阅配置
              </h2>
              <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                {configuredPlans.length} 项已配置
              </span>
            </div>

            {configuredPlans.length === 0 ? (
              <div
                className="py-8 px-5 flex flex-col md:flex-row md:items-center md:justify-between gap-5"
                style={{
                  borderTop: '1px solid var(--border-subtle)',
                  borderBottom: '1px solid var(--border-subtle)',
                  backgroundColor: 'var(--bg-elevated)',
                }}
              >
                <div className="space-y-1.5 max-w-2xl">
                  <div className="flex items-center gap-2 text-[13.5px] font-semibold">
                    <Info className="w-4 h-4 shrink-0" style={{ color: 'var(--accent-primary)' }} />
                    <span>尚未添加个人订阅配置</span>
                  </div>
                  <p className="text-[12px] leading-relaxed" style={{ color: 'var(--text-secondary)' }}>
                    TokenTracker 仅通过本地只读日志与官方公开账单接口核算用量，绝不探测私有账号接口或臆测您的订阅档位与月费。
                    如果您购买了固定月费订阅（如 Claude Pro / Max、ChatGPT Plus / Pro、Google AI Pro / Ultra），可点击右侧按钮添加并设定 5h / 7d 预警规则；若仅需查看按量等效金额，无需配置订阅。
                  </p>
                </div>

                <button
                  type="button"
                  onClick={() => openCreateDrawer()}
                  className="inline-flex items-center gap-1.5 px-3.5 py-2 rounded text-[12px] font-medium shrink-0 cursor-pointer transition-opacity hover:opacity-90 active:scale-[0.985]"
                  style={{
                    backgroundColor: 'var(--accent-primary)',
                    color: '#ffffff',
                  }}
                >
                  <Plus className="w-3.5 h-3.5" />
                  <span>添加我的订阅</span>
                </button>
              </div>
            ) : (
              <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
                {configuredPlans.map((status) => {
                  const { plan, windows } = status;
                  const confirmingDelete = confirmDeleteId === plan.plan_id;

                  return (
                    <div
                      key={plan.plan_id}
                      className="row-interactive py-5 px-2 grid grid-cols-12 gap-5 items-center"
                      style={{
                        borderBottom: '1px solid var(--border-hairline)',
                        opacity: plan.enabled ? 1 : 0.58,
                      }}
                    >
                      {/* Left Column: Provider + Plan Identity */}
                      <div className="col-span-12 lg:col-span-3 space-y-1.5">
                        <div className="flex items-center gap-2">
                          <span
                            className="inline-block w-2 h-2 rounded-full shrink-0"
                            style={{ backgroundColor: providerAccentColor(plan.provider_id) }}
                          />
                          <span
                            className="text-[11px] uppercase tracking-wider"
                            style={{ color: 'var(--text-muted)' }}
                          >
                            {providerDisplayName(plan.provider_id)}
                          </span>
                        </div>
                        <div className="text-[14px] font-semibold">{plan.plan_name}</div>
                        <div
                          className="flex flex-wrap items-center gap-2 text-[11.5px] tabular-nums"
                          style={{ color: 'var(--text-secondary)' }}
                        >
                          <span>
                            {plan.monthly_fee_micro_usd > 0
                              ? `月费 ${formatCost(plan.monthly_fee_micro_usd, currency, usdToCnyRate)}`
                              : '未设月费（不算 ROI）'}
                          </span>
                          <button
                            type="button"
                            onClick={() => toggleEnabled(status)}
                            disabled={savePlanMutation.isPending}
                            className="btn-interactive bg-[var(--bg-elevated)] px-2 py-0.5 rounded text-[11px] cursor-pointer disabled:opacity-50"
                            style={{
                              border: '1px solid var(--border-hairline)',
                              color: plan.enabled ? 'var(--accent-emerald)' : 'var(--text-muted)',
                            }}
                          >
                            {plan.enabled ? '已启用' : '已停用'}
                          </button>
                        </div>
                      </div>

                      {/* Center Column: 5h / 7d Inline Progress Bars */}
                      <div className="col-span-12 lg:col-span-6 space-y-3">
                        {windows.map((w) => {
                          const usedStr =
                            w.unit === 'micro_usd'
                              ? formatCost(w.used_value, currency, usdToCnyRate)
                              : formatTokens(w.used_value);
                          const limitStr =
                            w.unit === 'micro_usd'
                              ? formatCost(w.limit_value, currency, usdToCnyRate)
                              : formatTokens(w.limit_value);
                          return (
                            <div key={w.window_id} className="space-y-1">
                              <div className="flex items-center justify-between gap-2 text-[12px] tabular-nums">
                                <div className="flex items-center gap-2 min-w-0">
                                  <span className="font-medium truncate">{w.label}</span>
                                  <span
                                    className="px-1.5 py-0.5 text-[10px] rounded shrink-0"
                                    style={{
                                      backgroundColor: 'var(--bg-elevated)',
                                      border: '1px solid var(--border-hairline)',
                                      color:
                                        w.source === 'official_log_snapshot'
                                          ? 'var(--accent-emerald)'
                                          : 'var(--text-muted)',
                                    }}
                                  >
                                    {w.source === 'official_log_snapshot' ? '官方日志快照' : '本地推算'}
                                  </span>
                                </div>
                                <div className="shrink-0">
                                  <span style={{ color: 'var(--text-secondary)' }}>
                                    {usedStr} / {limitStr}
                                  </span>
                                  <span className="ml-2 font-semibold">
                                    {w.used_percent.toFixed(1)}%
                                  </span>
                                </div>
                              </div>
                              <InlineProgressBar
                                percent={w.used_percent}
                                alertLevel={w.alert_level}
                                heightPx={6}
                              />
                            </div>
                          );
                        })}
                      </div>

                      {/* Right Column: Burn Rate, Reset Countdown, Edit & Delete */}
                      <div className="col-span-12 lg:col-span-3 flex items-center justify-between gap-3">
                        <div className="space-y-2 text-[11.5px] tabular-nums">
                          {windows.map((w) => {
                            const burnRateStr =
                              w.unit === 'micro_usd'
                                ? `${formatCost(w.burn_rate_per_hour, currency, usdToCnyRate)}/h`
                                : `${formatTokens(w.burn_rate_per_hour)}/h`;
                            return (
                              <div key={w.window_id} className="space-y-0.5">
                                <div style={{ color: 'var(--text-secondary)' }}>
                                  {w.window_id} 重置:{' '}
                                  <strong style={{ color: 'var(--text-primary)' }}>
                                    {formatCountdown(w.resets_in_seconds)}
                                  </strong>
                                </div>
                                <div style={{ color: 'var(--text-muted)' }}>
                                  流速 {burnRateStr} ·{' '}
                                  {w.projected_exhaustion_hours !== null
                                    ? w.projected_exhaustion_hours <= 0
                                      ? '已触及上限'
                                      : `约 ${w.projected_exhaustion_hours.toFixed(1)}h 触顶`
                                    : '稳定'}
                                </div>
                              </div>
                            );
                          })}
                        </div>

                        <div className="flex items-center gap-1.5 shrink-0">
                          {confirmingDelete ? (
                            <div className="flex items-center gap-1">
                              <button
                                type="button"
                                disabled={deletePlanMutation.isPending}
                                onClick={() => deletePlanMutation.mutate(plan.plan_id)}
                                className="inline-flex items-center gap-1 px-2 py-1 rounded text-[11px] font-medium cursor-pointer transition-opacity hover:opacity-90 active:scale-[0.985] disabled:opacity-50"
                                style={{
                                  backgroundColor: 'var(--accent-rose)',
                                  color: '#ffffff',
                                }}
                              >
                                <Check className="w-3 h-3" />
                                <span>确认</span>
                              </button>
                              <button
                                type="button"
                                onClick={() => setConfirmDeleteId(null)}
                                className="btn-interactive bg-[var(--bg-elevated)] px-2 py-1 rounded text-[11px] cursor-pointer"
                                style={{
                                  border: '1px solid var(--border-subtle)',
                                  color: 'var(--text-secondary)',
                                }}
                              >
                                取消
                              </button>
                            </div>
                          ) : (
                            <>
                              <button
                                type="button"
                                onClick={() => openEditDrawer(plan)}
                                className="btn-interactive bg-[var(--bg-elevated)] p-1.5 rounded cursor-pointer"
                                style={{
                                  border: '1px solid var(--border-subtle)',
                                  color: 'var(--text-secondary)',
                                }}
                                title="编辑订阅推算规则"
                                aria-label={`编辑 ${plan.plan_name}`}
                              >
                                <Pencil className="w-3.5 h-3.5" />
                              </button>
                              <button
                                type="button"
                                onClick={() => setConfirmDeleteId(plan.plan_id)}
                                className="btn-interactive bg-[var(--bg-elevated)] p-1.5 rounded cursor-pointer"
                                style={{
                                  border: '1px solid var(--border-subtle)',
                                  color: 'var(--accent-rose)',
                                }}
                                title="删除该订阅配置"
                                aria-label={`删除 ${plan.plan_name}`}
                              >
                                <Trash2 className="w-3.5 h-3.5" />
                              </button>
                            </>
                          )}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </section>

          {/* Section 2: Unbound Official Log Observed Rate Limits */}
          {unboundObservedGroups.length > 0 && (
            <section className="space-y-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <h2
                    className="text-[11px] font-semibold uppercase tracking-wider"
                    style={{ color: 'var(--text-muted)' }}
                  >
                    官方日志可观测限额快照（未绑定月费套餐）
                  </h2>
                  <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                    直接忠实展示本地会话日志中的官方限额百分比与重置倒计时，不推测您的具体套餐档位或月费
                  </p>
                </div>
              </div>

              <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
                {unboundObservedGroups.map((group) => (
                  <div
                    key={group.provider_id}
                    className="row-interactive py-5 px-2 grid grid-cols-12 gap-5 items-center"
                    style={{ borderBottom: '1px solid var(--border-hairline)' }}
                  >
                    {/* Left: Provider Identity ONLY (no guessed plan name / fee) */}
                    <div className="col-span-12 lg:col-span-3 space-y-1.5">
                      <div className="flex items-center gap-2">
                        <span
                          className="inline-block w-2 h-2 rounded-full shrink-0"
                          style={{ backgroundColor: providerAccentColor(group.provider_id) }}
                        />
                        <span className="text-[14px] font-semibold">
                          {providerDisplayName(group.provider_id)}
                        </span>
                      </div>
                      <div className="flex items-center gap-2">
                        <span
                          className="px-1.5 py-0.5 rounded text-[10.5px]"
                          style={{
                            backgroundColor: 'var(--bg-elevated)',
                            border: '1px solid var(--border-hairline)',
                            color: 'var(--accent-emerald)',
                          }}
                        >
                          官方日志可观测 · 未绑定月费套餐
                        </span>
                      </div>
                    </div>

                    {/* Center: Observed Window Progress Bars */}
                    <div className="col-span-12 lg:col-span-6 space-y-3">
                      {group.windows.map((obs) => {
                        const pct = Number.isFinite(obs.used_percent)
                          ? Math.max(0, Math.min(100, obs.used_percent))
                          : 0;
                        const label = formatObservedWindowLabel(obs);
                        return (
                          <div key={`${obs.provider_id}-${obs.window_name}`} className="space-y-1">
                            <div className="flex items-center justify-between text-[12px] tabular-nums">
                              <span className="font-medium">{label}</span>
                              <span className="font-semibold">{pct.toFixed(1)}%</span>
                            </div>
                            <InlineProgressBar percent={pct} heightPx={6} />
                          </div>
                        );
                      })}
                    </div>

                    {/* Right: Reset Countdown & + Bind Action */}
                    <div className="col-span-12 lg:col-span-3 flex items-center justify-between gap-3">
                      <div className="space-y-1.5 text-[11.5px] tabular-nums">
                        {group.windows.map((obs) => {
                          const resetSecs =
                            obs.resets_at_ms !== null
                              ? Math.max(0, Math.floor((obs.resets_at_ms - Date.now()) / 1000))
                              : null;
                          return (
                            <div key={`${obs.provider_id}-${obs.window_name}-reset`}>
                              <div style={{ color: 'var(--text-secondary)' }}>
                                {formatObservedWindowLabel(obs).replace(' 官方限额', '')} 重置:{' '}
                                <strong style={{ color: 'var(--text-primary)' }}>
                                  {resetSecs !== null ? formatCountdown(resetSecs) : '未知'}
                                </strong>
                              </div>
                              <div style={{ color: 'var(--text-muted)' }}>
                                观测于 {formatRelativeTime(obs.observed_at_ms)}
                              </div>
                            </div>
                          );
                        })}
                      </div>

                      <button
                        type="button"
                        onClick={() => openCreateDrawer(group.provider_id)}
                        className="btn-interactive bg-[var(--bg-elevated)] inline-flex items-center gap-1 px-2.5 py-1.5 rounded text-[11.5px] font-medium shrink-0 cursor-pointer"
                        style={{
                          border: '1px solid var(--border-strong)',
                          color: 'var(--text-primary)',
                        }}
                      >
                        <Plus className="w-3.5 h-3.5" />
                        <span>绑定订阅</span>
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}

          {/* Section 3: Cross-Device Synced Remote Subscription Snapshots (Read-only) */}
          {remoteDeviceGroups.length > 0 && (
            <section className="space-y-2">
              <div>
                <h2
                  className="text-[11px] font-semibold uppercase tracking-wider"
                  style={{ color: 'var(--text-muted)' }}
                >
                  跨设备同步配额快照（只读）
                </h2>
                <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                  来自 Cloudflare Worker 同步的远端设备订阅窗口状态，与本机可编辑套餐完全隔离展示
                </p>
              </div>

              <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
                {remoteDeviceGroups.map((devGroup) => (
                  <div
                    key={devGroup.device_id}
                    className="py-5 px-2 grid grid-cols-12 gap-5 items-start"
                    style={{ borderBottom: '1px solid var(--border-hairline)' }}
                  >
                    <div className="col-span-12 lg:col-span-3 space-y-1">
                      <div className="text-[14px] font-semibold">{devGroup.device_display_name}</div>
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span
                          className="px-1.5 py-0.5 rounded text-[10.5px]"
                          style={{
                            backgroundColor: 'var(--bg-elevated)',
                            border: '1px solid var(--border-hairline)',
                            color: 'var(--accent-sky)',
                          }}
                        >
                          只读远端快照
                        </span>
                        <span
                          className="text-[10.5px] font-mono"
                          style={{ color: 'var(--text-muted)' }}
                        >
                          {devGroup.device_id.slice(0, 8)}
                        </span>
                      </div>
                    </div>

                    <div className="col-span-12 lg:col-span-9 space-y-3">
                      {devGroup.items.map((snap) => {
                        const pct = Number.isFinite(snap.used_percent)
                          ? Math.max(0, Math.min(100, snap.used_percent))
                          : 0;
                        const resetSecs =
                          snap.resets_at_ms !== null
                            ? Math.max(0, Math.floor((snap.resets_at_ms - Date.now()) / 1000))
                            : null;
                        return (
                          <div
                            key={`${snap.device_id}-${snap.plan_id}-${snap.window_id}`}
                            className="space-y-1"
                          >
                            <div className="flex flex-wrap items-center justify-between gap-2 text-[12px] tabular-nums">
                              <div className="flex items-center gap-2 min-w-0">
                                <span
                                  className="inline-block w-2 h-2 rounded-full shrink-0"
                                  style={{ backgroundColor: providerAccentColor(snap.provider_id) }}
                                />
                                <span className="font-medium">
                                  {providerDisplayName(snap.provider_id)} · {snap.plan_name} ·{' '}
                                  {snap.window_label}
                                </span>
                              </div>
                              <div className="flex items-center gap-3">
                                <span style={{ color: 'var(--text-muted)' }}>
                                  {resetSecs !== null ? `重置 ${formatCountdown(resetSecs)} · ` : ''}
                                  同步于 {formatRelativeTime(snap.updated_at_ms)}
                                </span>
                                <span className="font-semibold">{pct.toFixed(1)}%</span>
                              </div>
                            </div>
                            <InlineProgressBar percent={pct} heightPx={6} />
                          </div>
                        );
                      })}
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}
        </>
      )}

      {/* Create / Edit Subscription Drawer */}
      {draft && (
        <div
          className="drawer-backdrop-enter fixed inset-0 z-50 flex justify-end"
          style={{ backgroundColor: 'rgba(0, 0, 0, 0.45)' }}
          onClick={(e) => {
            if (e.target === e.currentTarget) {
              setDraft(null);
            }
          }}
        >
          <div
            className="drawer-panel-enter w-full max-w-[480px] h-full p-5 overflow-y-auto space-y-5"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              borderLeft: '1px solid var(--border-strong)',
              color: 'var(--text-primary)',
            }}
          >
            {/* Drawer Header */}
            <div
              className="flex items-center justify-between pb-3"
              style={{ borderBottom: '1px solid var(--border-subtle)' }}
            >
              <div>
                <h2 className="text-[14.5px] font-semibold">
                  {draft.mode === 'create' ? '添加我的订阅配置' : '编辑订阅推算规则'}
                </h2>
                <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                  {draft.mode === 'create'
                    ? '可选择参考套餐模板快速预填，或完全自定义名称、月费与滚动窗口'
                    : draft.plan_name || '自定义订阅规则'}
                </p>
              </div>
              <button
                type="button"
                onClick={() => setDraft(null)}
                className="btn-interactive p-1.5 rounded cursor-pointer"
                style={{ color: 'var(--text-secondary)' }}
                aria-label="关闭抽屉"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* Template Quick-Fill Section */}
            <div
              className="pb-4 space-y-2.5"
              style={{ borderBottom: '1px solid var(--border-subtle)' }}
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <span className="text-[12px] font-medium">参考模板快速预填（可选）</span>
                <div className="flex items-center gap-1">
                  {[{ id: 'all', label: '全部' }, ...PROVIDERS].map((p) => (
                    <button
                      key={p.id}
                      type="button"
                      onClick={() => setTemplateProviderFilter(p.id)}
                      className="seg-pill px-2 py-0.5 rounded text-[11px] cursor-pointer"
                      data-active={templateProviderFilter === p.id ? 'true' : 'false'}
                      style={{
                        border:
                          templateProviderFilter === p.id
                            ? '1px solid var(--border-strong)'
                            : '1px solid transparent',
                        color:
                          templateProviderFilter === p.id
                            ? 'var(--text-primary)'
                            : 'var(--text-muted)',
                      }}
                    >
                      {p.label}
                    </button>
                  ))}
                </div>
              </div>

              <div className="grid grid-cols-2 gap-2">
                {visibleTemplates.map((tpl) => {
                  const active = selectedTemplateId === tpl.template_id;
                  return (
                    <button
                      key={tpl.template_id}
                      type="button"
                      onClick={() => applyTemplateToDraft(tpl)}
                      className={`btn-interactive text-left px-2.5 py-2 rounded space-y-0.5 cursor-pointer ${
                        active ? 'bg-[var(--bg-hover)]' : 'bg-[var(--bg-elevated)]'
                      }`}
                      style={{
                        border: active
                          ? '1px solid var(--accent-primary)'
                          : '1px solid var(--border-subtle)',
                      }}
                    >
                      <div className="flex items-center gap-1.5">
                        <span
                          className="inline-block w-1.5 h-1.5 rounded-full shrink-0"
                          style={{ backgroundColor: providerAccentColor(tpl.provider_id) }}
                        />
                        <span className="text-[11.5px] font-medium truncate">
                          {tpl.plan_name}
                        </span>
                      </div>
                      <div className="text-[10.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                        {providerDisplayName(tpl.provider_id)} ·{' '}
                        {formatCost(tpl.monthly_fee_micro_usd, currency, usdToCnyRate)}/月
                      </div>
                    </button>
                  );
                })}

                <button
                  type="button"
                  onClick={applyCustomTemplateToDraft}
                  className={`btn-interactive text-left px-2.5 py-2 rounded space-y-0.5 cursor-pointer ${
                    selectedTemplateId === 'custom'
                      ? 'bg-[var(--bg-hover)]'
                      : 'bg-[var(--bg-elevated)]'
                  }`}
                  style={{
                    border:
                      selectedTemplateId === 'custom'
                        ? '1px solid var(--accent-primary)'
                        : '1px solid var(--border-subtle)',
                  }}
                >
                  <div className="text-[11.5px] font-medium">自定义订阅</div>
                  <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                    手填名称、月费（可为 0）与窗口额度
                  </div>
                </button>
              </div>
            </div>

            {/* Form Fields */}
            <div className="space-y-4">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div>
                  <label
                    className="block text-[11.5px] mb-1"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    所属 Provider
                  </label>
                  <select
                    aria-label="所属 Provider"
                    value={draft.provider_id}
                    onChange={(e) => setDraft({ ...draft, provider_id: e.target.value })}
                    className="w-full px-2.5 py-1.5 text-[12px] rounded outline-none cursor-pointer"
                    style={{
                      backgroundColor: 'var(--bg-input)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  >
                    {PROVIDERS.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.label}
                      </option>
                    ))}
                  </select>
                </div>

                <div>
                  <label
                    className="block text-[11.5px] mb-1"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    订阅套餐名称
                  </label>
                  <input
                    type="text"
                    placeholder="例如 Claude Max 5x 或 Google AI Ultra"
                    value={draft.plan_name}
                    onChange={(e) => setDraft({ ...draft, plan_name: e.target.value })}
                    className="w-full px-2.5 py-1.5 text-[12px] rounded outline-none"
                    style={{
                      backgroundColor: 'var(--bg-input)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  />
                </div>
              </div>

              <div>
                <label
                  className="block text-[11.5px] mb-1"
                  style={{ color: 'var(--text-secondary)' }}
                >
                  订阅月费 ({currency}，填 0 则不计算月度回本率)
                </label>
                <div className="flex items-center gap-2">
                  <input
                    type="text"
                    inputMode="decimal"
                    placeholder="0"
                    value={draft.monthly_fee_input}
                    onChange={(e) => setDraft({ ...draft, monthly_fee_input: e.target.value })}
                    className="w-36 px-2.5 py-1.5 text-[12px] rounded tabular-nums outline-none"
                    style={{
                      backgroundColor: 'var(--bg-input)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  />
                  <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                    {currency === 'CNY' && draftFeeMicroPreview !== null && draftFeeMicroPreview > 0
                      ? `≈ ${formatCost(draftFeeMicroPreview, 'USD', usdToCnyRate)} USD / 月（按汇率 1:${getSafeExchangeRate(usdToCnyRate)} 换算）`
                      : '仅当月费 > 0 时展示月度 ROI 回本参考'}
                  </span>
                </div>
              </div>

              {/* Rolling Windows */}
              {draft.windows.map((win, idx) => (
                <div
                  key={`${win.window_id}-${idx}`}
                  className="pt-3.5 space-y-2.5"
                  style={{ borderTop: '1px solid var(--border-subtle)' }}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-[12.5px] font-medium">
                      窗口 #{idx + 1} · {win.label}
                    </span>
                    <select
                      aria-label={`窗口 ${idx + 1} 计量单位`}
                      value={win.unit}
                      onChange={(e) => {
                        const nextUnit = e.target.value as QuotaUnit;
                        const nextWindows = [...draft.windows];
                        nextWindows[idx] = {
                          ...win,
                          unit: nextUnit,
                          limit_input:
                            nextUnit === 'micro_usd'
                              ? microUsdToCurrencyInput(20_000_000, currency, usdToCnyRate)
                              : '5000000',
                        };
                        setDraft({ ...draft, windows: nextWindows });
                      }}
                      className="px-2 py-0.5 text-[11px] rounded outline-none cursor-pointer"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-secondary)',
                      }}
                    >
                      <option value="micro_usd">等效金额 ({currency})</option>
                      <option value="tokens">Token 用量</option>
                    </select>
                  </div>

                  <div className="grid grid-cols-2 gap-2.5">
                    <div>
                      <label className="block text-[11px] mb-1" style={{ color: 'var(--text-muted)' }}>
                        周期时长 (小时)
                      </label>
                      <input
                        type="number"
                        min={1}
                        value={win.duration_hours}
                        onChange={(e) => {
                          const next = [...draft.windows];
                          next[idx] = { ...win, duration_hours: e.target.value };
                          setDraft({ ...draft, windows: next });
                        }}
                        className="w-full px-2 py-1 text-[12px] rounded tabular-nums outline-none"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                    </div>

                    <div>
                      <label className="block text-[11px] mb-1" style={{ color: 'var(--text-muted)' }}>
                        {win.unit === 'micro_usd'
                          ? `估算等效额度 (${currency})`
                          : '估算额度 (Tokens)'}
                      </label>
                      <input
                        type="text"
                        inputMode="decimal"
                        value={win.limit_input}
                        onChange={(e) => {
                          const next = [...draft.windows];
                          next[idx] = { ...win, limit_input: e.target.value };
                          setDraft({ ...draft, windows: next });
                        }}
                        className="w-full px-2 py-1 text-[12px] rounded tabular-nums outline-none"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                    </div>

                    <div>
                      <label className="block text-[11px] mb-1" style={{ color: 'var(--text-muted)' }}>
                        警告阈值 (%)
                      </label>
                      <input
                        type="number"
                        min={1}
                        max={99}
                        value={win.warning_percent}
                        onChange={(e) => {
                          const next = [...draft.windows];
                          next[idx] = { ...win, warning_percent: e.target.value };
                          setDraft({ ...draft, windows: next });
                        }}
                        className="w-full px-2 py-1 text-[12px] rounded tabular-nums outline-none"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                    </div>

                    <div>
                      <label className="block text-[11px] mb-1" style={{ color: 'var(--text-muted)' }}>
                        严重预警阈值 (%)
                      </label>
                      <input
                        type="number"
                        min={1}
                        max={100}
                        value={win.critical_percent}
                        onChange={(e) => {
                          const next = [...draft.windows];
                          next[idx] = { ...win, critical_percent: e.target.value };
                          setDraft({ ...draft, windows: next });
                        }}
                        className="w-full px-2 py-1 text-[12px] rounded tabular-nums outline-none"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                    </div>
                  </div>
                </div>
              ))}

              {formError && (
                <div
                  className="px-3 py-2 rounded text-[11.5px]"
                  style={{
                    backgroundColor: 'var(--bg-elevated)',
                    border: '1px solid var(--accent-rose)',
                    color: 'var(--accent-rose)',
                  }}
                >
                  {formError}
                </div>
              )}

              <div
                className="pt-3 flex items-center gap-2"
                style={{ borderTop: '1px solid var(--border-subtle)' }}
              >
                <button
                  type="button"
                  disabled={savePlanMutation.isPending}
                  onClick={handleSaveDraft}
                  className="px-3.5 py-1.5 text-[12px] font-medium rounded cursor-pointer transition-opacity hover:opacity-90 active:scale-[0.985] disabled:opacity-50"
                  style={{
                    backgroundColor: 'var(--accent-primary)',
                    color: '#ffffff',
                  }}
                >
                  {savePlanMutation.isPending ? '保存中…' : '保存订阅配置'}
                </button>
                <button
                  type="button"
                  onClick={() => setDraft(null)}
                  className="btn-interactive bg-[var(--bg-elevated)] px-3 py-1.5 text-[12px] rounded cursor-pointer"
                  style={{
                    border: '1px solid var(--border-subtle)',
                    color: 'var(--text-secondary)',
                  }}
                >
                  取消
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
