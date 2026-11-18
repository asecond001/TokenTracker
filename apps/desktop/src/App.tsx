import { Suspense, lazy, useCallback, useEffect, useRef, useState } from 'react';
import { type QueryClient, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { listen } from '@tauri-apps/api/event';
import { Sidebar } from './components/Sidebar';
import { TitleBar } from './components/TitleBar';
import { WorkspaceBackdrop } from './components/WorkspaceBackdrop';
import { ipc, isTauriRuntime } from './lib/ipc';
import { CLOCK_REFRESH_MS, createUsageRefresher } from './lib/liveRefresh';
import { checkForUpdates } from './lib/updater';

// Secondary pages and the auxiliary tray / floating windows are split into
// their own chunks so each webview only downloads the code it renders.
const OverviewPage = lazy(() =>
  import('./pages/OverviewPage').then((m) => ({ default: m.OverviewPage }))
);
const TrendsPage = lazy(() => import('./pages/TrendsPage').then((m) => ({ default: m.TrendsPage })));
const MonthlyPage = lazy(() =>
  import('./pages/MonthlyPage').then((m) => ({ default: m.MonthlyPage }))
);
const ModelsComparePage = lazy(() =>
  import('./pages/ModelsComparePage').then((m) => ({ default: m.ModelsComparePage }))
);
const SubscriptionsPage = lazy(() =>
  import('./pages/SubscriptionsPage').then((m) => ({ default: m.SubscriptionsPage }))
);
const DataSourcesPage = lazy(() =>
  import('./pages/DataSourcesPage').then((m) => ({ default: m.DataSourcesPage }))
);
const PreferencesPage = lazy(() =>
  import('./pages/PreferencesPage').then((m) => ({ default: m.PreferencesPage }))
);
const TrayPopupPage = lazy(() =>
  import('./pages/TrayPopupPage').then((m) => ({ default: m.TrayPopupPage }))
);
const FloatingMonitorPage = lazy(() =>
  import('./pages/FloatingMonitorPage').then((m) => ({ default: m.FloatingMonitorPage }))
);
import {
  type NavPage,
  syncDocumentAndNativeTheme,
  useWorkspaceStore,
} from './stores/workspaceStore';
import type {
  AppPreferences,
  BootstrapState,
  CurrencyCode,
  ThemeMode,
  TraySummary,
  WorkspaceNavigatePayload,
} from './types/ipc';

const VALID_NAV_PAGES: ReadonlyArray<NavPage> = [
  'overview',
  'trends',
  'monthly',
  'models',
  'subscriptions',
  'sources',
  'preferences',
];

function normalizeThemeMode(raw: string | undefined | null): ThemeMode {
  return raw === 'light' || raw === 'dark' || raw === 'system' ? raw : 'dark';
}

function updateTraySummaryCurrencyCache(
  queryClient: QueryClient,
  currency: CurrencyCode,
  usdToCnyRate: string
): void {
  const patchTray = (prev: TraySummary | undefined): TraySummary | undefined =>
    prev
      ? {
          ...prev,
          currency,
          usd_to_cny_rate: usdToCnyRate,
        }
      : prev;

  queryClient.setQueriesData<TraySummary>({ queryKey: ['tray_summary'] }, patchTray);
  queryClient.setQueriesData<TraySummary>({ queryKey: ['tray-summary'] }, patchTray);
}

function applyPreferencesToStoreAndCaches(
  queryClient: QueryClient,
  prefs: AppPreferences
): void {
  const { setCurrency, setUsdToCnyRate, setTheme } = useWorkspaceStore.getState();
  const normalizedTheme = normalizeThemeMode(prefs.theme);

  setCurrency(prefs.currency);
  setUsdToCnyRate(prefs.usd_to_cny_rate);
  setTheme(normalizedTheme);
  syncDocumentAndNativeTheme(normalizedTheme);

  queryClient.setQueryData<AppPreferences>(['preferences'], prefs);
  queryClient.setQueryData<BootstrapState>(['bootstrap'], (prev) =>
    prev ? { ...prev, preferences: prefs } : prev
  );
  updateTraySummaryCurrencyCache(queryClient, prefs.currency, prefs.usd_to_cny_rate);
}

export function App() {
  const viewParam =
    typeof window !== 'undefined' ? new URLSearchParams(window.location.search).get('view') : null;
  const isTrayView = viewParam === 'tray';
  const isFloatingView = viewParam === 'floating';
  const isAuxView = isTrayView || isFloatingView;

  const queryClient = useQueryClient();
  const {
    activePage,
    theme,
    tzOffsetMinutes,
    setActivePage,
    setProviderFilter,
    setCurrency,
    setUsdToCnyRate,
    setTheme,
  } = useWorkspaceStore();

  const hydratedPrefsRef = useRef(false);
  const inFlightPrefsWritesRef = useRef(0);
  const latestPrefsWriteIdRef = useRef(0);
  const [livePulseSeq, setLivePulseSeq] = useState(0);

  const { data: bootstrap } = useQuery({
    queryKey: ['bootstrap'],
    queryFn: () => ipc.getBootstrapState(),
    enabled: !isAuxView,
    refetchInterval: CLOCK_REFRESH_MS,
  });

  const { data: preferences } = useQuery({
    queryKey: ['preferences'],
    queryFn: () => ipc.getPreferences(),
    enabled: !isAuxView,
  });

  const { data: traySummary } = useQuery({
    queryKey: ['tray_summary', tzOffsetMinutes],
    queryFn: () => ipc.getTraySummary(tzOffsetMinutes),
    enabled: !isAuxView,
    refetchInterval: CLOCK_REFRESH_MS,
  });

  // Sync persisted preferences once on bootstrap
  useEffect(() => {
    if (!bootstrap?.preferences || hydratedPrefsRef.current) return;
    hydratedPrefsRef.current = true;
    const p = bootstrap.preferences;
    setCurrency(p.currency);
    setUsdToCnyRate(p.usd_to_cny_rate);
    const savedTheme = normalizeThemeMode(p.theme);
    setTheme(savedTheme);
    syncDocumentAndNativeTheme(savedTheme);
    queryClient.setQueryData<AppPreferences>(['preferences'], (prev) => prev ?? p);
  }, [bootstrap, queryClient, setCurrency, setUsdToCnyRate, setTheme]);

  // Keep resolvedTheme + DOM + native window theme synced with OS preference changes when theme === 'system'
  useEffect(() => {
    syncDocumentAndNativeTheme(theme);
    if (
      theme !== 'system' ||
      typeof window === 'undefined' ||
      typeof window.matchMedia !== 'function'
    ) {
      return;
    }
    const mq = window.matchMedia('(prefers-color-scheme: light)');
    const onChange = () => {
      syncDocumentAndNativeTheme('system');
    };
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, [theme]);

  // Subscribe to backend live ingestion and preference sync events in BOTH main and tray views
  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const usageRefresher = createUsageRefresher(queryClient);
    unlisteners.push(usageRefresher.dispose);

    void listen('usage-updated', () => {
      setLivePulseSeq((seq) => seq + 1);
      usageRefresher.schedule();
    })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          unlisteners.push(unlisten);
        }
      })
      .catch(() => {});

    void listen<AppPreferences>('preferences-updated', (event) => {
      const incoming = event.payload;
      if (!incoming) return;
      if (inFlightPrefsWritesRef.current > 0) {
        return;
      }
      hydratedPrefsRef.current = true;
      applyPreferencesToStoreAndCaches(queryClient, incoming);
    })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          unlisteners.push(unlisten);
        }
      })
      .catch(() => {});

    void listen<WorkspaceNavigatePayload>('navigate-workspace', (event) => {
      const payload = event.payload;
      if (!payload) return;
      // `speed` was merged into the 模型对比 page in v0.4.6.
      const requestedPage = payload.page === 'speed' ? 'models' : payload.page;
      if (requestedPage && VALID_NAV_PAGES.includes(requestedPage as NavPage)) {
        setActivePage(requestedPage as NavPage);
      }
      if (payload.provider_id !== undefined) {
        setProviderFilter(payload.provider_id || 'all');
      }
    })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          unlisteners.push(unlisten);
        }
      })
      .catch(() => {});

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, [queryClient, setActivePage, setProviderFilter]);

  const autoCheckUpdates =
    (preferences ?? bootstrap?.preferences)?.auto_check_updates ?? true;

  // Periodic quiet background check for signed desktop releases (offline-safe)
  useEffect(() => {
    if (isTrayView || !autoCheckUpdates || !isTauriRuntime()) {
      return;
    }
    const startupTimer = window.setTimeout(() => {
      void checkForUpdates(false);
    }, 10_000);
    const periodicTimer = window.setInterval(() => {
      void checkForUpdates(false);
    }, 12 * 60 * 60 * 1000);

    return () => {
      window.clearTimeout(startupTimer);
      window.clearInterval(periodicTimer);
    };
  }, [isTrayView, autoCheckUpdates]);

  const applyPreferencesOptimistically = useCallback(
    (patch: Partial<AppPreferences>): AppPreferences | null => {
      hydratedPrefsRef.current = true;
      latestPrefsWriteIdRef.current += 1;

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

      void queryClient.cancelQueries({ queryKey: ['preferences'] });
      void queryClient.cancelQueries({ queryKey: ['bootstrap'] });

      const currentPrefs =
        queryClient.getQueryData<AppPreferences>(['preferences']) ??
        queryClient.getQueryData<BootstrapState>(['bootstrap'])?.preferences;

      if (!currentPrefs) {
        return null;
      }

      const nextPrefs: AppPreferences = {
        ...currentPrefs,
        ...patch,
        theme: patch.theme !== undefined ? normalizeThemeMode(patch.theme) : currentPrefs.theme,
      };

      queryClient.setQueryData<AppPreferences>(['preferences'], nextPrefs);
      queryClient.setQueryData<BootstrapState>(['bootstrap'], (prev) =>
        prev ? { ...prev, preferences: nextPrefs } : prev
      );

      if (patch.currency !== undefined || patch.usd_to_cny_rate !== undefined) {
        updateTraySummaryCurrencyCache(queryClient, nextPrefs.currency, nextPrefs.usd_to_cny_rate);
      }

      return nextPrefs;
    },
    [queryClient, setCurrency, setTheme, setUsdToCnyRate]
  );

  const applyAndPersistPreferences = useCallback(
    async (patch: Partial<AppPreferences>): Promise<AppPreferences> => {
      let nextPrefs = applyPreferencesOptimistically(patch);
      if (!nextPrefs) {
        const fetched = await ipc.getPreferences();
        nextPrefs = {
          ...fetched,
          ...patch,
          theme: patch.theme !== undefined ? normalizeThemeMode(patch.theme) : fetched.theme,
        };
        applyPreferencesToStoreAndCaches(queryClient, nextPrefs);
        latestPrefsWriteIdRef.current += 1;
      }

      const writeId = latestPrefsWriteIdRef.current;
      inFlightPrefsWritesRef.current += 1;
      try {
        const saved = await ipc.updatePreferences(nextPrefs);
        if (writeId === latestPrefsWriteIdRef.current) {
          applyPreferencesToStoreAndCaches(queryClient, saved);
        }
        return saved;
      } finally {
        inFlightPrefsWritesRef.current = Math.max(0, inFlightPrefsWritesRef.current - 1);
      }
    },
    [applyPreferencesOptimistically, queryClient]
  );

  const applyAndPersistTheme = useCallback(
    (nextTheme: ThemeMode) => {
      void applyAndPersistPreferences({ theme: nextTheme });
    },
    [applyAndPersistPreferences]
  );

  const applyAndPersistCurrency = useCallback(
    (nextCurrency: CurrencyCode) => {
      void applyAndPersistPreferences({ currency: nextCurrency });
    },
    [applyAndPersistPreferences]
  );

  const scanMutation = useMutation({
    mutationFn: () => ipc.triggerScanNow(),
    onSuccess: async () => {
      setLivePulseSeq((seq) => seq + 1);
      await queryClient.invalidateQueries({
        predicate: (query) => query.queryKey[0] !== 'bg-image-data-url',
      });
    },
  });

  if (isTrayView) {
    return (
      <Suspense fallback={null}>
        <TrayPopupPage />
      </Suspense>
    );
  }

  if (isFloatingView) {
    return (
      <Suspense fallback={null}>
        <FloatingMonitorPage livePulseSeq={livePulseSeq} />
      </Suspense>
    );
  }

  const activePreferences = preferences ?? bootstrap?.preferences ?? null;
  const hasBackdrop = Boolean(
    activePreferences &&
      (activePreferences.bg_mode === 'gradient' ||
        (activePreferences.bg_mode === 'image' && activePreferences.bg_image_path))
  );

  const renderPage = () => {
    switch (activePage) {
      case 'overview':
        return <OverviewPage />;
      case 'trends':
        return <TrendsPage />;
      case 'monthly':
        return <MonthlyPage />;
      case 'models':
        return <ModelsComparePage />;
      case 'subscriptions':
        return <SubscriptionsPage />;
      case 'sources':
        return <DataSourcesPage />;
      case 'preferences':
        return (
          <PreferencesPage
            onApplyTheme={applyAndPersistTheme}
            onPreviewPreferences={applyPreferencesOptimistically}
            onPersistPreferences={applyAndPersistPreferences}
          />
        );
    }
  };

  return (
    <div
      className="relative h-screen w-screen flex flex-col overflow-hidden"
      style={{
        backgroundColor: 'var(--bg-canvas)',
        color: 'var(--text-primary)',
      }}
    >
      {/* Subtle Material You ambient tonal mesh for Liquid Glass chrome depth */}
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 z-0"
        style={{ background: 'var(--ambient-mesh)' }}
      />
      <WorkspaceBackdrop preferences={activePreferences} />

      <div className="relative z-10 flex flex-col flex-1 min-h-0">
        <TitleBar
          collectorWatching={bootstrap?.collector_watching ?? false}
          isScanning={scanMutation.isPending}
          livePulseSeq={livePulseSeq}
          hasBackdrop={hasBackdrop}
          floatingEnabled={activePreferences?.floating_window_enabled ?? false}
          onScanNow={() => scanMutation.mutate()}
          onPersistCurrency={applyAndPersistCurrency}
          onPersistTheme={applyAndPersistTheme}
        />

        <div className="flex flex-1 min-h-0">
          <Sidebar traySummary={traySummary} hasBackdrop={hasBackdrop} />
          <main className="flex-1 h-full overflow-y-auto pl-6 pr-8 py-5 [scrollbar-gutter:stable]">
            <div key={activePage} className="page-transition-enter min-h-full">
              <Suspense fallback={null}>{renderPage()}</Suspense>
            </div>
          </main>
        </div>
      </div>
    </div>
  );
}

export default App;
