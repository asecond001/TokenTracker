import { useEffect, useRef } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { listen } from '@tauri-apps/api/event';
import { Sidebar } from './components/Sidebar';
import { TitleBar } from './components/TitleBar';
import { ipc, isTauriRuntime } from './lib/ipc';
import { DataSourcesPage } from './pages/DataSourcesPage';
import { ModelsPricingPage } from './pages/ModelsPricingPage';
import { MonthlyPage } from './pages/MonthlyPage';
import { OverviewPage } from './pages/OverviewPage';
import { PreferencesPage } from './pages/PreferencesPage';
import { SpeedLatencyPage } from './pages/SpeedLatencyPage';
import { SubscriptionsPage } from './pages/SubscriptionsPage';
import { TrayPopupPage } from './pages/TrayPopupPage';
import { TrendsPage } from './pages/TrendsPage';
import { useWorkspaceStore } from './stores/workspaceStore';
import type { CurrencyCode, ThemeMode } from './types/ipc';

function applyResolvedTheme(mode: ThemeMode) {
  if (typeof document === 'undefined') return;
  let resolved: 'dark' | 'light' = 'dark';
  if (mode === 'light') {
    resolved = 'light';
  } else if (mode === 'system' && typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
    resolved = window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  }
  document.documentElement.dataset.theme = resolved;
}

export function App() {
  const isTrayView =
    typeof window !== 'undefined' &&
    new URLSearchParams(window.location.search).get('view') === 'tray';

  const queryClient = useQueryClient();
  const {
    activePage,
    theme,
    tzOffsetMinutes,
    setCurrency,
    setUsdToCnyRate,
    setTheme,
  } = useWorkspaceStore();

  const hydratedPrefsRef = useRef(false);

  const { data: bootstrap } = useQuery({
    queryKey: ['bootstrap'],
    queryFn: () => ipc.getBootstrapState(),
    enabled: !isTrayView,
    refetchInterval: 20_000,
  });

  const { data: traySummary } = useQuery({
    queryKey: ['tray_summary', tzOffsetMinutes],
    queryFn: () => ipc.getTraySummary(tzOffsetMinutes),
    enabled: !isTrayView,
    refetchInterval: 10_000,
  });

  // Sync persisted preferences once on bootstrap
  useEffect(() => {
    if (!bootstrap?.preferences || hydratedPrefsRef.current) return;
    hydratedPrefsRef.current = true;
    const p = bootstrap.preferences;
    setCurrency(p.currency);
    setUsdToCnyRate(p.usd_to_cny_rate);
    const savedTheme: ThemeMode =
      p.theme === 'light' || p.theme === 'dark' || p.theme === 'system'
        ? p.theme
        : 'dark';
    setTheme(savedTheme);
  }, [bootstrap, setCurrency, setUsdToCnyRate, setTheme]);

  // Apply light/dark/system theme attribute to <html>
  useEffect(() => {
    applyResolvedTheme(theme);
    if (theme !== 'system' || typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
      return;
    }
    const mq = window.matchMedia('(prefers-color-scheme: light)');
    const onChange = () => applyResolvedTheme('system');
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, [theme]);

  // Subscribe to backend live ingestion event when inside Tauri runtime
  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    let unlistenFn: (() => void) | null = null;
    void listen('usage-updated', () => {
      void queryClient.invalidateQueries();
    })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          unlistenFn = unlisten;
        }
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlistenFn?.();
    };
  }, [queryClient]);

  const scanMutation = useMutation({
    mutationFn: () => ipc.triggerScanNow(),
    onSuccess: async () => {
      await queryClient.invalidateQueries();
    },
  });

  const persistCurrency = (nextCurrency: CurrencyCode) => {
    if (!bootstrap?.preferences) return;
    void ipc.updatePreferences({
      ...bootstrap.preferences,
      currency: nextCurrency,
    });
  };

  const persistTheme = (nextTheme: ThemeMode) => {
    if (!bootstrap?.preferences) return;
    void ipc.updatePreferences({
      ...bootstrap.preferences,
      theme: nextTheme,
    });
  };

  if (isTrayView) {
    return <TrayPopupPage />;
  }

  const renderPage = () => {
    switch (activePage) {
      case 'overview':
        return <OverviewPage />;
      case 'trends':
        return <TrendsPage />;
      case 'monthly':
        return <MonthlyPage />;
      case 'models':
        return <ModelsPricingPage />;
      case 'subscriptions':
        return <SubscriptionsPage />;
      case 'speed':
        return <SpeedLatencyPage />;
      case 'sources':
        return <DataSourcesPage />;
      case 'preferences':
        return <PreferencesPage />;
    }
  };

  return (
    <div
      className="h-screen w-screen flex flex-col overflow-hidden"
      style={{
        backgroundColor: 'var(--bg-canvas)',
        color: 'var(--text-primary)',
      }}
    >
      <TitleBar
        collectorWatching={bootstrap?.collector_watching ?? false}
        isScanning={scanMutation.isPending}
        onScanNow={() => scanMutation.mutate()}
        onPersistCurrency={persistCurrency}
        onPersistTheme={persistTheme}
      />

      <div className="flex flex-1 min-h-0">
        <Sidebar traySummary={traySummary} />
        <main className="flex-1 h-full overflow-y-auto px-6 py-5">{renderPage()}</main>
      </div>
    </div>
  );
}

export default App;
