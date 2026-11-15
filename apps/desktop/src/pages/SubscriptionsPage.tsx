import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Pencil, X } from 'lucide-react';
import { InlineProgressBar } from '../components/InlineProgressBar';
import {
  formatCost,
  formatCountdown,
  formatTokens,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type { SubscriptionPlan, SubscriptionPlanStatus } from '../types/ipc';

export function SubscriptionsPage() {
  const queryClient = useQueryClient();
  const { currency, usdToCnyRate } = useWorkspaceStore();
  const [editingPlan, setEditingPlan] = useState<SubscriptionPlan | null>(null);

  const { data: plans = [], isLoading } = useQuery({
    queryKey: ['subscriptions'],
    queryFn: () => ipc.getSubscriptions(),
    refetchInterval: 10_000,
  });

  const savePlanMutation = useMutation({
    mutationFn: (plan: SubscriptionPlan) => ipc.upsertSubscriptionPlan(plan),
    onSuccess: async () => {
      setEditingPlan(null);
      await queryClient.invalidateQueries({ queryKey: ['subscriptions'] });
      await queryClient.invalidateQueries({ queryKey: ['overview'] });
      await queryClient.invalidateQueries({ queryKey: ['tray_summary'] });
    },
  });

  const toggleEnabled = (status: SubscriptionPlanStatus) => {
    savePlanMutation.mutate({
      ...status.plan,
      enabled: !status.plan.enabled,
    });
  };

  return (
    <div className="space-y-6">
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div>
          <h1 className="text-[16px] font-semibold tracking-tight">订阅配额与滚动窗口预警</h1>
          <p className="text-[12px]" style={{ color: 'var(--text-muted)' }}>
            当宿主工具无公开剩余配额接口时，基于本地可观测消耗与用户订阅规则推算；若 Codex
            日志包含官方限额快照则直接忠实渲染
          </p>
        </div>
      </section>

      {isLoading ? (
        <div className="py-12 text-center" style={{ color: 'var(--text-muted)' }}>
          计算滚动窗口配额中…
        </div>
      ) : (
        <div className="divide-y" style={{ borderColor: 'var(--border-subtle)' }}>
          {plans.map((status) => {
            const { plan, windows } = status;
            return (
              <div
                key={plan.plan_id}
                className="py-5 grid grid-cols-12 gap-6 items-center"
                style={{
                  borderBottom: '1px solid var(--border-subtle)',
                  opacity: plan.enabled ? 1 : 0.55,
                }}
              >
                {/* Left Column: Provider + Plan Identity */}
                <div className="col-span-3 space-y-1.5">
                  <div className="flex items-center gap-2">
                    <span
                      className="inline-block w-2 h-2 rounded-full"
                      style={{ backgroundColor: providerAccentColor(plan.provider_id) }}
                    />
                    <span className="text-[11px] uppercase tracking-wider" style={{ color: 'var(--text-muted)' }}>
                      {providerDisplayName(plan.provider_id)}
                    </span>
                  </div>
                  <div className="text-[14px] font-semibold">{plan.plan_name}</div>
                  <div className="flex items-center gap-2.5 text-[11.5px] tabular-nums" style={{ color: 'var(--text-secondary)' }}>
                    <span>月费 {formatCost(plan.monthly_fee_micro_usd, currency, usdToCnyRate)}</span>
                    <button
                      type="button"
                      onClick={() => toggleEnabled(status)}
                      className="px-1.5 py-0.5 rounded text-[11px] cursor-pointer"
                      style={{
                        backgroundColor: 'var(--bg-hover)',
                        color: plan.enabled ? 'var(--accent-emerald)' : 'var(--text-muted)',
                      }}
                    >
                      {plan.enabled ? '已启用' : '已停用'}
                    </button>
                  </div>
                </div>

                {/* Center Column: Dual Stacked Inline Progress Bars */}
                <div className="col-span-6 space-y-3">
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
                        <div className="flex items-center justify-between text-[12px] tabular-nums">
                          <div className="flex items-center gap-2">
                            <span className="font-medium">{w.label}</span>
                            <span
                              className="px-1.5 py-0.5 text-[10px] rounded"
                              style={{
                                backgroundColor: 'var(--bg-hover)',
                                color:
                                  w.source === 'official_log_snapshot'
                                    ? 'var(--accent-emerald)'
                                    : 'var(--text-muted)',
                              }}
                            >
                              {w.source === 'official_log_snapshot' ? '官方日志快照' : '本地推算'}
                            </span>
                          </div>
                          <div>
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

                {/* Right Column: Burn Rate Projection, Reset Countdown, Edit */}
                <div className="col-span-3 flex items-center justify-between gap-3">
                  <div className="space-y-1.5 text-[11.5px] tabular-nums">
                    {windows.map((w) => (
                      <div key={w.window_id}>
                        <div style={{ color: 'var(--text-secondary)' }}>
                          {w.window_id} 重置:{' '}
                          <strong style={{ color: 'var(--text-primary)' }}>
                            {formatCountdown(w.resets_in_seconds)}
                          </strong>
                        </div>
                        <div style={{ color: 'var(--text-muted)' }}>
                          {w.projected_exhaustion_hours !== null
                            ? w.projected_exhaustion_hours <= 0
                              ? '已触及当前估算上限'
                              : `按当前流速预计 ${w.projected_exhaustion_hours.toFixed(1)}h 后触顶`
                            : '当前无明显流速'}
                        </div>
                      </div>
                    ))}
                  </div>

                  <button
                    type="button"
                    onClick={() => setEditingPlan(structuredClone(plan))}
                    className="p-1.5 rounded hover:bg-white/5 cursor-pointer shrink-0"
                    style={{
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-secondary)',
                    }}
                    title="编辑订阅推算规则"
                  >
                    <Pencil className="w-3.5 h-3.5" />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Rule Editor Drawer */}
      {editingPlan && (
        <div className="fixed inset-0 z-50 flex justify-end bg-black/50">
          <div
            className="w-[440px] h-full p-5 overflow-y-auto space-y-5"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              borderLeft: '1px solid var(--border-strong)',
            }}
          >
            <div
              className="flex items-center justify-between pb-3"
              style={{ borderBottom: '1px solid var(--border-subtle)' }}
            >
              <div>
                <h2 className="text-[14.5px] font-semibold">编辑订阅窗口推算规则</h2>
                <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                  {editingPlan.plan_name}
                </p>
              </div>
              <button
                type="button"
                onClick={() => setEditingPlan(null)}
                className="p-1 rounded hover:bg-white/5 cursor-pointer"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <div className="space-y-4">
              <div>
                <label className="block text-[11.5px] mb-1" style={{ color: 'var(--text-secondary)' }}>
                  订阅套餐名称
                </label>
                <input
                  type="text"
                  value={editingPlan.plan_name}
                  onChange={(e) =>
                    setEditingPlan({ ...editingPlan, plan_name: e.target.value })
                  }
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
                  订阅月费 (USD，用于月度回本率对比)
                </label>
                <input
                  type="number"
                  step="0.01"
                  value={(editingPlan.monthly_fee_micro_usd / 1_000_000).toFixed(2)}
                  onChange={(e) =>
                    setEditingPlan({
                      ...editingPlan,
                      monthly_fee_micro_usd: Math.max(
                        0,
                        Math.round((Number.parseFloat(e.target.value) || 0) * 1_000_000)
                      ),
                    })
                  }
                  className="w-full px-2.5 py-1.5 text-[12px] rounded tabular-nums outline-none"
                  style={{
                    backgroundColor: 'var(--bg-input)',
                    border: '1px solid var(--border-subtle)',
                    color: 'var(--text-primary)',
                  }}
                />
              </div>

              {editingPlan.windows.map((win, idx) => (
                <div
                  key={win.window_id}
                  className="pt-3 space-y-2.5"
                  style={{ borderTop: '1px solid var(--border-subtle)' }}
                >
                  <div className="text-[12.5px] font-medium">
                    窗口 #{idx + 1}: {win.label}
                  </div>
                  <div className="grid grid-cols-2 gap-2.5">
                    <div>
                      <label className="block text-[11px] mb-1" style={{ color: 'var(--text-muted)' }}>
                        周期时长 (小时)
                      </label>
                      <input
                        type="number"
                        value={win.duration_hours}
                        onChange={(e) => {
                          const next = [...editingPlan.windows];
                          next[idx] = {
                            ...win,
                            duration_hours: Math.max(1, Number.parseInt(e.target.value, 10) || 1),
                          };
                          setEditingPlan({ ...editingPlan, windows: next });
                        }}
                        className="w-full px-2 py-1 text-[12px] rounded tabular-nums"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                    </div>
                    <div>
                      <label className="block text-[11px] mb-1" style={{ color: 'var(--text-muted)' }}>
                        {win.unit === 'micro_usd' ? '估算等效额度 (USD)' : '估算额度 (Tokens)'}
                      </label>
                      <input
                        type="number"
                        value={
                          win.unit === 'micro_usd'
                            ? win.limit_value / 1_000_000
                            : win.limit_value
                        }
                        onChange={(e) => {
                          const raw = Number.parseFloat(e.target.value) || 0;
                          const next = [...editingPlan.windows];
                          next[idx] = {
                            ...win,
                            limit_value:
                              win.unit === 'micro_usd'
                                ? Math.max(1, Math.round(raw * 1_000_000))
                                : Math.max(1, Math.round(raw)),
                          };
                          setEditingPlan({ ...editingPlan, windows: next });
                        }}
                        className="w-full px-2 py-1 text-[12px] rounded tabular-nums"
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
                        value={win.warning_percent}
                        onChange={(e) => {
                          const next = [...editingPlan.windows];
                          next[idx] = {
                            ...win,
                            warning_percent: Math.min(99, Math.max(1, Number.parseInt(e.target.value, 10) || 80)),
                          };
                          setEditingPlan({ ...editingPlan, windows: next });
                        }}
                        className="w-full px-2 py-1 text-[12px] rounded tabular-nums"
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
                        value={win.critical_percent}
                        onChange={(e) => {
                          const next = [...editingPlan.windows];
                          next[idx] = {
                            ...win,
                            critical_percent: Math.min(100, Math.max(1, Number.parseInt(e.target.value, 10) || 95)),
                          };
                          setEditingPlan({ ...editingPlan, windows: next });
                        }}
                        className="w-full px-2 py-1 text-[12px] rounded tabular-nums"
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

              <div className="pt-3 flex items-center gap-2">
                <button
                  type="button"
                  disabled={savePlanMutation.isPending}
                  onClick={() => savePlanMutation.mutate(editingPlan)}
                  className="px-3.5 py-1.5 text-[12px] font-medium rounded cursor-pointer"
                  style={{
                    backgroundColor: 'var(--accent-primary)',
                    color: '#ffffff',
                  }}
                >
                  保存订阅规则
                </button>
                <button
                  type="button"
                  onClick={() => setEditingPlan(null)}
                  className="px-3 py-1.5 text-[12px] rounded cursor-pointer"
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
