import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, Info, Pencil, Plus, RefreshCw, Trash2, Wallet, X } from 'lucide-react';
import { InlineProgressBar } from '../components/InlineProgressBar';
import {
  formatCost,
  formatCountdown,
  formatRelativeTime,
  formatTokens,
  providerAccentColor,
  providerDisplayName,
  resolveSubscriptionTierLabel,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type {
  CurrencyCode,
  ObservedRateLimitWindow,
  PaygAccount,
  PaygAccountStatus,
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
  { id: 'cursor', label: 'Cursor' },
  { id: 'github_copilot', label: 'GitHub Copilot' },
  { id: 'windsurf', label: 'Windsurf' },
];

interface PlanFormDraft {
  mode: 'create' | 'edit';
  plan_id: string;
  provider_id: string;
  plan_name: string;
  enabled: boolean;
  monthly_fee_input: string;
  windows: SubscriptionWindowRule[];
}

interface PaygFormDraft {
  mode: 'create' | 'edit';
  account_id: string;
  provider_id: string;
  account_name: string;
  enabled: boolean;
  is_auto: boolean;
  api_key_input: string;
  manual_balance_input: string;
  initial_manual_balance_micro_usd: number | null;
  existing_anchor_ms: number | null;
  monthly_budget_input: string;
}

const PAYG_OFFICIAL_BALANCE_PROVIDERS = new Set([
  'deepseek_harness',
  'kimi_code',
  'qcode',
  'minimax_code',
]);

function defaultPaygAccountName(providerId: string): string {
  switch (providerId) {
    case 'deepseek_harness':
      return 'DeepSeek 官方按量账户';
    case 'codex':
      return 'Codex Credits';
    case 'claude_code':
      return 'Claude Console 按量账户';
    case 'kimi_code':
      return 'Kimi Code 按量账户';
    case 'qcode':
      return 'QCode 按量账户';
    case 'minimax_code':
      return 'MiniMax 按量账户';
    default:
      return `${providerDisplayName(providerId)} 按量账户`;
  }
}

function formatBalanceAmount(
  micro: number | null | undefined,
  balCurrency: string,
  displayCurrency: CurrencyCode,
  usdToCnyRate: string
): string {
  const upper = balCurrency.toUpperCase();
  if (upper === 'CREDITS_UNLIMITED') {
    return 'Unlimited Credits (无限制)';
  }
  if (micro === null || micro === undefined || !Number.isFinite(micro)) {
    return '—';
  }
  if (upper === 'CREDITS') {
    return `${(micro / 1_000_000).toLocaleString('en-US', {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })} Credits`;
  }
  if (upper === 'CNY') {
    const cny = micro / 1_000_000;
    return `¥${cny.toLocaleString('en-US', {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })}`;
  }
  if (upper === 'USD') {
    return formatCost(micro, displayCurrency, usdToCnyRate);
  }
  return `${(micro / 1_000_000).toFixed(2)} ${balCurrency}`;
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
  if (lower.includes('30d') || lower.includes('month')) {
    return '30d 官方限额';
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
  if (lower.includes('30d') || lower.includes('month')) return 43200;
  return 999999;
}

function windowSourceBadge(source: string): { label: string; color: string } {
  if (source === 'official_api') {
    return { label: '官方 API 实时', color: 'var(--accent-emerald)' };
  }
  if (source === 'official_log_snapshot') {
    return { label: '官方日志快照', color: 'var(--accent-emerald)' };
  }
  return { label: '本地推算', color: 'var(--text-muted)' };
}

function autoSourceBadge(
  autoSource: string | null | undefined
): { label: string; color: string } | null {
  if (!autoSource) {
    return null;
  }
  if (autoSource === 'official_api') {
    return { label: '自动获取 · 官方 API', color: 'var(--accent-emerald)' };
  }
  if (autoSource === 'local_log') {
    return { label: '自动获取 · 本地日志', color: 'var(--accent-sky)' };
  }
  return { label: `自动获取 · ${autoSource}`, color: 'var(--accent-sky)' };
}

function computeLiveResetSeconds(
  win: { resets_at_ms: number | null; resets_in_seconds: number },
  fetchedAtMs: number,
  nowMs: number
): number {
  if (win.resets_at_ms !== null && Number.isFinite(win.resets_at_ms)) {
    return Math.max(0, Math.floor((win.resets_at_ms - nowMs) / 1000));
  }
  const elapsedSecs =
    fetchedAtMs > 0 ? Math.max(0, Math.floor((nowMs - fetchedAtMs) / 1000)) : 0;
  return Math.max(0, win.resets_in_seconds - elapsedSecs);
}

export function SubscriptionsPage() {
  const queryClient = useQueryClient();
  const { currency, usdToCnyRate, tzOffsetMinutes } = useWorkspaceStore();

  const [draft, setDraft] = useState<PlanFormDraft | null>(null);
  const [selectedTemplateId, setSelectedTemplateId] = useState<string>('custom');
  const [templateProviderFilter, setTemplateProviderFilter] = useState<string>('all');
  const [templateSearch, setTemplateSearch] = useState<string>('');
  const [formError, setFormError] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const [paygDraft, setPaygDraft] = useState<PaygFormDraft | null>(null);
  const [paygFormError, setPaygFormError] = useState<string | null>(null);
  const [confirmDeletePaygId, setConfirmDeletePaygId] = useState<string | null>(null);
  const [paygNotice, setPaygNotice] = useState<{ kind: 'success' | 'error'; text: string } | null>(
    null
  );

  const [nowMs, setNowMs] = useState<number>(() => Date.now());
  const prevCurrencyRef = useRef<{ currency: CurrencyCode; usdToCnyRate: string }>({
    currency,
    usdToCnyRate,
  });

  useEffect(() => {
    const timer = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    const prev = prevCurrencyRef.current;
    if (prev.currency === currency && prev.usdToCnyRate === usdToCnyRate) {
      return;
    }
    prevCurrencyRef.current = { currency, usdToCnyRate };
    setDraft((current) => {
      if (!current) {
        return current;
      }
      const feeMicro = currencyInputToMicroUsd(
        current.monthly_fee_input,
        prev.currency,
        prev.usdToCnyRate
      );
      return {
        ...current,
        monthly_fee_input:
          feeMicro !== null
            ? microUsdToCurrencyInput(feeMicro, currency, usdToCnyRate)
            : current.monthly_fee_input,
      };
    });
    setPaygDraft((current) => {
      if (!current) {
        return current;
      }
      const manualMicro = current.manual_balance_input.trim()
        ? currencyInputToMicroUsd(current.manual_balance_input, prev.currency, prev.usdToCnyRate)
        : null;
      const budgetMicro = current.monthly_budget_input.trim()
        ? currencyInputToMicroUsd(current.monthly_budget_input, prev.currency, prev.usdToCnyRate)
        : null;
      return {
        ...current,
        manual_balance_input:
          manualMicro !== null
            ? microUsdToCurrencyInput(manualMicro, currency, usdToCnyRate)
            : current.manual_balance_input,
        monthly_budget_input:
          budgetMicro !== null
            ? microUsdToCurrencyInput(budgetMicro, currency, usdToCnyRate)
            : current.monthly_budget_input,
      };
    });
  }, [currency, usdToCnyRate]);

  const {
    data: plans = [],
    isLoading,
    dataUpdatedAt: plansUpdatedAt,
  } = useQuery({
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
    refetchInterval: 10_000,
  });

  const { data: paygAccounts = [] } = useQuery({
    queryKey: ['payg_accounts', tzOffsetMinutes],
    queryFn: () => ipc.getPaygAccounts(tzOffsetMinutes),
    refetchInterval: 10_000,
  });

  const { data: providerConfigs = [] } = useQuery({
    queryKey: ['provider_configs'],
    queryFn: () => ipc.getProviderConfigs(),
    staleTime: 30_000,
  });

  const { data: prefs } = useQuery({
    queryKey: ['preferences'],
    queryFn: () => ipc.getPreferences(),
    staleTime: 30_000,
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
    await queryClient.invalidateQueries({ queryKey: ['subscription_templates'] });
    await queryClient.invalidateQueries({ queryKey: ['preferences'] });
    await queryClient.invalidateQueries({ queryKey: ['payg_accounts'] });
    await queryClient.invalidateQueries({ queryKey: ['provider_configs'] });
    await queryClient.invalidateQueries({ queryKey: ['provider_balances'] });
    await queryClient.invalidateQueries({ queryKey: ['monthly'] });
    await queryClient.invalidateQueries({ queryKey: ['overview'] });
    await queryClient.invalidateQueries({ queryKey: ['tray_summary'] });
    await queryClient.invalidateQueries({ queryKey: ['tray-summary'] });
    await queryClient.invalidateQueries({ queryKey: ['observed_rate_limits'] });
  };

  const syncCatalogMutation = useMutation({
    mutationFn: () => ipc.syncPricingAndRate(true),
    onSuccess: async (res) => {
      await invalidateSubscriptionQueries();
      if (
        res.warning &&
        !res.subscriptions_updated &&
        !res.pricing_updated &&
        !res.rate_updated
      ) {
        setPaygNotice({
          kind: 'error',
          text: `套餐目录同步受限（已保留本地基线）：${res.warning}`,
        });
      } else {
        setPaygNotice({
          kind: 'success',
          text: res.subscriptions_updated
            ? `套餐目录已更新（共 ${res.synced_plans_count} 款套餐${res.subscription_source ? ` · ${res.subscription_source}` : ''}）`
            : `套餐目录已是最新（${res.synced_plans_count || templates.length} 款模板）`,
        });
      }
    },
    onError: (err: unknown) => {
      setPaygNotice({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    },
  });

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

  const savePaygMutation = useMutation({
    mutationFn: async (payload: { account: PaygAccount; apiKey?: string }) => {
      const trimmedKey = payload.apiKey?.trim();
      if (trimmedKey && PAYG_OFFICIAL_BALANCE_PROVIDERS.has(payload.account.provider_id)) {
        await ipc.setProviderApiKey(payload.account.provider_id, trimmedKey);
      }
      await ipc.upsertPaygAccount(payload.account);
      return payload;
    },
    onSuccess: async (payload) => {
      setPaygDraft(null);
      setPaygFormError(null);
      setPaygNotice({
        kind: 'success',
        text: payload.apiKey?.trim()
          ? `已保存「${payload.account.account_name}」并安全写入系统凭据库，官方余额已同步`
          : `已保存按量计费账户「${payload.account.account_name}」`,
      });
      await invalidateSubscriptionQueries();
    },
    onError: (err: unknown) => {
      setPaygFormError(err instanceof Error ? err.message : String(err));
    },
  });

  const deletePaygMutation = useMutation({
    mutationFn: (accountId: string) => ipc.deletePaygAccount(accountId),
    onSuccess: async () => {
      setConfirmDeletePaygId(null);
      await invalidateSubscriptionQueries();
    },
  });

  const refreshPaygBalanceMutation = useMutation({
    mutationFn: (providerId: string) => ipc.refreshProviderBalance(providerId),
    onSuccess: async (outcome) => {
      await invalidateSubscriptionQueries();
      if (outcome.from_cache) {
        setPaygNotice({
          kind: 'success',
          text: `${providerDisplayName(outcome.snapshot.provider_id)} 使用近期缓存快照（冷却剩余 ${outcome.cooldown_remaining_secs}s）`,
        });
      } else {
        setPaygNotice({
          kind: 'success',
          text: `${providerDisplayName(outcome.snapshot.provider_id)} 官方余额已刷新`,
        });
      }
    },
    onError: (err: unknown) => {
      setPaygNotice({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    },
  });

  const openCreatePaygDrawer = (preselectedProviderId?: string) => {
    const providerId = preselectedProviderId ?? 'deepseek_harness';
    setPaygFormError(null);
    setPaygDraft({
      mode: 'create',
      account_id: `payg_${providerId}_${Date.now()}`,
      provider_id: providerId,
      account_name: defaultPaygAccountName(providerId),
      enabled: true,
      is_auto: false,
      api_key_input: '',
      manual_balance_input: '',
      initial_manual_balance_micro_usd: null,
      existing_anchor_ms: null,
      monthly_budget_input: '',
    });
  };

  const openEditPaygDrawer = (status: PaygAccountStatus) => {
    const acc = status.account;
    setPaygFormError(null);
    setPaygDraft({
      mode: 'edit',
      account_id: acc.account_id,
      provider_id: acc.provider_id,
      account_name: acc.account_name,
      enabled: acc.enabled,
      is_auto: acc.is_auto,
      api_key_input: '',
      manual_balance_input:
        acc.manual_balance_micro_usd !== null
          ? microUsdToCurrencyInput(acc.manual_balance_micro_usd, currency, usdToCnyRate)
          : '',
      initial_manual_balance_micro_usd: acc.manual_balance_micro_usd,
      existing_anchor_ms: acc.balance_anchor_ms,
      monthly_budget_input:
        acc.monthly_budget_micro_usd !== null
          ? microUsdToCurrencyInput(acc.monthly_budget_micro_usd, currency, usdToCnyRate)
          : '',
    });
  };

  const togglePaygEnabled = (status: PaygAccountStatus) => {
    savePaygMutation.mutate({
      account: {
        ...status.account,
        enabled: !status.account.enabled,
        updated_at_ms: Date.now(),
      },
    });
  };

  const handleSavePaygDraft = () => {
    if (!paygDraft) return;
    const trimmedName = paygDraft.account_name.trim();
    if (!trimmedName) {
      setPaygFormError('请输入按量计费账户名称（例如 DeepSeek 官方按量账户）');
      return;
    }

    let manualBalanceMicro: number | null = null;
    if (paygDraft.manual_balance_input.trim()) {
      const parsed = currencyInputToMicroUsd(
        paygDraft.manual_balance_input,
        currency,
        usdToCnyRate
      );
      if (parsed === null || parsed < 0) {
        setPaygFormError('预充起始余额格式无效，请输入大于或等于 0 的数字');
        return;
      }
      manualBalanceMicro = parsed;
    }

    let monthlyBudgetMicro: number | null = null;
    if (paygDraft.monthly_budget_input.trim()) {
      const parsed = currencyInputToMicroUsd(
        paygDraft.monthly_budget_input,
        currency,
        usdToCnyRate
      );
      if (parsed === null || parsed <= 0) {
        setPaygFormError('月度支出预警上限必须大于 0');
        return;
      }
      monthlyBudgetMicro = parsed;
    }

    const now = Date.now();
    const balanceAnchorMs =
      manualBalanceMicro === null
        ? null
        : paygDraft.mode === 'edit' &&
            paygDraft.initial_manual_balance_micro_usd === manualBalanceMicro &&
            paygDraft.existing_anchor_ms !== null
          ? paygDraft.existing_anchor_ms
          : now;

    setPaygFormError(null);
    savePaygMutation.mutate({
      account: {
        account_id: paygDraft.account_id,
        provider_id: paygDraft.provider_id,
        account_name: trimmedName,
        enabled: paygDraft.enabled,
        manual_balance_micro_usd: manualBalanceMicro,
        balance_anchor_ms: balanceAnchorMs,
        monthly_budget_micro_usd: monthlyBudgetMicro,
        is_auto: paygDraft.is_auto,
        updated_at_ms: now,
      },
      apiKey: paygDraft.api_key_input.trim() || undefined,
    });
  };

  const openCreateDrawer = (preselectedProviderId?: string) => {
    const providerId = preselectedProviderId ?? 'claude_code';
    setTemplateProviderFilter(preselectedProviderId ?? 'all');
    setTemplateSearch('');
    setSelectedTemplateId('custom');
    setFormError(null);
    setDraft({
      mode: 'create',
      plan_id: `sub_${providerId}_${Date.now()}`,
      provider_id: providerId,
      plan_name: '',
      enabled: true,
      monthly_fee_input: '0',
      windows: [],
    });
  };

  const openEditDrawer = (plan: SubscriptionPlan) => {
    setTemplateProviderFilter('all');
    setTemplateSearch('');
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
      windows: plan.windows,
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
      windows: tpl.windows,
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
      windows: draft.mode === 'create' ? [] : draft.windows,
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

    setFormError(null);
    savePlanMutation.mutate({
      plan_id: draft.plan_id,
      provider_id: draft.provider_id,
      plan_name: trimmedName,
      enabled: draft.enabled,
      monthly_fee_micro_usd: monthlyFeeMicroUsd,
      windows: draft.windows,
      is_user_configured: true,
      auto_source: null,
      updated_at_ms: Date.now(),
    });
  };

  const dynamicProviders = useMemo(() => {
    const map = new Map<string, string>();
    for (const p of PROVIDERS) {
      map.set(p.id, p.label);
    }
    for (const tpl of templates) {
      if (!map.has(tpl.provider_id)) {
        map.set(tpl.provider_id, providerDisplayName(tpl.provider_id));
      }
    }
    for (const sub of plans) {
      if (!map.has(sub.plan.provider_id)) {
        map.set(sub.plan.provider_id, providerDisplayName(sub.plan.provider_id));
      }
    }
    return Array.from(map.entries()).map(([id, label]) => ({ id, label }));
  }, [templates, plans]);

  const templateFilterOptions = useMemo(() => {
    const seen = new Set(templates.map((t) => t.provider_id));
    return dynamicProviders.filter((p) => seen.has(p.id));
  }, [dynamicProviders, templates]);

  const visibleTemplates = useMemo(() => {
    const query = templateSearch.trim().toLowerCase();
    return templates.filter((t) => {
      if (templateProviderFilter !== 'all' && t.provider_id !== templateProviderFilter) {
        return false;
      }
      if (!query) {
        return true;
      }
      const haystack = `${t.plan_name} ${t.provider_id} ${providerDisplayName(t.provider_id)} ${t.description}`.toLowerCase();
      return haystack.includes(query);
    });
  }, [templates, templateProviderFilter, templateSearch]);

  const draftFeeMicroPreview = draft
    ? currencyInputToMicroUsd(draft.monthly_fee_input, currency, usdToCnyRate)
    : null;

  const hasCredentialForProvider = (providerId: string): boolean =>
    providerConfigs.some((cfg) => cfg.provider_id === providerId && cfg.has_credential);

  return (
    <div className="space-y-7">
      {/* Header */}
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div className="flex flex-wrap items-center gap-2.5">
          <h1 className="text-[16px] font-semibold tracking-tight">订阅用量</h1>
          <span
            className="px-2 py-0.5 text-[11px] rounded-md tabular-nums"
            style={{
              backgroundColor: 'var(--bg-elevated)',
              border: '1px solid var(--border-hairline)',
              color: 'var(--text-secondary)',
            }}
          >
            套餐目录 · {templates.length} 款
          </span>
          <span className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            {prefs?.last_subscription_sync_at_ms
              ? `同步于 ${formatRelativeTime(prefs.last_subscription_sync_at_ms)}`
              : '内置基线目录'}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            disabled={syncCatalogMutation.isPending}
            onClick={() => syncCatalogMutation.mutate()}
            className="btn-interactive bg-[var(--bg-elevated)] inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[12px] cursor-pointer disabled:opacity-50"
            style={{
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-primary)',
            }}
          >
            <RefreshCw
              className={`w-3.5 h-3.5 ${syncCatalogMutation.isPending ? 'animate-spin' : ''}`}
            />
            <span>{syncCatalogMutation.isPending ? '同步中…' : '同步套餐目录'}</span>
          </button>

          <button
            type="button"
            onClick={() => openCreatePaygDrawer()}
            className="btn-tonal inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium cursor-pointer"
          >
            <Wallet className="w-3.5 h-3.5" />
            <span>添加按量计费</span>
          </button>

          <button
            type="button"
            onClick={() => openCreateDrawer()}
            className="btn-primary inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium cursor-pointer"
            style={{
              backgroundColor: 'var(--accent-primary)',
              color: '#ffffff',
            }}
          >
            <Plus className="w-3.5 h-3.5" />
            <span>添加我的订阅</span>
          </button>
        </div>
      </section>

      {paygNotice && (
        <div
          className="surface-tonal-band px-3.5 py-2 text-[12px] flex items-center justify-between"
          style={{
            color: paygNotice.kind === 'error' ? 'var(--accent-rose)' : 'var(--accent-emerald)',
          }}
        >
          <span>{paygNotice.text}</span>
          <button
            type="button"
            onClick={() => setPaygNotice(null)}
            className="btn-interactive px-2 py-0.5 rounded-lg text-[11px] cursor-pointer"
            style={{ color: 'var(--text-muted)' }}
          >
            关闭
          </button>
        </div>
      )}

      {isLoading ? (
        <div className="py-12 text-center" style={{ color: 'var(--text-muted)' }}>
          加载订阅数据与实测用量中…
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
              <div className="surface-tonal-band py-7 px-5 flex flex-col md:flex-row md:items-center md:justify-between gap-5">
                <div className="space-y-1.5 max-w-2xl">
                  <div className="flex items-center gap-2 text-[13.5px] font-semibold">
                    <Info className="w-4 h-4 shrink-0" style={{ color: 'var(--accent-primary)' }} />
                    <span>尚未添加个人订阅配置</span>
                  </div>
                  <p className="text-[12px] leading-relaxed" style={{ color: 'var(--text-secondary)' }}>
                    TokenTracker 仅通过本地只读日志与官方公开 API 核算用量；在配置支持官方配额接口的 Key（如 Kimi Code `sk-kimi-...`、MiniMax、QCode）或产生含官方限额快照的本地日志（如 Codex）后，系统会自动将其加入此处。您也可以随时从云端共享目录添加固定月费订阅（如 Claude Pro / Max、ChatGPT Plus / Pro、Google AI Pro / Ultra、Cursor、GitHub Copilot）追踪真实等效消耗与月费回本率，或在下方追踪 DeepSeek / Codex Credits 等按量计费账户。
                  </p>
                </div>

                <button
                  type="button"
                  onClick={() => openCreateDrawer()}
                  className="btn-primary inline-flex items-center gap-1.5 px-3.5 py-2 rounded-lg text-[12px] font-medium shrink-0 cursor-pointer"
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
                  const hasOfficialWindows = windows.length > 0;
                  const confirmingDelete = confirmDeleteId === plan.plan_id;
                  const autoBadge = autoSourceBadge(plan.auto_source);
                  const hasFee = plan.monthly_fee_micro_usd > 0;
                  const roiRatio = hasFee
                    ? status.month_cost_micro_usd / plan.monthly_fee_micro_usd
                    : 0;
                  const roiPct = roiRatio * 100;

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
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-[14px] font-semibold">{plan.plan_name}</span>
                          {autoBadge && (
                            <span
                              className="px-1.5 py-0.5 text-[10px] rounded-md shrink-0"
                              style={{
                                backgroundColor: 'var(--bg-elevated)',
                                border: '1px solid var(--border-hairline)',
                                color: autoBadge.color,
                              }}
                            >
                              {autoBadge.label}
                            </span>
                          )}
                          {!hasOfficialWindows && (
                            <span
                              className="px-1.5 py-0.5 text-[10px] rounded-md shrink-0"
                              style={{
                                backgroundColor: 'var(--bg-elevated)',
                                border: '1px solid var(--border-hairline)',
                                color: 'var(--text-muted)',
                              }}
                            >
                              仅统计真实消耗 · 官方未公开配额接口
                            </span>
                          )}
                        </div>
                        <div
                          className="flex flex-wrap items-center gap-2 text-[11.5px] tabular-nums"
                          style={{ color: 'var(--text-secondary)' }}
                        >
                          <span>
                            {resolveSubscriptionTierLabel(
                              plan,
                              currency,
                              usdToCnyRate,
                              hasOfficialWindows
                            )}
                          </span>
                          <button
                            type="button"
                            onClick={() => toggleEnabled(status)}
                            disabled={savePlanMutation.isPending}
                            className="btn-interactive bg-[var(--bg-elevated)] px-2 py-0.5 rounded-lg text-[11px] cursor-pointer disabled:opacity-50"
                            style={{
                              border: '1px solid var(--border-hairline)',
                              color: plan.enabled ? 'var(--accent-emerald)' : 'var(--text-muted)',
                            }}
                          >
                            {plan.enabled ? '已启用' : '已停用'}
                          </button>
                        </div>
                      </div>

                      {/* Center Column: Official Windows OR Factual Spend & ROI */}
                      <div className="col-span-12 lg:col-span-6 space-y-3">
                        {hasOfficialWindows ? (
                          <>
                            {windows.map((w) => {
                              const usedStr =
                                w.unit === 'micro_usd'
                                  ? formatCost(w.used_value, currency, usdToCnyRate)
                                  : formatTokens(w.used_value);
                              const limitStr =
                                w.unit === 'micro_usd'
                                  ? formatCost(w.limit_value, currency, usdToCnyRate)
                                  : formatTokens(w.limit_value);
                              const srcBadge = windowSourceBadge(w.source);
                              return (
                                <div key={w.window_id} className="space-y-1">
                                  <div className="flex items-center justify-between gap-2 text-[12px] tabular-nums">
                                    <div className="flex items-center gap-2 min-w-0">
                                      <span className="font-medium truncate">{w.label}</span>
                                      <span
                                        className="px-1.5 py-0.5 text-[10px] rounded-md shrink-0"
                                        style={{
                                          backgroundColor: 'var(--bg-elevated)',
                                          border: '1px solid var(--border-hairline)',
                                          color: srcBadge.color,
                                        }}
                                      >
                                        {srcBadge.label}
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
                            <div
                              className="pt-1 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] tabular-nums"
                              style={{ color: 'var(--text-muted)' }}
                            >
                              <span>
                                今日等效{' '}
                                {formatCost(status.today_cost_micro_usd, currency, usdToCnyRate)}
                              </span>
                              <span>
                                近 7d{' '}
                                {formatCost(status.week_cost_micro_usd, currency, usdToCnyRate)}
                              </span>
                              <span>
                                本月等效价值{' '}
                                {formatCost(status.month_cost_micro_usd, currency, usdToCnyRate)}（
                                {formatTokens(status.month_tokens)} Tokens）
                              </span>
                              {hasFee && (
                                <span>
                                  月费回本率{' '}
                                  <strong
                                    style={{
                                      color:
                                        roiRatio >= 1
                                          ? 'var(--accent-emerald)'
                                          : 'var(--text-primary)',
                                    }}
                                  >
                                    {roiPct.toFixed(1)}% ({roiRatio.toFixed(2)}x)
                                  </strong>
                                </span>
                              )}
                            </div>
                          </>
                        ) : (
                          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 tabular-nums">
                            <div>
                              <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                                今日等效消耗
                              </div>
                              <div className="mt-0.5 text-[14px] font-semibold">
                                {formatCost(status.today_cost_micro_usd, currency, usdToCnyRate)}
                              </div>
                            </div>

                            <div
                              className="sm:pl-3"
                              style={{ borderLeft: '1px solid var(--border-hairline)' }}
                            >
                              <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                                近 7d 等效消耗
                              </div>
                              <div className="mt-0.5 text-[14px] font-semibold">
                                {formatCost(status.week_cost_micro_usd, currency, usdToCnyRate)}
                              </div>
                            </div>

                            <div
                              className="sm:pl-3"
                              style={{ borderLeft: '1px solid var(--border-hairline)' }}
                            >
                              <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                                本月等效价值
                              </div>
                              <div className="mt-0.5 text-[14px] font-semibold">
                                {formatCost(status.month_cost_micro_usd, currency, usdToCnyRate)}
                              </div>
                              <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                                {formatTokens(status.month_tokens)} Tokens
                              </div>
                            </div>

                            <div
                              className="sm:pl-3"
                              style={{ borderLeft: '1px solid var(--border-hairline)' }}
                            >
                              <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                                月费回本率
                              </div>
                              <div
                                className="mt-0.5 text-[14px] font-semibold"
                                style={{
                                  color: hasFee
                                    ? roiRatio >= 1
                                      ? 'var(--accent-emerald)'
                                      : 'var(--text-primary)'
                                    : 'var(--text-muted)',
                                }}
                              >
                                {hasFee ? `${roiPct.toFixed(1)}%` : '—'}
                              </div>
                              <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                                {hasFee ? `${roiRatio.toFixed(2)}x 月费` : '未设月费'}
                              </div>
                            </div>
                          </div>
                        )}
                      </div>

                      {/* Right Column: Burn Rate & Reset (Official only) + Edit & Delete */}
                      <div className="col-span-12 lg:col-span-3 flex items-center justify-between gap-3">
                        {hasOfficialWindows ? (
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
                                      {formatCountdown(
                                        computeLiveResetSeconds(w, plansUpdatedAt, nowMs)
                                      )}
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
                        ) : (
                          <div className="space-y-0.5 text-[11.5px]">
                            <div style={{ color: 'var(--text-secondary)' }}>按真实用量追踪</div>
                            <div style={{ color: 'var(--text-muted)' }}>不显示本地推算限额</div>
                          </div>
                        )}

                        <div className="flex items-center gap-1.5 shrink-0">
                          {confirmingDelete ? (
                            <div className="flex items-center gap-1">
                              <button
                                type="button"
                                disabled={deletePlanMutation.isPending}
                                onClick={() => deletePlanMutation.mutate(plan.plan_id)}
                                className="btn-danger inline-flex items-center gap-1 px-2 py-1 rounded-lg text-[11px] font-medium cursor-pointer disabled:opacity-50"
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
                                className="btn-interactive bg-[var(--bg-elevated)] px-2 py-1 rounded-lg text-[11px] cursor-pointer"
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
                                className="btn-interactive bg-[var(--bg-elevated)] p-1.5 rounded-lg cursor-pointer"
                                style={{
                                  border: '1px solid var(--border-subtle)',
                                  color: 'var(--text-secondary)',
                                }}
                                title="编辑订阅配置"
                                aria-label={`编辑 ${plan.plan_name}`}
                              >
                                <Pencil className="w-3.5 h-3.5" />
                              </button>
                              <button
                                type="button"
                                onClick={() => setConfirmDeleteId(plan.plan_id)}
                                className="btn-danger bg-[var(--bg-elevated)] p-1.5 rounded-lg cursor-pointer"
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
                    官方日志可观测限额快照
                  </h2>
                </div>
              </div>

              <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
                {unboundObservedGroups.map((group) => (
                  <div
                    key={group.provider_id}
                    className="row-interactive py-5 px-2 grid grid-cols-12 gap-5 items-center"
                    style={{ borderBottom: '1px solid var(--border-hairline)' }}
                  >
                    {/* Left: Provider Identity */}
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
                          className="px-1.5 py-0.5 rounded-md text-[10.5px]"
                          style={{
                            backgroundColor: 'var(--bg-elevated)',
                            border: '1px solid var(--border-hairline)',
                            color: 'var(--accent-emerald)',
                          }}
                        >
                          {group.windows[0]?.plan_hint
                            ? `识别档位 · ${group.windows[0].plan_hint}`
                            : '官方实时限额'}
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
                              ? Math.max(0, Math.floor((obs.resets_at_ms - nowMs) / 1000))
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
                        className="btn-interactive bg-[var(--bg-elevated)] inline-flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11.5px] font-medium shrink-0 cursor-pointer"
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

          {/* Section 2: Pay-As-You-Go Accounts, Official Balances & Credits */}
          <section className="space-y-2.5">
            <div className="flex items-center justify-between">
              <h2
                className="text-[11px] font-semibold uppercase tracking-wider"
                style={{ color: 'var(--text-muted)' }}
              >
                按量计费与账户余额 / Credits
              </h2>
              <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                {paygAccounts.length} 项账户
              </span>
            </div>

            {paygAccounts.length === 0 ? (
              <div className="surface-tonal-band py-5 px-5 flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
                <div className="space-y-1 max-w-2xl">
                  <div className="flex items-center gap-2 text-[13px] font-semibold">
                    <Wallet
                      className="w-4 h-4 shrink-0"
                      style={{ color: 'var(--accent-tonal-text)' }}
                    />
                    <span>支持按量计费、API 余额与 Codex Credits 追踪</span>
                  </div>
                  <p className="text-[11.5px] leading-relaxed" style={{ color: 'var(--text-secondary)' }}>
                    配置 DeepSeek / Moonshot / MiniMax / QCode 官方 API Key 可直接同步账户总余额（含赠金与充值拆解）；Codex 本地日志中的 Credits 将自动识别并纳入统计。也可添加本地按量预算上限与预充余额倒扣追踪。
                  </p>
                </div>

                <button
                  type="button"
                  onClick={() => openCreatePaygDrawer()}
                  className="btn-tonal inline-flex items-center gap-1.5 px-3 py-1.5 text-[12px] font-medium shrink-0 cursor-pointer"
                >
                  <Plus className="w-3.5 h-3.5" />
                  <span>添加按量计费</span>
                </button>
              </div>
            ) : (
              <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
                {paygAccounts.map((status) => {
                  const acc = status.account;
                  const offBal = status.official_balance;
                  const confirmingDelete = confirmDeletePaygId === acc.account_id;
                  const supportsBalanceRefresh =
                    PAYG_OFFICIAL_BALANCE_PROVIDERS.has(acc.provider_id) &&
                    hasCredentialForProvider(acc.provider_id);
                  const budgetMicro = acc.monthly_budget_micro_usd;
                  const budgetPct =
                    status.budget_used_percent !== null && Number.isFinite(status.budget_used_percent)
                      ? Math.max(0, status.budget_used_percent)
                      : null;

                  return (
                    <div
                      key={acc.account_id}
                      className="row-interactive py-4 px-2.5 grid grid-cols-12 gap-5 items-center"
                      style={{
                        borderBottom: '1px solid var(--border-hairline)',
                        opacity: acc.enabled ? 1 : 0.58,
                      }}
                    >
                      {/* Left: Provider & Account Identity */}
                      <div className="col-span-12 lg:col-span-3 space-y-1.5">
                        <div className="flex items-center gap-2">
                          <span
                            className="inline-block w-2 h-2 rounded-full shrink-0"
                            style={{ backgroundColor: providerAccentColor(acc.provider_id) }}
                          />
                          <span
                            className="text-[11px] uppercase tracking-wider"
                            style={{ color: 'var(--text-muted)' }}
                          >
                            {providerDisplayName(acc.provider_id)}
                          </span>
                        </div>
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-[14px] font-semibold">{acc.account_name}</span>
                          <span
                            className="px-1.5 py-0.5 text-[10px] rounded-md shrink-0"
                            style={{
                              backgroundColor: 'var(--bg-elevated)',
                              border: '1px solid var(--border-hairline)',
                              color: offBal
                                ? 'var(--accent-emerald)'
                                : acc.is_auto
                                  ? 'var(--accent-sky)'
                                  : 'var(--text-secondary)',
                            }}
                          >
                            {offBal
                              ? acc.provider_id === 'codex'
                                ? '本地日志 Credits'
                                : '官方 API 余额'
                              : '按量预算追踪'}
                          </span>
                        </div>
                        <div className="flex items-center gap-2">
                          <button
                            type="button"
                            onClick={() => togglePaygEnabled(status)}
                            disabled={savePaygMutation.isPending}
                            className="btn-interactive bg-[var(--bg-elevated)] px-2 py-0.5 text-[11px] cursor-pointer disabled:opacity-50"
                            style={{
                              border: '1px solid var(--border-hairline)',
                              color: acc.enabled ? 'var(--accent-emerald)' : 'var(--text-muted)',
                            }}
                          >
                            {acc.enabled ? '已启用' : '已停用'}
                          </button>
                        </div>
                      </div>

                      {/* Center: Balance / Credits & Spend */}
                      <div className="col-span-12 lg:col-span-6 space-y-2.5 tabular-nums">
                        {offBal && (
                          <div className="flex flex-wrap items-baseline justify-between gap-2">
                            <div className="flex flex-wrap items-baseline gap-2.5">
                              <span className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                                {offBal.currency.startsWith('CREDITS') ? '可用 Credits' : '账户总余额'}
                              </span>
                              <span className="text-[16px] font-semibold">
                                {formatBalanceAmount(
                                  offBal.total_balance_micro,
                                  offBal.currency,
                                  currency,
                                  usdToCnyRate
                                )}
                              </span>
                            </div>
                            {(offBal.granted_balance_micro !== null ||
                              offBal.topped_up_balance_micro !== null) && (
                              <div
                                className="flex flex-wrap items-center gap-2 text-[11px]"
                                style={{ color: 'var(--text-secondary)' }}
                              >
                                {offBal.granted_balance_micro !== null && (
                                  <span>
                                    赠金{' '}
                                    {formatBalanceAmount(
                                      offBal.granted_balance_micro,
                                      offBal.currency,
                                      currency,
                                      usdToCnyRate
                                    )}
                                  </span>
                                )}
                                {offBal.topped_up_balance_micro !== null && (
                                  <span>
                                    · 充值{' '}
                                    {formatBalanceAmount(
                                      offBal.topped_up_balance_micro,
                                      offBal.currency,
                                      currency,
                                      usdToCnyRate
                                    )}
                                  </span>
                                )}
                              </div>
                            )}
                          </div>
                        )}

                        {status.remaining_manual_balance_micro_usd !== null && (
                          <div className="flex items-baseline justify-between gap-2 text-[12px]">
                            <span style={{ color: 'var(--text-secondary)' }}>
                              预充剩余推算（基于本地用量扣减）
                            </span>
                            <span
                              className="font-semibold"
                              style={{
                                color:
                                  status.remaining_manual_balance_micro_usd <= 0
                                    ? 'var(--accent-rose)'
                                    : 'var(--text-primary)',
                              }}
                            >
                              {formatCost(
                                status.remaining_manual_balance_micro_usd,
                                currency,
                                usdToCnyRate
                              )}
                            </span>
                          </div>
                        )}

                        <div
                          className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11.5px]"
                          style={{ color: 'var(--text-secondary)' }}
                        >
                          <span>
                            今日按量{' '}
                            <strong style={{ color: 'var(--text-primary)' }}>
                              {formatCost(status.today_cost_micro_usd, currency, usdToCnyRate)}
                            </strong>
                          </span>
                          <span>
                            本月累计{' '}
                            <strong style={{ color: 'var(--text-primary)' }}>
                              {formatCost(status.month_cost_micro_usd, currency, usdToCnyRate)}
                            </strong>
                          </span>
                        </div>

                        {budgetMicro !== null && budgetMicro > 0 && budgetPct !== null && (
                          <div className="space-y-1">
                            <div className="flex items-center justify-between text-[11.5px]">
                              <span style={{ color: 'var(--text-muted)' }}>本月按量预算</span>
                              <span>
                                {formatCost(status.month_cost_micro_usd, currency, usdToCnyRate)} /{' '}
                                {formatCost(budgetMicro, currency, usdToCnyRate)} ({budgetPct.toFixed(1)}%)
                              </span>
                            </div>
                            <InlineProgressBar percent={budgetPct} heightPx={5} />
                          </div>
                        )}
                      </div>

                      {/* Right: Timestamp, Refresh Balance, Edit & Delete */}
                      <div className="col-span-12 lg:col-span-3 flex items-center justify-between gap-3">
                        <div className="space-y-1.5 text-[11.5px] tabular-nums">
                          <div style={{ color: 'var(--text-muted)' }}>
                            {offBal
                              ? `快照于 ${formatRelativeTime(offBal.fetched_at_ms)}`
                              : `更新于 ${formatRelativeTime(acc.updated_at_ms)}`}
                          </div>
                          {supportsBalanceRefresh && (
                            <button
                              type="button"
                              disabled={refreshPaygBalanceMutation.isPending}
                              onClick={() => refreshPaygBalanceMutation.mutate(acc.provider_id)}
                              className="btn-tonal inline-flex items-center gap-1 px-2 py-0.5 text-[11px] cursor-pointer disabled:opacity-50"
                            >
                              <RefreshCw
                                className={`w-3 h-3 ${
                                  refreshPaygBalanceMutation.isPending ? 'animate-spin' : ''
                                }`}
                              />
                              <span>刷新余额</span>
                            </button>
                          )}
                        </div>

                        <div className="flex items-center gap-1.5 shrink-0">
                          {confirmingDelete ? (
                            <div className="flex items-center gap-1">
                              <button
                                type="button"
                                disabled={deletePaygMutation.isPending}
                                onClick={() => deletePaygMutation.mutate(acc.account_id)}
                                className="btn-danger inline-flex items-center gap-1 px-2 py-1 rounded-lg text-[11px] font-medium cursor-pointer disabled:opacity-50"
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
                                onClick={() => setConfirmDeletePaygId(null)}
                                className="btn-interactive bg-[var(--bg-elevated)] px-2 py-1 rounded-lg text-[11px] cursor-pointer"
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
                                onClick={() => openEditPaygDrawer(status)}
                                className="btn-interactive bg-[var(--bg-elevated)] p-1.5 rounded-lg cursor-pointer"
                                style={{
                                  border: '1px solid var(--border-subtle)',
                                  color: 'var(--text-secondary)',
                                }}
                                title="编辑按量计费账户"
                                aria-label={`编辑 ${acc.account_name}`}
                              >
                                <Pencil className="w-3.5 h-3.5" />
                              </button>
                              <button
                                type="button"
                                onClick={() => setConfirmDeletePaygId(acc.account_id)}
                                className="btn-danger bg-[var(--bg-elevated)] p-1.5 rounded-lg cursor-pointer"
                                style={{
                                  border: '1px solid var(--border-subtle)',
                                  color: 'var(--accent-rose)',
                                }}
                                title="移除该按量计费账户"
                                aria-label={`删除 ${acc.account_name}`}
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

          {/* Section 4: Cross-Device Synced Remote Subscription Snapshots (Read-only) */}
          {remoteDeviceGroups.length > 0 && (
            <section className="space-y-2">
              <div>
                <h2
                  className="text-[11px] font-semibold uppercase tracking-wider"
                  style={{ color: 'var(--text-muted)' }}
                >
                  跨设备同步配额快照（只读）
                </h2>
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
                          className="px-1.5 py-0.5 rounded-md text-[10.5px]"
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
                            ? Math.max(0, Math.floor((snap.resets_at_ms - nowMs) / 1000))
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
      {draft &&
        createPortal(
          <div
            className="drawer-backdrop-enter fixed inset-0 z-50 flex justify-end backdrop-blur-sm"
          style={{ backgroundColor: 'rgba(6, 8, 14, 0.42)' }}
          onClick={(e) => {
            if (e.target === e.currentTarget) {
              setDraft(null);
            }
          }}
        >
          <div
            className="drawer-panel-enter liquid-drawer w-full max-w-[480px] h-full p-5 overflow-y-auto space-y-5"
            style={{
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
                  {draft.mode === 'create' ? '添加我的订阅配置' : '编辑订阅配置'}
                </h2>
              </div>
              <button
                type="button"
                onClick={() => setDraft(null)}
                className="btn-interactive p-1.5 rounded-lg cursor-pointer"
                style={{ color: 'var(--text-secondary)' }}
                aria-label="关闭抽屉"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* Template Quick-Fill Section (AI Subscription Catalog) */}
            <div
              className="pb-4 space-y-2.5"
              style={{ borderBottom: '1px solid var(--border-subtle)' }}
            >
              <div className="flex items-center justify-between gap-2">
                <span className="text-[12px] font-medium">套餐目录快速预填（可选）</span>
                <span className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                  共 {templates.length} 款
                </span>
              </div>

              <input
                type="search"
                aria-label="搜索套餐模板"
                placeholder="搜索套餐名称或提供商（如 Ultra、Max、Cursor、Pro）"
                value={templateSearch}
                onChange={(e) => setTemplateSearch(e.target.value)}
                className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />

              <div className="flex flex-wrap items-center gap-1">
                {[{ id: 'all', label: '全部' }, ...templateFilterOptions].map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    onClick={() => setTemplateProviderFilter(p.id)}
                    className="seg-pill px-2 py-0.5 rounded-lg text-[11px] cursor-pointer"
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

              <div className="grid grid-cols-2 gap-2 max-h-60 overflow-y-auto pr-0.5">
                {visibleTemplates.map((tpl) => {
                  const active = selectedTemplateId === tpl.template_id;
                  return (
                    <button
                      key={tpl.template_id}
                      type="button"
                      onClick={() => applyTemplateToDraft(tpl)}
                      className={`btn-interactive text-left px-2.5 py-2 rounded-lg space-y-0.5 cursor-pointer ${
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
                        {tpl.monthly_fee_micro_usd > 0
                          ? `${formatCost(tpl.monthly_fee_micro_usd, currency, usdToCnyRate)}/月`
                          : '免费 / 按量'}
                      </div>
                      {tpl.live_windows && tpl.live_windows.length > 0 ? (
                        <div
                          className="text-[10px] tabular-nums truncate"
                          style={{ color: 'var(--accent-emerald)' }}
                        >
                          {tpl.live_windows
                            .map((w) => `${w.window_id} ${w.used_percent.toFixed(1)}%`)
                            .join(' · ')}
                        </div>
                      ) : (
                        tpl.description && (
                          <div
                            className="text-[10px] truncate"
                            style={{ color: 'var(--text-muted)' }}
                            title={tpl.description}
                          >
                            {tpl.description}
                          </div>
                        )
                      )}
                    </button>
                  );
                })}

                <button
                  type="button"
                  onClick={applyCustomTemplateToDraft}
                  className={`btn-interactive text-left px-2.5 py-2 rounded-lg space-y-0.5 cursor-pointer ${
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
                    手填名称与月费（可为 0）
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
                    所属模型提供商
                  </label>
                  <select
                    aria-label="所属模型提供商"
                    value={draft.provider_id}
                    onChange={(e) => setDraft({ ...draft, provider_id: e.target.value })}
                    className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none cursor-pointer"
                    style={{
                      backgroundColor: 'var(--bg-input)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  >
                    {dynamicProviders.map((p) => (
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
                    className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none"
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
                  htmlFor="sub-monthly-fee"
                  className="block text-[11.5px] mb-1"
                  style={{ color: 'var(--text-secondary)' }}
                >
                  订阅月费 ({currency}，填 0 则不计算月度回本率)
                </label>
                <div className="flex items-center gap-2">
                  <input
                    id="sub-monthly-fee"
                    type="text"
                    inputMode="decimal"
                    placeholder="0"
                    value={draft.monthly_fee_input}
                    onChange={(e) => setDraft({ ...draft, monthly_fee_input: e.target.value })}
                    className="w-36 px-2.5 py-1.5 text-[12px] rounded-lg tabular-nums outline-none"
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

              <div
                className="px-3 py-2.5 rounded-lg text-[11.5px] leading-relaxed"
                style={{
                  backgroundColor: 'var(--bg-elevated)',
                  border: '1px solid var(--border-hairline)',
                  color: 'var(--text-secondary)',
                }}
              >
                配额进度仅在数据源产生官方实测窗口（如 Codex 日志限额快照、Kimi / MiniMax / QCode 官方 API）时自动匹配显示；对于未开放配额接口的订阅，界面仅基于真实 Token 用量统计今日 / 近 7d / 本月等效消耗与月费回本率，不展示估算百分比。
              </div>

              {formError && (
                <div
                  className="px-3 py-2 rounded-lg text-[11.5px]"
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
                  className="btn-primary px-3.5 py-1.5 text-[12px] font-medium rounded-lg cursor-pointer disabled:opacity-50"
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
                  className="btn-interactive bg-[var(--bg-elevated)] px-3 py-1.5 text-[12px] rounded-lg cursor-pointer"
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
        </div>,
          document.body
        )}

      {/* Create / Edit Pay-As-You-Go Drawer */}
      {paygDraft &&
        createPortal(
          <div
            className="drawer-backdrop-enter fixed inset-0 z-50 flex justify-end backdrop-blur-sm"
          style={{ backgroundColor: 'rgba(6, 8, 14, 0.42)' }}
          onClick={(e) => {
            if (e.target === e.currentTarget) {
              setPaygDraft(null);
            }
          }}
        >
          <div
            className="drawer-panel-enter liquid-drawer w-full max-w-[460px] h-full p-5 overflow-y-auto space-y-5"
            style={{
              borderLeft: '1px solid var(--border-strong)',
              color: 'var(--text-primary)',
            }}
          >
            <div
              className="flex items-center justify-between pb-3"
              style={{ borderBottom: '1px solid var(--border-subtle)' }}
            >
              <h2 className="text-[14.5px] font-semibold">
                {paygDraft.mode === 'create' ? '添加按量计费账户' : '编辑按量计费账户'}
              </h2>
              <button
                type="button"
                onClick={() => setPaygDraft(null)}
                className="btn-interactive p-1.5 cursor-pointer"
                style={{ color: 'var(--text-secondary)' }}
                aria-label="关闭按量计费抽屉"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <div className="space-y-4">
              <div>
                <label
                  htmlFor="payg-provider"
                  className="block text-[11.5px] mb-1"
                  style={{ color: 'var(--text-secondary)' }}
                >
                  模型提供商
                </label>
                <select
                  id="payg-provider"
                  aria-label="按量计费提供商"
                  value={paygDraft.provider_id}
                  onChange={(e) => {
                    const nextPid = e.target.value;
                    setPaygDraft({
                      ...paygDraft,
                      provider_id: nextPid,
                      account_name:
                        paygDraft.mode === 'create'
                          ? defaultPaygAccountName(nextPid)
                          : paygDraft.account_name,
                    });
                  }}
                  className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none cursor-pointer"
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
                  htmlFor="payg-account-name"
                  className="block text-[11.5px] mb-1"
                  style={{ color: 'var(--text-secondary)' }}
                >
                  账户 / 账本名称
                </label>
                <input
                  id="payg-account-name"
                  type="text"
                  placeholder="例如 DeepSeek 官方按量账户 或 OpenRouter 余额"
                  value={paygDraft.account_name}
                  onChange={(e) => setPaygDraft({ ...paygDraft, account_name: e.target.value })}
                  className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none"
                  style={{
                    backgroundColor: 'var(--bg-input)',
                    border: '1px solid var(--border-subtle)',
                    color: 'var(--text-primary)',
                  }}
                />
              </div>

              {PAYG_OFFICIAL_BALANCE_PROVIDERS.has(paygDraft.provider_id) ? (
                <div className="surface-tonal-band p-3 space-y-2">
                  <label
                    htmlFor="payg-api-key"
                    className="block text-[11.5px] font-medium"
                    style={{ color: 'var(--text-primary)' }}
                  >
                    官方 API Key（可选 · 加密存入系统凭据库并自动同步官方总余额）
                  </label>
                  <input
                    id="payg-api-key"
                    type="password"
                    autoComplete="off"
                    placeholder={
                      hasCredentialForProvider(paygDraft.provider_id)
                        ? '系统凭据库已保存 Key，留空保持不变或输入新 Key 替换'
                        : 'sk-...'
                    }
                    value={paygDraft.api_key_input}
                    onChange={(e) => setPaygDraft({ ...paygDraft, api_key_input: e.target.value })}
                    className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none font-mono"
                    style={{
                      backgroundColor: 'var(--bg-input)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  />
                  <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                    仅调用厂商官方公开余额接口（如 DeepSeek `/user/balance`）；密钥不落盘明文、不同步云端。
                  </div>
                </div>
              ) : (
                paygDraft.provider_id === 'codex' && (
                  <div
                    className="surface-tonal-band p-3 text-[11.5px]"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    Codex 的官方 Credits 会自动从本地 `~/.codex/sessions/` 只读日志提取并刷新。
                  </div>
                )
              )}

              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div>
                  <label
                    htmlFor="payg-manual-balance"
                    className="block text-[11.5px] mb-1"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    预充起始余额 ({currency}，可选)
                  </label>
                  <input
                    id="payg-manual-balance"
                    type="text"
                    inputMode="decimal"
                    placeholder="留空则不追踪预充"
                    value={paygDraft.manual_balance_input}
                    onChange={(e) =>
                      setPaygDraft({ ...paygDraft, manual_balance_input: e.target.value })
                    }
                    className="w-full px-2.5 py-1.5 text-[12px] rounded-lg tabular-nums outline-none"
                    style={{
                      backgroundColor: 'var(--bg-input)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  />
                </div>

                <div>
                  <label
                    htmlFor="payg-monthly-budget"
                    className="block text-[11.5px] mb-1"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    月度支出上限 ({currency}，可选)
                  </label>
                  <input
                    id="payg-monthly-budget"
                    type="text"
                    inputMode="decimal"
                    placeholder="例如 50"
                    value={paygDraft.monthly_budget_input}
                    onChange={(e) =>
                      setPaygDraft({ ...paygDraft, monthly_budget_input: e.target.value })
                    }
                    className="w-full px-2.5 py-1.5 text-[12px] rounded-lg tabular-nums outline-none"
                    style={{
                      backgroundColor: 'var(--bg-input)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  />
                </div>
              </div>

              {paygFormError && (
                <div
                  className="px-3 py-2 rounded-lg text-[11.5px]"
                  style={{
                    backgroundColor: 'var(--bg-elevated)',
                    border: '1px solid var(--accent-rose)',
                    color: 'var(--accent-rose)',
                  }}
                >
                  {paygFormError}
                </div>
              )}

              <div
                className="pt-3 flex items-center gap-2"
                style={{ borderTop: '1px solid var(--border-subtle)' }}
              >
                <button
                  type="button"
                  disabled={savePaygMutation.isPending}
                  onClick={handleSavePaygDraft}
                  className="btn-primary px-3.5 py-1.5 rounded-lg text-[12px] font-medium cursor-pointer disabled:opacity-50"
                  style={{
                    backgroundColor: 'var(--accent-primary)',
                    color: '#ffffff',
                  }}
                >
                  {savePaygMutation.isPending ? '保存中…' : '保存按量计费账户'}
                </button>
                <button
                  type="button"
                  onClick={() => setPaygDraft(null)}
                  className="btn-interactive bg-[var(--bg-elevated)] px-3 py-1.5 rounded-lg text-[12px] cursor-pointer"
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
        </div>,
          document.body
        )}
    </div>
  );
}
