import { useEffect, useState } from 'react';
import { Check, Circle, Loader2, XCircle } from 'lucide-react';
import type { CloudSyncDeployMode, CloudSyncProvisionStep } from '../types/ipc';

const STEP_LABELS: Record<CloudSyncProvisionStep, string> = {
  preparing: '准备文件',
  checking_auth: '检查 Wrangler 登录',
  browser_login: '浏览器授权',
  resolving_account: '识别账户',
  deploying: '上传 Worker',
  writing_secret: '写入令牌',
  saving: '保存连接',
  initial_sync: '首次同步',
};

export function provisionStepPlan(
  mode: CloudSyncDeployMode,
  seen: readonly CloudSyncProvisionStep[]
): CloudSyncProvisionStep[] {
  if (mode === 'cloudflare_api') {
    return ['preparing', 'resolving_account', 'deploying', 'writing_secret', 'saving', 'initial_sync'];
  }
  const plan: CloudSyncProvisionStep[] = ['preparing', 'checking_auth'];
  if (seen.includes('browser_login')) {
    plan.push('browser_login');
  }
  plan.push('deploying', 'writing_secret', 'saving', 'initial_sync');
  return plan;
}

export interface ProvisionProgressProps {
  mode: CloudSyncDeployMode;
  seen: readonly CloudSyncProvisionStep[];
  status: 'running' | 'failed';
  startedAtMs: number;
}

/** Inline step strip shown beside the 一键配置 button while provisioning runs. */
export function ProvisionProgress({ mode, seen, status, startedAtMs }: ProvisionProgressProps) {
  const [nowMs, setNowMs] = useState(() => Date.now());

  useEffect(() => {
    if (status !== 'running') {
      return;
    }
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [status]);

  const plan = provisionStepPlan(mode, seen);
  const current = seen.length > 0 ? seen[seen.length - 1] : 'preparing';
  const currentIdx = Math.max(0, plan.indexOf(current));
  const elapsedSec = Math.max(0, Math.floor((nowMs - startedAtMs) / 1000));

  return (
    <ol
      data-testid="provision-progress"
      aria-label="一键配置进度"
      className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]"
    >
      {plan.map((step, idx) => {
        const done = idx < currentIdx;
        const active = idx === currentIdx;
        const failed = active && status === 'failed';
        let color = 'var(--text-muted)';
        if (done) color = 'var(--accent-emerald)';
        if (active) color = failed ? 'var(--accent-rose)' : 'var(--text-primary)';
        return (
          <li
            key={step}
            data-step={step}
            data-state={failed ? 'failed' : done ? 'done' : active ? 'active' : 'pending'}
            className="inline-flex items-center gap-1 whitespace-nowrap"
            style={{ color, fontWeight: active ? 500 : 400 }}
          >
            {done && <Check className="w-3 h-3" />}
            {active && !failed && <Loader2 className="w-3 h-3 animate-spin" />}
            {failed && <XCircle className="w-3 h-3" />}
            {!done && !active && <Circle className="w-2 h-2" />}
            <span>{STEP_LABELS[step]}</span>
          </li>
        );
      })}
      {status === 'running' && (
        <li className="tabular-nums" style={{ color: 'var(--text-muted)' }}>
          {elapsedSec}s
        </li>
      )}
    </ol>
  );
}
