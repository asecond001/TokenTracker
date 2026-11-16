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
import { AnimatedNumber } from './AnimatedNumber';
import { formatCost } from '../lib/format';
import { useUpdaterStore } from '../lib/updater';
import { useWorkspaceStore, type NavPage } from '../stores/workspaceStore';
import type { TraySummary } from '../types/ipc';

export interface SidebarProps {
  traySummary?: TraySummary;
  hasBackdrop?: boolean;
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

export function Sidebar({ traySummary, hasBackdrop = false }: SidebarProps) {
  const { activePage, setActivePage, currency, usdToCnyRate } = useWorkspaceStore();
  const availableUpdate = useUpdaterStore((s) => s.availableUpdate);

  const borderToken = hasBackdrop ? 'var(--border-hairline)' : 'var(--border-subtle)';

  const renderItem = (item: { id: NavPage; label: string; icon: typeof LayoutDashboard }) => {
    const active = activePage === item.id;
    const Icon = item.icon;
    return (
      <button
        key={item.id}
        type="button"
        data-selected={active ? 'true' : undefined}
        onClick={() => setActivePage(item.id)}
        className="row-interactive nav-item relative w-full flex items-center gap-2.5 px-2.5 py-1.5 text-left rounded text-[12.5px] cursor-pointer"
        style={{
          color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
          fontWeight: active ? 500 : 400,
        }}
      >
        <span
          className="absolute left-0 top-1.5 bottom-1.5 w-[2px] rounded-full transition-all duration-200"
          style={{
            backgroundColor: 'var(--accent-primary)',
            opacity: active ? 1 : 0,
            transform: active ? 'scaleY(1)' : 'scaleY(0.35)',
          }}
        />
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
      className="relative z-10 flex flex-col justify-between shrink-0 h-full"
      style={{
        width: '208px',
        backgroundColor: hasBackdrop ? 'var(--bg-translucent-chrome)' : 'var(--bg-sidebar)',
        backdropFilter: hasBackdrop ? 'blur(14px)' : undefined,
        WebkitBackdropFilter: hasBackdrop ? 'blur(14px)' : undefined,
        borderRight: `1px solid ${borderToken}`,
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
            className="px-2.5 pb-1.5 flex items-center justify-between text-[10.5px] font-semibold uppercase tracking-wider"
            style={{ color: 'var(--text-muted)' }}
          >
            <span>系统配置</span>
            <div className="flex items-center gap-1.5">
              {availableUpdate && (
                <button
                  type="button"
                  onClick={() => setActivePage('preferences')}
                  className="px-1.5 py-0.5 rounded text-[9.5px] font-medium normal-case tracking-normal cursor-pointer hover:opacity-90 tabular-nums"
                  style={{
                    backgroundColor: 'rgba(16, 185, 129, 0.14)',
                    color: 'var(--accent-emerald)',
                  }}
                  title={`发现新版本 v${availableUpdate.version}，点击前往偏好设置安装`}
                >
                  有新版本 v{availableUpdate.version}
                </button>
              )}
              <span className="font-mono text-[10px] font-normal lowercase tabular-nums">
                v0.2.3
              </span>
            </div>
          </div>
          <nav className="space-y-0.5">{SYSTEM_NAV.map(renderItem)}</nav>
        </div>
      </div>

      {/* Bottom Pinned 1-Line Live Summary */}
      <div
        className="px-3 py-2.5 flex items-center justify-between text-[11.5px] tabular-nums"
        style={{ borderTop: `1px solid ${borderToken}` }}
      >
        <span style={{ color: 'var(--text-secondary)' }}>
          今日{' '}
          <strong style={{ color: 'var(--text-primary)', fontWeight: 600 }}>
            <AnimatedNumber
              value={traySummary?.today_cost_micro_usd ?? 0}
              formatter={(microUsd) => formatCost(microUsd, currency, usdToCnyRate)}
            />
          </strong>
        </span>
        <span style={{ color: 'var(--text-muted)' }}>
          {(traySummary?.total_ingested_events ?? 0).toLocaleString('en-US')} 条
        </span>
      </div>
    </aside>
  );
}
