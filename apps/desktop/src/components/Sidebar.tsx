import {
  Activity,
  BarChart3,
  CalendarDays,
  Cpu,
  Database,
  Gauge,
  LayoutDashboard,
  Settings,
} from 'lucide-react';
import { formatCost } from '../lib/format';
import { useWorkspaceStore, type NavPage } from '../stores/workspaceStore';
import type { TraySummary } from '../types/ipc';

interface SidebarProps {
  traySummary?: TraySummary;
}

const ANALYTICS_NAV: Array<{ id: NavPage; label: string; icon: typeof LayoutDashboard }> = [
  { id: 'overview', label: '概览', icon: LayoutDashboard },
  { id: 'trends', label: '趋势分析', icon: Activity },
  { id: 'monthly', label: '月度统计', icon: CalendarDays },
  { id: 'models', label: '模型与价格', icon: Cpu },
  { id: 'subscriptions', label: '订阅用量', icon: BarChart3 },
  { id: 'speed', label: '速度监测', icon: Gauge },
];

const SYSTEM_NAV: Array<{ id: NavPage; label: string; icon: typeof Database }> = [
  { id: 'sources', label: '数据源与合规', icon: Database },
  { id: 'preferences', label: '偏好设置', icon: Settings },
];

export function Sidebar({ traySummary }: SidebarProps) {
  const { activePage, setActivePage, currency, usdToCnyRate } = useWorkspaceStore();

  const renderItem = (item: { id: NavPage; label: string; icon: typeof LayoutDashboard }) => {
    const active = activePage === item.id;
    const Icon = item.icon;
    return (
      <button
        key={item.id}
        type="button"
        onClick={() => setActivePage(item.id)}
        className="w-full flex items-center gap-2.5 px-2.5 py-1.5 text-left rounded text-[12.5px] transition-colors cursor-pointer"
        style={{
          backgroundColor: active ? 'var(--bg-hover)' : 'transparent',
          color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
          fontWeight: active ? 500 : 400,
        }}
      >
        <Icon
          className="w-4 h-4 shrink-0"
          style={{
            color: active ? 'var(--accent-primary)' : 'var(--text-muted)',
          }}
        />
        <span>{item.label}</span>
      </button>
    );
  };

  return (
    <aside
      className="flex flex-col justify-between shrink-0 h-full"
      style={{
        width: '208px',
        backgroundColor: 'var(--bg-sidebar)',
        borderRight: '1px solid var(--border-subtle)',
      }}
    >
      <div className="p-2.5 space-y-5 overflow-y-auto">
        <div>
          <div
            className="px-2.5 pb-1.5 text-[10.5px] font-semibold uppercase tracking-wider"
            style={{ color: 'var(--text-muted)' }}
          >
            监测视图
          </div>
          <nav className="space-y-0.5">{ANALYTICS_NAV.map(renderItem)}</nav>
        </div>

        <div>
          <div
            className="px-2.5 pb-1.5 text-[10.5px] font-semibold uppercase tracking-wider"
            style={{ color: 'var(--text-muted)' }}
          >
            系统配置
          </div>
          <nav className="space-y-0.5">{SYSTEM_NAV.map(renderItem)}</nav>
        </div>
      </div>

      {/* Bottom Pinned 1-Line Live Summary */}
      <div
        className="px-3 py-2.5 flex items-center justify-between text-[11.5px] tabular-nums"
        style={{ borderTop: '1px solid var(--border-subtle)' }}
      >
        <span style={{ color: 'var(--text-secondary)' }}>
          今日{' '}
          <strong style={{ color: 'var(--text-primary)', fontWeight: 600 }}>
            {formatCost(traySummary?.today_cost_micro_usd ?? 0, currency, usdToCnyRate)}
          </strong>
        </span>
        <span style={{ color: 'var(--text-muted)' }}>
          {(traySummary?.total_ingested_events ?? 0).toLocaleString('en-US')} 条
        </span>
      </div>
    </aside>
  );
}
