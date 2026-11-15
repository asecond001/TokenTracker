import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { disable as disableAutostart, enable as enableAutostart } from '@tauri-apps/plugin-autostart';
import { Cloud, Save } from 'lucide-react';
import { ipc, isTauriRuntime } from '../lib/ipc';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type { AppPreferences, CurrencyCode, SyncEnvelopeV1, ThemeMode } from '../types/ipc';

export function PreferencesPage() {
  const queryClient = useQueryClient();
  const { setCurrency, setUsdToCnyRate, setTheme } = useWorkspaceStore();

  const { data: bootstrap } = useQuery({
    queryKey: ['bootstrap'],
    queryFn: () => ipc.getBootstrapState(),
  });

  const { data: prefs } = useQuery({
    queryKey: ['preferences'],
    queryFn: () => ipc.getPreferences(),
  });

  const [currencyDraft, setCurrencyDraft] = useState<CurrencyCode>('USD');
  const [rateDraft, setRateDraft] = useState('7.25');
  const [themeDraft, setThemeDraft] = useState<ThemeMode>('dark');
  const [minimizeToTray, setMinimizeToTray] = useState(true);
  const [launchAtStartup, setLaunchAtStartup] = useState(false);
  const [deviceDisplayName, setDeviceDisplayName] = useState('');
  const [saveNotice, setSaveNotice] = useState<string | null>(null);
  const [syncEnvelope, setSyncEnvelope] = useState<SyncEnvelopeV1 | null>(null);

  useEffect(() => {
    const src = prefs ?? bootstrap?.preferences;
    if (!src) return;
    setCurrencyDraft(src.currency ?? 'USD');
    setRateDraft(src.usd_to_cny_rate ?? '7.25');
    const normalizedTheme: ThemeMode =
      src.theme === 'light' || src.theme === 'dark' || src.theme === 'system'
        ? src.theme
        : 'dark';
    setThemeDraft(normalizedTheme);
    setMinimizeToTray(src.minimize_to_tray ?? true);
    setLaunchAtStartup(src.launch_at_startup ?? false);
    setDeviceDisplayName(src.device_display_name ?? '');
  }, [prefs, bootstrap]);

  const savePrefsMutation = useMutation({
    mutationFn: async () => {
      const base = prefs ?? bootstrap?.preferences;
      if (!base) {
        throw new Error('偏好配置尚未加载完成');
      }
      const rateNum = Number.parseFloat(rateDraft);
      if (!Number.isFinite(rateNum) || rateNum <= 0) {
        throw new Error('USD 对 CNY 汇率必须为正数');
      }

      const next: AppPreferences = {
        ...base,
        currency: currencyDraft,
        usd_to_cny_rate: rateDraft.trim(),
        theme: themeDraft,
        minimize_to_tray: minimizeToTray,
        launch_at_startup: launchAtStartup,
        device_display_name: deviceDisplayName.trim() || base.device_display_name,
      };

      const saved = await ipc.updatePreferences(next);

      if (isTauriRuntime()) {
        try {
          if (saved.launch_at_startup) {
            await enableAutostart();
          } else {
            await disableAutostart();
          }
        } catch {
          // Non-fatal if OS registry access is restricted
        }
      }

      return saved;
    },
    onSuccess: async (saved) => {
      setCurrency(saved.currency);
      setUsdToCnyRate(saved.usd_to_cny_rate);
      const nextTheme: ThemeMode =
        saved.theme === 'light' || saved.theme === 'dark' || saved.theme === 'system'
          ? saved.theme
          : 'dark';
      setTheme(nextTheme);
      setSaveNotice('偏好设置已持久化至本地 SQLite');
      await queryClient.invalidateQueries({ queryKey: ['preferences'] });
      await queryClient.invalidateQueries({ queryKey: ['bootstrap'] });
      await queryClient.invalidateQueries({ queryKey: ['tray_summary'] });
    },
    onError: (err: unknown) => {
      setSaveNotice(err instanceof Error ? err.message : '保存偏好失败');
    },
  });

  const previewSyncMutation = useMutation({
    mutationFn: () => ipc.previewSyncEnvelopeV1(0),
    onSuccess: (env) => {
      setSyncEnvelope(env);
    },
  });

  const envelopeJson = syncEnvelope ? JSON.stringify(syncEnvelope, null, 2) : null;

  return (
    <div className="space-y-6">
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div>
          <h1 className="text-[16px] font-semibold tracking-tight">偏好设置与同步契约</h1>
          <p className="text-[12px]" style={{ color: 'var(--text-muted)' }}>
            配置全局显示币种、定点汇率、深浅色主题、系统托盘行为与脱敏多端同步契约（SyncEnvelope v1）
          </p>
        </div>

        <button
          type="button"
          onClick={() => savePrefsMutation.mutate()}
          disabled={savePrefsMutation.isPending}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded text-[12px] font-medium cursor-pointer disabled:opacity-50"
          style={{
            backgroundColor: 'var(--accent-primary)',
            color: '#ffffff',
          }}
        >
          <Save className="w-3.5 h-3.5" />
          <span>{savePrefsMutation.isPending ? '保存中…' : '保存设置'}</span>
        </button>
      </section>

      {saveNotice && (
        <div
          className="px-3 py-2 text-[12px]"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            borderBottom: '1px solid var(--border-subtle)',
            color: 'var(--accent-emerald)',
          }}
        >
          {saveNotice}
        </div>
      )}

      {/* Flat Hairline-Divided Preferences Form */}
      <section style={{ borderTop: '1px solid var(--border-subtle)' }}>
        <div
          className="py-4 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">显示币种与定点汇率</div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              所有成本金额在核心层使用 micro-USD 定点数换算，避免浮点误差
            </div>
          </div>
          <div className="col-span-8 flex flex-wrap items-center gap-3">
            <select
              aria-label="显示币种"
              value={currencyDraft}
              onChange={(e) => setCurrencyDraft(e.target.value as CurrencyCode)}
              className="px-2.5 py-1 text-[12px] rounded outline-none cursor-pointer"
              style={{
                backgroundColor: 'var(--bg-input)',
                border: '1px solid var(--border-subtle)',
                color: 'var(--text-primary)',
              }}
            >
              <option value="USD">USD（美元）</option>
              <option value="CNY">CNY（人民币）</option>
            </select>

            <label className="flex items-center gap-2 text-[12px]">
              <span style={{ color: 'var(--text-secondary)' }}>1 USD =</span>
              <input
                type="text"
                aria-label="USD 对 CNY 汇率"
                value={rateDraft}
                onChange={(e) => setRateDraft(e.target.value)}
                className="w-24 px-2 py-1 text-[12px] rounded outline-none tabular-nums"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
              <span style={{ color: 'var(--text-secondary)' }}>CNY</span>
            </label>
          </div>
        </div>

        <div
          className="py-4 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">界面主题</div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              支持暗黑工作台、浅色高对比度或跟随操作系统深浅色切换
            </div>
          </div>
          <div className="col-span-8 flex items-center gap-2">
            {(
              [
                { id: 'dark', label: '深色 (Dark)' },
                { id: 'light', label: '浅色 (Light)' },
                { id: 'system', label: '跟随系统' },
              ] as const
            ).map((opt) => (
              <button
                key={opt.id}
                type="button"
                onClick={() => setThemeDraft(opt.id)}
                className="px-3 py-1 rounded text-[12px] cursor-pointer"
                style={{
                  backgroundColor:
                    themeDraft === opt.id ? 'var(--bg-hover)' : 'var(--bg-input)',
                  border:
                    themeDraft === opt.id
                      ? '1px solid var(--accent-primary)'
                      : '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </div>

        <div
          className="py-4 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">桌面常驻与开机自启</div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              关闭主窗口时最小化至系统托盘继续静默增量采集
            </div>
          </div>
          <div className="col-span-8 flex flex-wrap items-center gap-6 text-[12px]">
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={minimizeToTray}
                onChange={(e) => setMinimizeToTray(e.target.checked)}
              />
              <span>关闭窗口时最小化至托盘</span>
            </label>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={launchAtStartup}
                onChange={(e) => setLaunchAtStartup(e.target.checked)}
              />
              <span>Windows 登录后自动启动</span>
            </label>
          </div>
        </div>

        <div
          className="py-4 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">本地设备与 SQLite 存储</div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              仅导出脱敏聚合统计时使用的匿名设备别名与本地 WAL 数据库路径
            </div>
          </div>
          <div className="col-span-8 space-y-2 text-[12px]">
            <div className="flex flex-wrap items-center gap-3">
              <span style={{ color: 'var(--text-secondary)' }}>设备显示名：</span>
              <input
                type="text"
                value={deviceDisplayName}
                onChange={(e) => setDeviceDisplayName(e.target.value)}
                className="w-56 px-2.5 py-1 text-[12px] rounded outline-none"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
              <span
                className="text-[11px] font-mono tabular-nums"
                style={{ color: 'var(--text-muted)' }}
              >
                ID: {bootstrap?.device.device_id ?? '—'}
              </span>
            </div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              SQLite WAL 数据库路径：
              <code className="ml-1 select-text">{bootstrap?.db_path ?? '—'}</code>
            </div>
          </div>
        </div>
      </section>

      {/* SyncEnvelope v1 Contract Preview (Phase 2 Ready) */}
      <section className="space-y-3 pb-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="space-y-0.5">
            <div className="flex items-center gap-2">
              <Cloud className="w-4 h-4" style={{ color: 'var(--accent-sky)' }} />
              <h2 className="text-[13.5px] font-semibold">
                多端同步契约预览（SyncEnvelope v1）
              </h2>
            </div>
            <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              为后续 Cloudflare Worker 云端同步与纯原生移动端准备：严格仅含小时聚合计数、等效金额与订阅快照，零 Prompt 文本或密钥
            </p>
          </div>

          <button
            type="button"
            onClick={() => previewSyncMutation.mutate()}
            disabled={previewSyncMutation.isPending}
            className="px-3 py-1.5 rounded text-[12px] cursor-pointer"
            style={{
              backgroundColor: 'var(--bg-hover)',
              color: 'var(--text-primary)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            {previewSyncMutation.isPending ? '生成中…' : '生成脱敏同步负载预览'}
          </button>
        </div>

        {syncEnvelope && envelopeJson && (
          <div
            className="p-3 rounded space-y-2"
            style={{
              backgroundColor: 'var(--bg-elevated)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            <div
              className="flex flex-wrap items-center gap-4 text-[11px] tabular-nums"
              style={{ color: 'var(--text-secondary)' }}
            >
              <span>schema_version: {syncEnvelope.schema_version}</span>
              <span>小时聚合条数: {syncEnvelope.hourly_deltas.length}</span>
              <span>订阅快照数: {syncEnvelope.subscription_snapshots.length}</span>
              <span>负载体积: {envelopeJson.length.toLocaleString('en-US')} Bytes</span>
            </div>
            <pre
              className="m-0 max-h-64 overflow-auto text-[11px] font-mono select-text"
              style={{ color: 'var(--text-secondary)' }}
            >
              {envelopeJson}
            </pre>
          </div>
        )}
      </section>
    </div>
  );
}
