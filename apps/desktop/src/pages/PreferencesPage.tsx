import { useCallback, useEffect, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { disable as disableAutostart, enable as enableAutostart } from '@tauri-apps/plugin-autostart';
import { open } from '@tauri-apps/plugin-dialog';
import {
  Cloud,
  Download,
  Image as ImageIcon,
  KeyRound,
  RefreshCw,
  RotateCcw,
  Save,
  ShieldCheck,
  Sparkles,
} from 'lucide-react';
import { InlineProgressBar } from '../components/InlineProgressBar';
import {
  BACKGROUND_GRADIENT_PRESETS,
  type BackgroundPresetId,
} from '../components/WorkspaceBackdrop';
import { formatRelativeTime } from '../lib/format';
import { ipc, isTauriRuntime } from '../lib/ipc';
import {
  checkForUpdates,
  downloadAndInstallUpdate,
  useUpdaterStore,
} from '../lib/updater';
import { syncDocumentAndNativeTheme, useWorkspaceStore } from '../stores/workspaceStore';
import type {
  AppPreferences,
  BootstrapState,
  CurrencyCode,
  SyncEnvelopeV1,
  SyncStatusView,
  ThemeMode,
} from '../types/ipc';

export interface PreferencesPageProps {
  onApplyTheme?: (nextTheme: ThemeMode) => void;
  onPreviewPreferences?: (patch: Partial<AppPreferences>) => AppPreferences | null;
  onPersistPreferences?: (patch: Partial<AppPreferences>) => Promise<AppPreferences>;
}

interface NoticeState {
  text: string;
  kind: 'success' | 'error';
}

interface SyncedFormSnapshot {
  currency: CurrencyCode;
  usd_to_cny_rate: string;
  minimize_to_tray: boolean;
  launch_at_startup: boolean;
  floating_window_enabled: boolean;
  device_display_name: string;
  auto_sync_pricing: boolean;
  exchange_rate_auto: boolean;
  auto_check_updates: boolean;
}

type BackgroundMode = 'none' | 'gradient' | 'image';

const THEME_OPTIONS: ReadonlyArray<{ id: ThemeMode; label: string }> = [
  { id: 'dark', label: '深色 (Dark)' },
  { id: 'light', label: '浅色 (Light)' },
  { id: 'system', label: '跟随系统' },
];

const BG_MODE_OPTIONS: ReadonlyArray<{ id: BackgroundMode; label: string }> = [
  { id: 'none', label: '纯色默认' },
  { id: 'gradient', label: '微光渐变' },
  { id: 'image', label: '本地壁纸' },
];

function normalizeThemeMode(raw: string | undefined | null): ThemeMode {
  return raw === 'light' || raw === 'dark' || raw === 'system' ? raw : 'dark';
}

function normalizeBgMode(raw: string | undefined | null): BackgroundMode {
  return raw === 'gradient' || raw === 'image' ? raw : 'none';
}

function clampOpacity(raw: number | undefined): number {
  if (raw === undefined || !Number.isFinite(raw)) return 0.18;
  return Math.min(0.6, Math.max(0.05, raw));
}

function clampBlurPx(raw: number | undefined): number {
  if (raw === undefined || !Number.isFinite(raw)) return 16;
  return Math.round(Math.min(32, Math.max(0, raw)));
}

export function PreferencesPage({
  onApplyTheme,
  onPreviewPreferences,
  onPersistPreferences,
}: PreferencesPageProps) {
  const queryClient = useQueryClient();
  const {
    theme,
    resolvedTheme,
    setCurrency,
    setUsdToCnyRate,
    setTheme,
  } = useWorkspaceStore();

  const { data: bootstrap } = useQuery({
    queryKey: ['bootstrap'],
    queryFn: () => ipc.getBootstrapState(),
  });

  const { data: prefs } = useQuery({
    queryKey: ['preferences'],
    queryFn: () => ipc.getPreferences(),
  });

  const { data: syncStatus } = useQuery({
    queryKey: ['sync-status'],
    queryFn: () => ipc.getSyncStatus(),
    refetchInterval: 15_000,
  });

  const [currencyDraft, setCurrencyDraft] = useState<CurrencyCode>('USD');
  const [rateDraft, setRateDraft] = useState('7.25');
  const [minimizeToTray, setMinimizeToTray] = useState(true);
  const [launchAtStartup, setLaunchAtStartup] = useState(false);
  const [floatingWindowEnabled, setFloatingWindowEnabled] = useState(false);
  const [deviceDisplayName, setDeviceDisplayName] = useState('');
  const [autoSyncPricing, setAutoSyncPricing] = useState(true);
  const [exchangeRateAuto, setExchangeRateAuto] = useState(true);
  const [autoCheckUpdates, setAutoCheckUpdates] = useState(true);
  const [syncEnabledDraft, setSyncEnabledDraft] = useState(false);
  const [syncWorkerUrlDraft, setSyncWorkerUrlDraft] = useState('');
  const [syncE2eeDraft, setSyncE2eeDraft] = useState(false);
  const [syncBearerTokenInput, setSyncBearerTokenInput] = useState('');
  const [syncPassphraseInput, setSyncPassphraseInput] = useState('');
  const [notice, setNotice] = useState<NoticeState | null>(null);
  const [syncEnvelope, setSyncEnvelope] = useState<SyncEnvelopeV1 | null>(null);
  const updater = useUpdaterStore();

  const lastSyncedSnapshotRef = useRef<SyncedFormSnapshot | null>(null);
  const lastSyncCfgSnapshotRef = useRef<{
    enabled: boolean;
    worker_url: string;
    e2ee_enabled: boolean;
  } | null>(null);
  const sliderDraftRef = useRef<{ bg_opacity?: number; bg_blur_px?: number }>({});
  const sliderTimerRef = useRef<number | null>(null);

  const activePrefs = prefs ?? bootstrap?.preferences ?? null;
  const bgMode = normalizeBgMode(activePrefs?.bg_mode);
  const bgPresetId = activePrefs?.bg_preset_id ?? 'aurora_slate';
  const bgImagePath = activePrefs?.bg_image_path ?? null;
  const bgOpacity = clampOpacity(activePrefs?.bg_opacity);
  const bgBlurPx = clampBlurPx(activePrefs?.bg_blur_px);

  useEffect(() => {
    const src = prefs ?? bootstrap?.preferences;
    if (!src) return;

    const prev = lastSyncedSnapshotRef.current;
    const nextCurrency: CurrencyCode = src.currency ?? 'USD';
    const nextRate = src.usd_to_cny_rate ?? '7.25';
    const nextMinimize = src.minimize_to_tray ?? true;
    const nextAutostart = src.launch_at_startup ?? false;
    const nextFloating = src.floating_window_enabled ?? false;
    const nextDeviceName = src.device_display_name ?? '';
    const nextAutoSync = src.auto_sync_pricing ?? true;
    const nextRateAuto = src.exchange_rate_auto ?? true;
    const nextAutoUpdates = src.auto_check_updates ?? true;

    if (!prev || prev.currency !== nextCurrency) {
      setCurrencyDraft(nextCurrency);
    }
    if (!prev || prev.usd_to_cny_rate !== nextRate) {
      setRateDraft(nextRate);
    }
    if (!prev || prev.minimize_to_tray !== nextMinimize) {
      setMinimizeToTray(nextMinimize);
    }
    if (!prev || prev.launch_at_startup !== nextAutostart) {
      setLaunchAtStartup(nextAutostart);
    }
    if (!prev || prev.floating_window_enabled !== nextFloating) {
      setFloatingWindowEnabled(nextFloating);
    }
    if (!prev || prev.device_display_name !== nextDeviceName) {
      setDeviceDisplayName(nextDeviceName);
    }
    if (!prev || prev.auto_sync_pricing !== nextAutoSync) {
      setAutoSyncPricing(nextAutoSync);
    }
    if (!prev || prev.exchange_rate_auto !== nextRateAuto) {
      setExchangeRateAuto(nextRateAuto);
    }
    if (!prev || prev.auto_check_updates !== nextAutoUpdates) {
      setAutoCheckUpdates(nextAutoUpdates);
    }

    lastSyncedSnapshotRef.current = {
      currency: nextCurrency,
      usd_to_cny_rate: nextRate,
      minimize_to_tray: nextMinimize,
      launch_at_startup: nextAutostart,
      floating_window_enabled: nextFloating,
      device_display_name: nextDeviceName,
      auto_sync_pricing: nextAutoSync,
      exchange_rate_auto: nextRateAuto,
      auto_check_updates: nextAutoUpdates,
    };
  }, [prefs, bootstrap]);

  useEffect(() => {
    if (!syncStatus) return;
    const nextUrl = syncStatus.worker_url ?? '';
    const prev = lastSyncCfgSnapshotRef.current;
    if (!prev || prev.enabled !== syncStatus.enabled) {
      setSyncEnabledDraft(syncStatus.enabled);
    }
    if (!prev || prev.worker_url !== nextUrl) {
      setSyncWorkerUrlDraft(nextUrl);
    }
    if (!prev || prev.e2ee_enabled !== syncStatus.e2ee_enabled) {
      setSyncE2eeDraft(syncStatus.e2ee_enabled);
    }
    lastSyncCfgSnapshotRef.current = {
      enabled: syncStatus.enabled,
      worker_url: nextUrl,
      e2ee_enabled: syncStatus.e2ee_enabled,
    };
  }, [syncStatus]);

  const previewPreferences = useCallback(
    (patch: Partial<AppPreferences>): AppPreferences | null => {
      if (onPreviewPreferences) {
        return onPreviewPreferences(patch);
      }

      if (patch.theme !== undefined) {
        const normalized = normalizeThemeMode(patch.theme);
        setTheme(normalized);
        syncDocumentAndNativeTheme(normalized);
      }
      if (patch.currency !== undefined) {
        setCurrency(patch.currency);
      }
      if (patch.usd_to_cny_rate !== undefined) {
        setUsdToCnyRate(patch.usd_to_cny_rate);
      }

      const base =
        queryClient.getQueryData<AppPreferences>(['preferences']) ??
        queryClient.getQueryData<BootstrapState>(['bootstrap'])?.preferences;
      if (!base) {
        return null;
      }

      const next: AppPreferences = {
        ...base,
        ...patch,
      };
      queryClient.setQueryData<AppPreferences>(['preferences'], next);
      queryClient.setQueryData<BootstrapState>(['bootstrap'], (prev) =>
        prev ? { ...prev, preferences: next } : prev
      );
      return next;
    },
    [onPreviewPreferences, queryClient, setCurrency, setTheme, setUsdToCnyRate]
  );

  const persistPreferences = useCallback(
    async (patch: Partial<AppPreferences>): Promise<AppPreferences> => {
      if (onPersistPreferences) {
        return onPersistPreferences(patch);
      }

      let next = previewPreferences(patch);
      if (!next) {
        const fetched = await ipc.getPreferences();
        next = { ...fetched, ...patch };
      }
      const saved = await ipc.updatePreferences(next);
      queryClient.setQueryData<AppPreferences>(['preferences'], saved);
      queryClient.setQueryData<BootstrapState>(['bootstrap'], (prev) =>
        prev ? { ...prev, preferences: saved } : prev
      );
      return saved;
    },
    [onPersistPreferences, previewPreferences, queryClient]
  );

  const persistPreferencesRef = useRef(persistPreferences);
  persistPreferencesRef.current = persistPreferences;

  const flushSliderPersist = useCallback(() => {
    if (sliderTimerRef.current !== null) {
      window.clearTimeout(sliderTimerRef.current);
      sliderTimerRef.current = null;
    }
    const pending = sliderDraftRef.current;
    if (pending.bg_opacity === undefined && pending.bg_blur_px === undefined) {
      return;
    }
    sliderDraftRef.current = {};
    void persistPreferencesRef.current(pending).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '更新背景参数失败',
        kind: 'error',
      });
    });
  }, []);

  useEffect(() => {
    return () => {
      flushSliderPersist();
    };
  }, [flushSliderPersist]);

  const handleThemeSelect = (nextTheme: ThemeMode) => {
    if (onApplyTheme) {
      onApplyTheme(nextTheme);
      return;
    }
    void persistPreferences({ theme: nextTheme });
  };

  const handleBgModeSelect = (nextMode: BackgroundMode) => {
    void persistPreferences({ bg_mode: nextMode }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换背景模式失败',
        kind: 'error',
      });
    });
  };

  const handlePresetSelect = (presetId: BackgroundPresetId) => {
    void persistPreferences({
      bg_mode: 'gradient',
      bg_preset_id: presetId,
    }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '应用渐变预设失败',
        kind: 'error',
      });
    });
  };

  const handleSliderPreview = (patch: { bg_opacity?: number; bg_blur_px?: number }) => {
    sliderDraftRef.current = {
      ...sliderDraftRef.current,
      ...patch,
    };
    previewPreferences(patch);

    if (sliderTimerRef.current !== null) {
      window.clearTimeout(sliderTimerRef.current);
    }
    sliderTimerRef.current = window.setTimeout(() => {
      flushSliderPersist();
    }, 180);
  };

  const importBgMutation = useMutation({
    mutationFn: (sourcePath: string) => ipc.importBackgroundImage(sourcePath),
    onSuccess: async (filename) => {
      previewPreferences({
        bg_mode: 'image',
        bg_image_path: filename,
      });
      await queryClient.invalidateQueries({ queryKey: ['bg-image-data-url'] });
      if (isTauriRuntime()) {
        await queryClient.invalidateQueries({ queryKey: ['preferences'] });
        await queryClient.invalidateQueries({ queryKey: ['bootstrap'] });
      }
      setNotice({ text: `已导入本地壁纸：${filename}`, kind: 'success' });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '导入壁纸失败',
        kind: 'error',
      });
    },
  });

  const clearBgMutation = useMutation({
    mutationFn: () => ipc.clearBackgroundImage(),
    onSuccess: async () => {
      previewPreferences({
        bg_image_path: null,
        bg_mode: 'none',
      });
      queryClient.removeQueries({ queryKey: ['bg-image-data-url'] });
      if (isTauriRuntime()) {
        await queryClient.invalidateQueries({ queryKey: ['preferences'] });
        await queryClient.invalidateQueries({ queryKey: ['bootstrap'] });
      }
      setNotice({ text: '已清除自定义壁纸并恢复纯色默认', kind: 'success' });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '清除壁纸失败',
        kind: 'error',
      });
    },
  });

  const handlePickBackgroundImage = async () => {
    if (!isTauriRuntime()) {
      setNotice({
        text: '本地壁纸文件选择器仅在桌面端原生运行时可用',
        kind: 'error',
      });
      return;
    }
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        title: '选择工作台背景壁纸',
        filters: [
          {
            name: '图片文件',
            extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp'],
          },
        ],
      });
      if (typeof selected === 'string' && selected.trim()) {
        await importBgMutation.mutateAsync(selected);
      }
    } catch (err: unknown) {
      setNotice({
        text: err instanceof Error ? err.message : '打开文件选择器失败',
        kind: 'error',
      });
    }
  };

  const savePrefsMutation = useMutation({
    mutationFn: async () => {
      if (sliderTimerRef.current !== null) {
        window.clearTimeout(sliderTimerRef.current);
        sliderTimerRef.current = null;
      }
      const pendingSliders = sliderDraftRef.current;
      sliderDraftRef.current = {};

      const base =
        queryClient.getQueryData<AppPreferences>(['preferences']) ??
        prefs ??
        bootstrap?.preferences;
      if (!base) {
        throw new Error('偏好配置尚未加载完成');
      }

      const rateNum = Number.parseFloat(rateDraft);
      if (!Number.isFinite(rateNum) || rateNum <= 0) {
        throw new Error('USD 对 CNY 汇率必须为正数');
      }

      const currentTheme = useWorkspaceStore.getState().theme;
      const patch: Partial<AppPreferences> = {
        currency: currencyDraft,
        usd_to_cny_rate: rateDraft.trim(),
        theme: currentTheme,
        minimize_to_tray: minimizeToTray,
        launch_at_startup: launchAtStartup,
        floating_window_enabled: floatingWindowEnabled,
        device_display_name: deviceDisplayName.trim() || base.device_display_name,
        auto_sync_pricing: autoSyncPricing,
        exchange_rate_auto: exchangeRateAuto,
        auto_check_updates: autoCheckUpdates,
        ...pendingSliders,
      };

      const saved = await persistPreferences(patch);

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
      const nextTheme = normalizeThemeMode(saved.theme);
      setTheme(nextTheme);
      syncDocumentAndNativeTheme(nextTheme);

      lastSyncedSnapshotRef.current = {
        currency: saved.currency,
        usd_to_cny_rate: saved.usd_to_cny_rate,
        minimize_to_tray: saved.minimize_to_tray,
        launch_at_startup: saved.launch_at_startup,
        floating_window_enabled: saved.floating_window_enabled,
        device_display_name: saved.device_display_name,
        auto_sync_pricing: saved.auto_sync_pricing,
        exchange_rate_auto: saved.exchange_rate_auto,
        auto_check_updates: saved.auto_check_updates,
      };

      setNotice({ text: '偏好设置已持久化至本地 SQLite', kind: 'success' });
      await queryClient.invalidateQueries({ queryKey: ['tray_summary'] });
      await queryClient.invalidateQueries({ queryKey: ['tray-summary'] });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '保存偏好失败',
        kind: 'error',
      });
    },
  });

  const syncNowMutation = useMutation({
    mutationFn: () => ipc.syncPricingAndRate(true),
    onSuccess: async (res) => {
      if (res.rate_updated) {
        setRateDraft(res.usd_to_cny_rate);
        setUsdToCnyRate(res.usd_to_cny_rate);
      }
      await queryClient.invalidateQueries({
        predicate: (query) => query.queryKey[0] !== 'bg-image-data-url',
      });
      if (res.warning && !res.rate_updated && !res.pricing_updated) {
        setNotice({
          text: `在线刷新受限（已保持本地基线）：${res.warning}`,
          kind: 'error',
        });
      } else {
        setNotice({
          text: `已刷新汇率与公开模型单价（1 USD = ${res.usd_to_cny_rate} CNY · ${res.synced_models_count} 款模型）`,
          kind: 'success',
        });
      }
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '刷新汇率与模型单价失败',
        kind: 'error',
      });
    },
  });

  const handleExchangeRateAutoToggle = (auto: boolean) => {
    setExchangeRateAuto(auto);
    void persistPreferences({ exchange_rate_auto: auto }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换汇率模式失败',
        kind: 'error',
      });
    });
  };

  const handleAutoSyncPricingToggle = (enabled: boolean) => {
    setAutoSyncPricing(enabled);
    void persistPreferences({ auto_sync_pricing: enabled }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换自动更新设置失败',
        kind: 'error',
      });
    });
  };

  const handleAutoCheckUpdatesToggle = (enabled: boolean) => {
    setAutoCheckUpdates(enabled);
    void persistPreferences({ auto_check_updates: enabled }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换自动检查更新设置失败',
        kind: 'error',
      });
    });
  };

  const handleFloatingWindowToggle = (enabled: boolean) => {
    setFloatingWindowEnabled(enabled);
    void persistPreferences({ floating_window_enabled: enabled }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换桌面悬浮窗状态失败',
        kind: 'error',
      });
    });
  };

  const updateDownloadPercent =
    updater.totalBytes && updater.totalBytes > 0
      ? Math.min(100, (updater.downloadedBytes / updater.totalBytes) * 100)
      : updater.status === 'installing'
        ? 100
        : 0;

  const previewSyncMutation = useMutation({
    mutationFn: () => ipc.previewSyncEnvelopeV1(0),
    onSuccess: (env) => {
      setSyncEnvelope(env);
    },
  });

  const saveSyncConfigMutation = useMutation({
    mutationFn: () =>
      ipc.updateSyncConfig(
        syncEnabledDraft,
        syncWorkerUrlDraft.trim() || null,
        syncE2eeDraft
      ),
    onSuccess: async (status: SyncStatusView) => {
      lastSyncCfgSnapshotRef.current = {
        enabled: status.enabled,
        worker_url: status.worker_url ?? '',
        e2ee_enabled: status.e2ee_enabled,
      };
      queryClient.setQueryData<SyncStatusView>(['sync-status'], status);
      await queryClient.invalidateQueries({ queryKey: ['sync-status'] });
      setNotice({ text: '云端同步配置已保存', kind: 'success' });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '保存云端同步配置失败',
        kind: 'error',
      });
    },
  });

  const saveSyncCredsMutation = useMutation({
    mutationFn: () => {
      const token = syncBearerTokenInput.trim() || null;
      const pass = syncPassphraseInput.trim() || null;
      if (!token && !pass) {
        return Promise.reject(new Error('请至少填写 SYNC Bearer Token 或 E2EE 同步口令'));
      }
      return ipc.setSyncCredentials(token, pass);
    },
    onSuccess: async (status: SyncStatusView) => {
      setSyncBearerTokenInput('');
      setSyncPassphraseInput('');
      queryClient.setQueryData<SyncStatusView>(['sync-status'], status);
      await queryClient.invalidateQueries({ queryKey: ['sync-status'] });
      setNotice({ text: '同步凭据已写入系统安全凭据库（未触碰 SQLite 明文）', kind: 'success' });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '保存同步凭据失败',
        kind: 'error',
      });
    },
  });

  const clearSyncCredsMutation = useMutation({
    mutationFn: () => ipc.clearSyncCredentials(),
    onSuccess: async (status: SyncStatusView) => {
      setSyncBearerTokenInput('');
      setSyncPassphraseInput('');
      queryClient.setQueryData<SyncStatusView>(['sync-status'], status);
      await queryClient.invalidateQueries({ queryKey: ['sync-status'] });
      setNotice({ text: '已从系统凭据库清除云端同步凭据', kind: 'success' });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '清除同步凭据失败',
        kind: 'error',
      });
    },
  });

  const triggerCloudSyncMutation = useMutation({
    mutationFn: () => ipc.triggerCloudSyncNow(),
    onSuccess: async (report) => {
      await queryClient.invalidateQueries({ queryKey: ['sync-status'] });
      await queryClient.invalidateQueries({ queryKey: ['devices'] });
      await queryClient.invalidateQueries({ queryKey: ['remote-subscriptions'] });
      await queryClient.invalidateQueries({ queryKey: ['overview'] });
      await queryClient.invalidateQueries({ queryKey: ['trends'] });
      await queryClient.invalidateQueries({ queryKey: ['monthly'] });
      await queryClient.invalidateQueries({ queryKey: ['models_pricing'] });
      await queryClient.invalidateQueries({ queryKey: ['speed_latency'] });
      await queryClient.invalidateQueries({ queryKey: ['tray_summary'] });
      setNotice({
        text: `云端同步完成：上传 ${report.pushed_hourly_deltas} 条小时增量 / ${report.pushed_subscriptions} 条订阅快照，拉取并合并 ${report.applied_hourly_buckets} 条远端小时聚合`,
        kind: 'success',
      });
    },
    onError: (err: unknown) => {
      void queryClient.invalidateQueries({ queryKey: ['sync-status'] });
      setNotice({
        text: err instanceof Error ? err.message : '云端同步失败',
        kind: 'error',
      });
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
          <div className="flex items-center gap-2">
            <h1 className="text-[16px] font-semibold tracking-tight">偏好设置与同步契约</h1>
            <span
              className="px-1.5 py-0.5 rounded text-[10.5px] font-mono tabular-nums"
              style={{
                backgroundColor: 'var(--bg-hover)',
                border: '1px solid var(--border-hairline)',
                color: 'var(--text-secondary)',
              }}
            >
              v0.2.2
            </span>
          </div>
          <p className="text-[12px]" style={{ color: 'var(--text-muted)' }}>
            配置全局币种、主题与外观、桌面悬浮窗与多端同步（SyncEnvelope v1）
          </p>
        </div>

        <button
          type="button"
          onClick={() => savePrefsMutation.mutate()}
          disabled={savePrefsMutation.isPending}
          className="btn-primary inline-flex items-center gap-1.5 px-3 py-1.5 rounded text-[12px] font-medium cursor-pointer disabled:opacity-50"
          style={{
            backgroundColor: 'var(--accent-primary)',
            color: '#ffffff',
          }}
        >
          <Save className="w-3.5 h-3.5" />
          <span>{savePrefsMutation.isPending ? '保存中…' : '保存设置'}</span>
        </button>
      </section>

      {notice && (
        <div
          className="px-3 py-2 text-[12px]"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            borderBottom: '1px solid var(--border-subtle)',
            color: notice.kind === 'error' ? 'var(--accent-rose)' : 'var(--accent-emerald)',
          }}
        >
          {notice.text}
        </div>
      )}

      {updater.availableUpdate &&
        (updater.dismissedVersion !== updater.availableUpdate.version ||
          updater.status === 'downloading' ||
          updater.status === 'installing') && (
          <div
            className="px-4 py-3 space-y-2"
            style={{
              backgroundColor: 'var(--bg-elevated)',
              borderTop: '1px solid var(--border-subtle)',
              borderBottom: '1px solid var(--border-subtle)',
            }}
          >
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="space-y-0.5 min-w-0">
                <div className="flex items-center gap-2 text-[12.5px] font-medium">
                  <span style={{ color: 'var(--accent-emerald)' }}>
                    发现新版本 v{updater.availableUpdate.version}
                  </span>
                  <span className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                    当前 v{updater.availableUpdate.currentVersion}
                    {updater.availableUpdate.date ? ` · ${updater.availableUpdate.date.slice(0, 10)}` : ''}
                  </span>
                </div>
                {updater.availableUpdate.body && (
                  <p
                    className="text-[11.5px] line-clamp-2"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    {updater.availableUpdate.body}
                  </p>
                )}
              </div>

              <div className="flex items-center gap-2 shrink-0">
                <button
                  type="button"
                  disabled={updater.status === 'downloading' || updater.status === 'installing'}
                  onClick={() => void downloadAndInstallUpdate()}
                  className="inline-flex items-center gap-1.5 px-3 py-1 rounded text-[11.5px] font-medium hover:opacity-90 cursor-pointer disabled:opacity-50"
                  style={{
                    backgroundColor: 'var(--accent-emerald)',
                    color: '#ffffff',
                  }}
                >
                  <Download className="w-3.5 h-3.5" />
                  <span>
                    {updater.status === 'downloading'
                      ? `下载中 ${updateDownloadPercent.toFixed(0)}%`
                      : updater.status === 'installing'
                        ? '正在安装并重启…'
                        : '立即下载并重启安装'}
                  </span>
                </button>
                {updater.status !== 'downloading' && updater.status !== 'installing' && (
                  <button
                    type="button"
                    onClick={() => updater.dismissBanner()}
                    className="px-2 py-1 rounded text-[11px] cursor-pointer"
                    style={{ color: 'var(--text-muted)' }}
                  >
                    稍后提醒
                  </button>
                )}
              </div>
            </div>

            {(updater.status === 'downloading' || updater.status === 'installing') && (
              <InlineProgressBar
                percent={updateDownloadPercent}
                accentColor="var(--accent-emerald)"
                heightPx={4}
              />
            )}
          </div>
        )}

      {/* Flat Hairline-Divided Preferences Form */}
      <section style={{ borderTop: '1px solid var(--border-subtle)' }}>
        <div
          className="py-4 grid grid-cols-12 gap-4 items-start"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4 pt-0.5">
            <div className="text-[13px] font-medium">显示币种、汇率与定价同步</div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              所有成本金额在核心层使用 micro-USD 定点数换算，支持免鉴权公开汇率与开源模型定价自动同步
            </div>
          </div>
          <div className="col-span-8 space-y-2.5">
            <div className="flex flex-wrap items-center gap-3">
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

              <div
                className="inline-flex items-center p-0.5 rounded"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                }}
              >
                <button
                  type="button"
                  data-active={exchangeRateAuto ? 'true' : undefined}
                  onClick={() => handleExchangeRateAutoToggle(true)}
                  className="seg-pill px-2.5 py-0.5 text-[11.5px] rounded-sm cursor-pointer"
                  style={{
                    color: exchangeRateAuto ? 'var(--text-primary)' : 'var(--text-secondary)',
                    fontWeight: exchangeRateAuto ? 500 : 400,
                  }}
                >
                  自动汇率
                </button>
                <button
                  type="button"
                  data-active={!exchangeRateAuto ? 'true' : undefined}
                  onClick={() => handleExchangeRateAutoToggle(false)}
                  className="seg-pill px-2.5 py-0.5 text-[11.5px] rounded-sm cursor-pointer"
                  style={{
                    color: !exchangeRateAuto ? 'var(--text-primary)' : 'var(--text-secondary)',
                    fontWeight: !exchangeRateAuto ? 500 : 400,
                  }}
                >
                  手动锁定
                </button>
              </div>

              <label className="flex items-center gap-2 text-[12px]">
                <span style={{ color: 'var(--text-secondary)' }}>1 USD =</span>
                <input
                  type="text"
                  aria-label="USD 对 CNY 汇率"
                  readOnly={exchangeRateAuto}
                  value={rateDraft}
                  onChange={(e) => setRateDraft(e.target.value)}
                  className="w-24 px-2 py-1 text-[12px] rounded outline-none tabular-nums"
                  style={{
                    backgroundColor: 'var(--bg-input)',
                    border: '1px solid var(--border-subtle)',
                    color: exchangeRateAuto ? 'var(--text-secondary)' : 'var(--text-primary)',
                  }}
                />
                <span style={{ color: 'var(--text-secondary)' }}>CNY</span>
              </label>

              <button
                type="button"
                disabled={syncNowMutation.isPending}
                onClick={() => syncNowMutation.mutate()}
                className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1 text-[11.5px] rounded cursor-pointer disabled:opacity-50"
                style={{
                  backgroundColor: 'var(--bg-elevated)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              >
                <RefreshCw
                  className={`w-3 h-3 ${syncNowMutation.isPending ? 'animate-spin' : ''}`}
                />
                <span>{syncNowMutation.isPending ? '刷新中…' : '刷新汇率与价格'}</span>
              </button>
            </div>

            <div className="flex flex-wrap items-center gap-4 text-[11.5px]">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={autoSyncPricing}
                  onChange={(e) => handleAutoSyncPricingToggle(e.target.checked)}
                />
                <span style={{ color: 'var(--text-secondary)' }}>
                  自动联网更新开源模型价格目录与汇率（启动及每 6 小时静默巡检）
                </span>
              </label>
              <span className="tabular-nums" style={{ color: 'var(--text-muted)' }}>
                {activePrefs?.last_pricing_sync_at_ms
                  ? `定价同步于 ${formatRelativeTime(activePrefs.last_pricing_sync_at_ms)}`
                  : '定价使用内置基线'}
                {' · '}
                {activePrefs?.last_rate_sync_at_ms
                  ? `汇率同步于 ${formatRelativeTime(activePrefs.last_rate_sync_at_ms)}`
                  : '汇率待首次同步'}
              </span>
            </div>
          </div>
        </div>

        <div
          className="py-4 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">界面主题</div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              即时生效并同步至主工作台、系统托盘与原生窗口边框
            </div>
          </div>
          <div className="col-span-8 flex flex-wrap items-center gap-2">
            {THEME_OPTIONS.map((opt) => {
              const active = theme === opt.id;
              return (
                <button
                  key={opt.id}
                  type="button"
                  data-active={active ? 'true' : undefined}
                  onClick={() => handleThemeSelect(opt.id)}
                  className="seg-pill px-3 py-1 rounded text-[12px] cursor-pointer"
                  style={{
                    backgroundColor: active ? 'var(--bg-hover)' : 'var(--bg-input)',
                    border: active
                      ? '1px solid var(--accent-primary)'
                      : '1px solid var(--border-subtle)',
                    color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                    fontWeight: active ? 500 : 400,
                  }}
                >
                  {opt.label}
                </button>
              );
            })}
          </div>
        </div>

        {/* Appearance & Custom Background System */}
        <div
          className="py-4 grid grid-cols-12 gap-4 items-start"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4 pt-0.5">
            <div className="text-[13px] font-medium">外观与自定义背景</div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              支持纯色工作台、内置克制微光渐变或本地壁纸，保持高密度数据锐利可读
            </div>
          </div>

          <div className="col-span-8 space-y-4">
            {/* Mode Segmented Selector */}
            <div
              className="inline-flex items-center p-0.5 rounded"
              style={{
                backgroundColor: 'var(--bg-input)',
                border: '1px solid var(--border-subtle)',
              }}
            >
              {BG_MODE_OPTIONS.map((modeOpt) => {
                const active = bgMode === modeOpt.id;
                return (
                  <button
                    key={modeOpt.id}
                    type="button"
                    data-active={active ? 'true' : undefined}
                    onClick={() => handleBgModeSelect(modeOpt.id)}
                    className="seg-pill px-3 py-1 text-[12px] rounded-sm cursor-pointer"
                    style={{
                      color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                      fontWeight: active ? 500 : 400,
                    }}
                  >
                    {modeOpt.label}
                  </button>
                );
              })}
            </div>

            {/* Gradient Presets */}
            {bgMode === 'gradient' && (
              <div className="space-y-2">
                <div
                  className="flex items-center gap-1.5 text-[11.5px]"
                  style={{ color: 'var(--text-secondary)' }}
                >
                  <Sparkles className="w-3.5 h-3.5" style={{ color: 'var(--accent-primary)' }} />
                  <span>内置微光渐变预设</span>
                </div>
                <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
                  {BACKGROUND_GRADIENT_PRESETS.map((preset) => {
                    const selected = bgPresetId === preset.id;
                    return (
                      <button
                        key={preset.id}
                        type="button"
                        data-selected={selected ? 'true' : undefined}
                        onClick={() => handlePresetSelect(preset.id)}
                        className="btn-interactive text-left p-2 rounded cursor-pointer space-y-1.5"
                        style={{
                          backgroundColor: selected ? 'var(--bg-hover)' : 'var(--bg-input)',
                          border: selected
                            ? '1px solid var(--accent-primary)'
                            : '1px solid var(--border-subtle)',
                        }}
                      >
                        <div
                          className="h-7 w-full rounded-sm overflow-hidden"
                          style={{
                            backgroundColor: 'var(--bg-canvas)',
                            border: '1px solid var(--border-hairline)',
                          }}
                        >
                          <div
                            className="w-full h-full"
                            style={{
                              backgroundImage: preset.gradients[resolvedTheme],
                              opacity: 0.72,
                            }}
                          />
                        </div>
                        <div>
                          <div
                            className="text-[12px] font-medium leading-tight"
                            style={{ color: 'var(--text-primary)' }}
                          >
                            {preset.label}
                          </div>
                          <div
                            className="text-[10.5px] leading-tight mt-0.5 truncate"
                            style={{ color: 'var(--text-muted)' }}
                          >
                            {preset.subtitle}
                          </div>
                        </div>
                      </button>
                    );
                  })}
                </div>
              </div>
            )}

            {/* Local Image Picker & Clear */}
            {bgMode === 'image' && (
              <div
                className="flex flex-wrap items-center justify-between gap-3 px-3 py-2.5 rounded"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                }}
              >
                <div className="flex items-center gap-2 min-w-0">
                  <ImageIcon
                    className="w-4 h-4 shrink-0"
                    style={{ color: 'var(--text-muted)' }}
                  />
                  <div className="min-w-0">
                    <div
                      className="text-[12px] truncate"
                      style={{ color: 'var(--text-primary)' }}
                    >
                      {bgImagePath ? `当前壁纸：${bgImagePath}` : '尚未选择本地壁纸'}
                    </div>
                    <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                      支持 PNG / JPG / WEBP / GIF / BMP（最大 15 MiB，自动复制至应用托管目录）
                    </div>
                  </div>
                </div>

                <div className="flex items-center gap-2 shrink-0">
                  <button
                    type="button"
                    onClick={() => void handlePickBackgroundImage()}
                    disabled={importBgMutation.isPending}
                    className="btn-interactive px-2.5 py-1 rounded text-[11.5px] font-medium cursor-pointer disabled:opacity-50"
                    style={{
                      backgroundColor: 'var(--bg-elevated)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  >
                    {importBgMutation.isPending
                      ? '导入中…'
                      : bgImagePath
                        ? '更换壁纸'
                        : '选择壁纸'}
                  </button>

                  {bgImagePath && (
                    <button
                      type="button"
                      onClick={() => clearBgMutation.mutate()}
                      disabled={clearBgMutation.isPending}
                      className="btn-interactive inline-flex items-center gap-1 px-2.5 py-1 rounded text-[11.5px] cursor-pointer disabled:opacity-50"
                      style={{
                        backgroundColor: 'var(--bg-elevated)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-secondary)',
                      }}
                    >
                      <RotateCcw className="w-3 h-3" />
                      <span>{clearBgMutation.isPending ? '清除中…' : '清除壁纸'}</span>
                    </button>
                  )}
                </div>
              </div>
            )}

            {/* Live Opacity & Blur Sliders */}
            {bgMode !== 'none' && (
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 pt-1">
                <label className="space-y-1.5 block">
                  <div className="flex items-center justify-between text-[11.5px]">
                    <span style={{ color: 'var(--text-secondary)' }}>背景可见度</span>
                    <span
                      className="font-mono tabular-nums"
                      style={{ color: 'var(--text-primary)' }}
                    >
                      {Math.round(bgOpacity * 100)}%
                    </span>
                  </div>
                  <input
                    type="range"
                    aria-label="背景可见度"
                    min={0.05}
                    max={0.6}
                    step={0.01}
                    value={bgOpacity}
                    onChange={(e) => {
                      const val = Number.parseFloat(e.target.value);
                      if (Number.isFinite(val)) {
                        handleSliderPreview({ bg_opacity: clampOpacity(val) });
                      }
                    }}
                    onPointerUp={flushSliderPersist}
                    onKeyUp={flushSliderPersist}
                    onBlur={flushSliderPersist}
                    className="w-full cursor-pointer accent-[var(--accent-primary)]"
                  />
                </label>

                <label className="space-y-1.5 block">
                  <div className="flex items-center justify-between text-[11.5px]">
                    <span style={{ color: 'var(--text-secondary)' }}>模糊度</span>
                    <span
                      className="font-mono tabular-nums"
                      style={{ color: 'var(--text-primary)' }}
                    >
                      {bgBlurPx}px
                    </span>
                  </div>
                  <input
                    type="range"
                    aria-label="模糊度"
                    min={0}
                    max={32}
                    step={1}
                    value={bgBlurPx}
                    onChange={(e) => {
                      const val = Number.parseInt(e.target.value, 10);
                      if (Number.isFinite(val)) {
                        handleSliderPreview({ bg_blur_px: clampBlurPx(val) });
                      }
                    }}
                    onPointerUp={flushSliderPersist}
                    onKeyUp={flushSliderPersist}
                    onBlur={flushSliderPersist}
                    className="w-full cursor-pointer accent-[var(--accent-primary)]"
                  />
                </label>
              </div>
            )}
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
                checked={floatingWindowEnabled}
                onChange={(e) => handleFloatingWindowToggle(e.target.checked)}
              />
              <span>显示桌面实时悬浮窗</span>
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
            <div className="text-[13px] font-medium">客户端版本与自动更新</div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              基于 GitHub Releases 无服务器静态分发与 Minisign 公钥验签，离线时静默跳过
            </div>
          </div>
          <div className="col-span-8 space-y-2 text-[12px]">
            <div className="flex flex-wrap items-center gap-3">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={autoCheckUpdates}
                  onChange={(e) => handleAutoCheckUpdatesToggle(e.target.checked)}
                />
                <span>启动时与每 12 小时自动检查客户端更新</span>
              </label>
              <button
                type="button"
                disabled={
                  updater.status === 'checking' ||
                  updater.status === 'downloading' ||
                  updater.status === 'installing'
                }
                onClick={() => void checkForUpdates(true)}
                className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1 text-[11.5px] rounded cursor-pointer disabled:opacity-50"
                style={{
                  backgroundColor: 'var(--bg-elevated)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              >
                <RefreshCw
                  className={`w-3 h-3 ${updater.status === 'checking' ? 'animate-spin' : ''}`}
                />
                <span>{updater.status === 'checking' ? '检查中…' : '检查更新'}</span>
              </button>
            </div>
            <div className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
              {updater.status === 'error' && updater.errorMessage
                ? `检查更新受限：${updater.errorMessage}`
                : updater.status === 'up_to_date'
                  ? '当前已是最新版本'
                  : updater.availableUpdate
                    ? `检测到新版本 v${updater.availableUpdate.version}`
                    : updater.lastCheckedAtMs
                      ? `上次检查于 ${formatRelativeTime(updater.lastCheckedAtMs)}`
                      : '尚未检查更新'}
            </div>
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

      {/* Cloud Multi-Device Sync (Cloudflare Worker + Optional E2EE) */}
      <section className="space-y-4 pb-6">
        <div
          className="flex flex-wrap items-center justify-between gap-3 pb-3"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="space-y-0.5">
            <div className="flex items-center gap-2">
              <Cloud className="w-4 h-4" style={{ color: 'var(--accent-sky)' }} />
              <h2 className="text-[13.5px] font-semibold">
                云端多端同步（Cloudflare Worker）
              </h2>
            </div>
            <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              自托管 Cloudflare Worker + Durable Object 实时 SSE 推拉；仅传输脱敏小时级统计与订阅快照，支持可选端到端加密（AES-256-GCM）
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-2 text-[11px] tabular-nums">
            <span
              className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded"
              style={{
                backgroundColor: 'var(--bg-elevated)',
                border: '1px solid var(--border-subtle)',
                color: syncStatus?.sse_connected ? 'var(--accent-emerald)' : 'var(--text-muted)',
              }}
            >
              <span
                className="inline-block w-1.5 h-1.5 rounded-full"
                style={{
                  backgroundColor: syncStatus?.sse_connected
                    ? 'var(--accent-emerald)'
                    : 'var(--text-muted)',
                }}
              />
              {syncStatus?.sse_connected ? 'SSE 实时已连接' : 'SSE 待机'}
            </span>
            <button
              type="button"
              onClick={() => triggerCloudSyncMutation.mutate()}
              disabled={triggerCloudSyncMutation.isPending}
              className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1 rounded text-[12px] font-medium cursor-pointer disabled:opacity-50"
              style={{
                backgroundColor: 'var(--bg-elevated)',
                color: 'var(--text-primary)',
                border: '1px solid var(--border-strong)',
              }}
            >
              <RefreshCw
                className={`w-3.5 h-3.5 ${triggerCloudSyncMutation.isPending ? 'animate-spin' : ''}`}
              />
              <span>{triggerCloudSyncMutation.isPending ? '同步中…' : '立即同步'}</span>
            </button>
          </div>
        </div>

        {/* Row 1: Worker Endpoint & E2EE Toggle */}
        <div
          className="pb-4 grid grid-cols-12 gap-4 items-start"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <div className="col-span-4 space-y-0.5">
            <div className="text-[13px] font-medium">Worker 网关与同步开关</div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              填写您部署的 `services/sync-worker` 域名，开启后本地写入 3s 防抖自动推送
            </div>
          </div>
          <div className="col-span-8 space-y-2.5 text-[12px]">
            <div className="flex flex-wrap items-center gap-4">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={syncEnabledDraft}
                  onChange={(e) => setSyncEnabledDraft(e.target.checked)}
                />
                <span>启用 Cloudflare Worker 云端同步</span>
              </label>
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={syncE2eeDraft}
                  onChange={(e) => setSyncE2eeDraft(e.target.checked)}
                />
                <span className="inline-flex items-center gap-1">
                  <ShieldCheck className="w-3.5 h-3.5" style={{ color: 'var(--accent-emerald)' }} />
                  <span>启用端到端加密 (AES-256-GCM)</span>
                </span>
              </label>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <input
                type="url"
                aria-label="Worker 服务地址"
                placeholder="https://tokentracker-sync.your-subdomain.workers.dev"
                value={syncWorkerUrlDraft}
                onChange={(e) => setSyncWorkerUrlDraft(e.target.value)}
                className="flex-1 min-w-[260px] px-2.5 py-1.5 text-[12px] font-mono rounded outline-none"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
              <button
                type="button"
                onClick={() => saveSyncConfigMutation.mutate()}
                disabled={saveSyncConfigMutation.isPending}
                className="btn-interactive inline-flex items-center gap-1.5 px-3 py-1.5 rounded text-[12px] font-medium cursor-pointer disabled:opacity-50"
                style={{
                  backgroundColor: 'var(--bg-elevated)',
                  color: 'var(--text-primary)',
                  border: '1px solid var(--border-strong)',
                }}
              >
                <Save className="w-3.5 h-3.5" />
                <span>{saveSyncConfigMutation.isPending ? '保存中…' : '保存同步配置'}</span>
              </button>
            </div>
            {syncStatus?.e2ee_kdf_salt_hex && (
              <div className="text-[11px] font-mono tabular-nums" style={{ color: 'var(--text-muted)' }}>
                KDF Salt (PBKDF2 210k): <code>{syncStatus.e2ee_kdf_salt_hex}</code>
              </div>
            )}
          </div>
        </div>

        {/* Row 2: OS Keyring Sync Credentials */}
        <div
          className="pb-4 grid grid-cols-12 gap-4 items-start"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <div className="col-span-4 space-y-0.5">
            <div className="flex items-center gap-1.5 text-[13px] font-medium">
              <KeyRound className="w-3.5 h-3.5" style={{ color: 'var(--text-secondary)' }} />
              <span>系统凭据库密钥（Zero SQLite）</span>
            </div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              Bearer Token 与 E2EE 同步口令独占存储于系统凭据管理器，绝不写入本地 SQLite 或上传明文
            </div>
          </div>
          <div className="col-span-8 space-y-2.5 text-[12px]">
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
              <div className="space-y-1">
                <div className="flex items-center justify-between text-[11px]">
                  <span style={{ color: 'var(--text-secondary)' }}>Worker SYNC_TOKEN</span>
                  <span
                    style={{
                      color: syncStatus?.has_bearer_token
                        ? 'var(--accent-emerald)'
                        : 'var(--text-muted)',
                    }}
                  >
                    {syncStatus?.has_bearer_token ? 'Bearer 已配置' : 'Bearer 未配置'}
                  </span>
                </div>
                <input
                  type="password"
                  aria-label="Sync Bearer Token"
                  placeholder={syncStatus?.has_bearer_token ? '已保存在系统凭据库，输入新值可覆盖' : '输入 Cloudflare SYNC_TOKEN'}
                  value={syncBearerTokenInput}
                  onChange={(e) => setSyncBearerTokenInput(e.target.value)}
                  className="w-full px-2.5 py-1.5 text-[12px] font-mono rounded outline-none"
                  style={{
                    backgroundColor: 'var(--bg-input)',
                    border: '1px solid var(--border-subtle)',
                    color: 'var(--text-primary)',
                  }}
                />
              </div>

              <div className="space-y-1">
                <div className="flex items-center justify-between text-[11px]">
                  <span style={{ color: 'var(--text-secondary)' }}>E2EE 同步口令（多设备一致）</span>
                  <span
                    style={{
                      color: syncStatus?.has_e2ee_passphrase
                        ? 'var(--accent-emerald)'
                        : 'var(--text-muted)',
                    }}
                  >
                    {syncStatus?.has_e2ee_passphrase ? 'E2EE 口令已配置' : 'E2EE 口令未配置'}
                  </span>
                </div>
                <input
                  type="password"
                  aria-label="E2EE 同步口令"
                  placeholder={syncStatus?.has_e2ee_passphrase ? '已保存在系统凭据库，输入新值可覆盖' : '开启 E2EE 时填写共享同步口令'}
                  value={syncPassphraseInput}
                  onChange={(e) => setSyncPassphraseInput(e.target.value)}
                  className="w-full px-2.5 py-1.5 text-[12px] font-mono rounded outline-none"
                  style={{
                    backgroundColor: 'var(--bg-input)',
                    border: '1px solid var(--border-subtle)',
                    color: 'var(--text-primary)',
                  }}
                />
              </div>
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={() => saveSyncCredsMutation.mutate()}
                disabled={saveSyncCredsMutation.isPending}
                className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1 rounded text-[11.5px] font-medium cursor-pointer disabled:opacity-50"
                style={{
                  backgroundColor: 'var(--bg-elevated)',
                  border: '1px solid var(--border-strong)',
                  color: 'var(--text-primary)',
                }}
              >
                <KeyRound className="w-3 h-3" />
                <span>保存同步凭据</span>
              </button>
              <button
                type="button"
                onClick={() => clearSyncCredsMutation.mutate()}
                disabled={clearSyncCredsMutation.isPending}
                className="btn-interactive px-2.5 py-1 rounded text-[11.5px] cursor-pointer disabled:opacity-50"
                style={{
                  backgroundColor: 'var(--bg-elevated)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--accent-rose)',
                }}
              >
                清除凭据
              </button>
            </div>
          </div>
        </div>

        {/* Row 3: Watermarks, Diagnostics & Sanitized Payload Preview */}
        <div className="space-y-2.5">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div
              className="flex flex-wrap items-center gap-4 text-[11.5px] tabular-nums"
              style={{ color: 'var(--text-muted)' }}
            >
              <span>Push 水位线: {syncStatus?.last_push_watermark_ms ?? 0}</span>
              <span>Pull 水位线: {syncStatus?.last_pull_watermark_ms ?? 0}</span>
              <span>
                最近同步:{' '}
                {syncStatus?.last_sync_at_ms
                  ? formatRelativeTime(syncStatus.last_sync_at_ms)
                  : '尚未同步'}
              </span>
            </div>

            <button
              type="button"
              onClick={() => previewSyncMutation.mutate()}
              disabled={previewSyncMutation.isPending}
              className="btn-interactive px-3 py-1.5 rounded text-[12px] cursor-pointer disabled:opacity-50"
              style={{
                backgroundColor: 'var(--bg-elevated)',
                color: 'var(--text-primary)',
                border: '1px solid var(--border-subtle)',
              }}
            >
              {previewSyncMutation.isPending ? '生成中…' : '生成脱敏同步负载预览'}
            </button>
          </div>

          {syncStatus?.last_sync_error && (
            <div
              className="px-3 py-2 rounded text-[11.5px]"
              style={{
                backgroundColor: 'rgba(244, 63, 94, 0.08)',
                border: '1px solid rgba(244, 63, 94, 0.35)',
                color: 'var(--accent-rose)',
              }}
            >
              最近同步异常：{syncStatus.last_sync_error}
            </div>
          )}
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
