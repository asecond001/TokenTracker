import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ExternalLink, RefreshCw } from 'lucide-react';
import { InlineProgressBar } from '../components/InlineProgressBar';
import {
  formatCost,
  formatCountdown,
  formatTokens,
  formatTps,
  providerDisplayName,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';

export function TrayPopupPage() {
  const queryClient = useQueryClient();
  const { tzOffsetMinutes } = useWorkspaceStore();

  const { data: summary } = useQuery({
    queryKey: ['tray_summary', tzOffsetMinutes],
    queryFn: () => ipc.getTraySummary(tzOffsetMinutes),
    refetchInterval: 10_000,
  });

  const scanMutation = useMutation({
    mutationFn: () => ipc.triggerScanNow(),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['tray_summary'] });
    },
  });

  const currency = summary?.currency ?? 'USD';
  const rate = summary?.usd_to_cny_rate ?? '7.25';

  return (
    <div
      className="h-screen w-screen flex flex-col justify-between p-3.5 select-none"
      style={{
        backgroundColor: 'var(--bg-canvas)',
        color: 'var(--text-primary)',
        border: '1px solid var(--border-strong)',
      }}
    >
      <div className="space-y-3 overflow-y-auto pr-0.5">
        {/* Compact Header */}
        <div
          className="flex items-center justify-between pb-2"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="flex items-center gap-2">
            <span className="inline-block w-2 h-2 rounded-full collector-dot-live" style={{ backgroundColor: 'var(--accent-emerald)' }} />
            <span className="text-[12.5px] font-semibold">TokenTracker 实时摘要</span>
          </div>
          <span
            className="text-[11px] font-medium tabular-nums"
            style={{ color: 'var(--accent-sky)' }}
          >
            {formatTps(summary?.latest_output_tps)}
          </span>
        </div>

        {/* Today Metrics Grid (Flat, Hairline Divided, No Shadow Cards) */}
        <div
          className="grid grid-cols-2 gap-3 pb-3 tabular-nums"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div>
            <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
              今日等效金额
            </div>
            <div className="text-[18px] font-semibold tracking-tight">
              {formatCost(summary?.today_cost_micro_usd ?? 0, currency, rate)}
            </div>
          </div>
          <div>
            <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
              今日总 Token
            </div>
            <div className="text-[18px] font-semibold tracking-tight">
              {formatTokens(summary?.today_tokens ?? 0)}
            </div>
          </div>
          <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
            今日请求：{summary?.today_requests ?? 0} 次
          </div>
          <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
            启用数据源：{summary?.active_providers_count ?? 0} 个
          </div>
        </div>

        {/* Active Subscription Windows */}
        <div className="space-y-2.5">
          <div
            className="text-[10.5px] font-semibold uppercase tracking-wider"
            style={{ color: 'var(--text-muted)' }}
          >
            订阅窗口配额
          </div>
          {(summary?.subscriptions ?? []).slice(0, 3).map((sub) => {
            const win = sub.windows[0];
            if (!win) return null;
            return (
              <div key={sub.plan.plan_id} className="space-y-1">
                <div className="flex items-center justify-between text-[11.5px] tabular-nums">
                  <span className="truncate max-w-[190px]" style={{ color: 'var(--text-primary)' }}>
                    {providerDisplayName(sub.plan.provider_id)} · {win.label}
                  </span>
                  <span style={{ color: 'var(--text-secondary)' }}>
                    {win.used_percent.toFixed(1)}% · {formatCountdown(win.resets_in_seconds)}
                  </span>
                </div>
                <InlineProgressBar
                  percent={win.used_percent}
                  alertLevel={win.alert_level}
                  heightPx={4}
                />
              </div>
            );
          })}
          {(summary?.subscriptions ?? []).length === 0 && (
            <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
              暂无启用的订阅配额规则
            </div>
          )}
        </div>
      </div>

      {/* Bottom Actions */}
      <div
        className="pt-2.5 flex items-center justify-between gap-2"
        style={{ borderTop: '1px solid var(--border-subtle)' }}
      >
        <button
          type="button"
          onClick={() => scanMutation.mutate()}
          disabled={scanMutation.isPending}
          className="flex-1 inline-flex items-center justify-center gap-1.5 py-1.5 rounded text-[11.5px] cursor-pointer"
          style={{
            backgroundColor: 'var(--bg-hover)',
            color: 'var(--text-primary)',
            border: '1px solid var(--border-subtle)',
          }}
        >
          <RefreshCw className={`w-3 h-3 ${scanMutation.isPending ? 'animate-spin' : ''}`} />
          <span>立即扫描</span>
        </button>

        <button
          type="button"
          onClick={() => {
            void ipc.openMainWorkspace();
          }}
          className="flex-1 inline-flex items-center justify-center gap-1.5 py-1.5 rounded text-[11.5px] font-medium cursor-pointer"
          style={{
            backgroundColor: 'var(--accent-primary)',
            color: '#ffffff',
          }}
        >
          <ExternalLink className="w-3 h-3" />
          <span>打开工作台</span>
        </button>
      </div>
    </div>
  );
}
