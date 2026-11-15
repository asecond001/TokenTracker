import { getCurrentWindow } from '@tauri-apps/api/window';
import { create } from 'zustand';
import { isTauriRuntime } from '../lib/ipc';
import type { AnalyticsFilter, CurrencyCode, ThemeMode } from '../types/ipc';

export type NavPage =
  | 'overview'
  | 'trends'
  | 'monthly'
  | 'models'
  | 'subscriptions'
  | 'speed'
  | 'sources'
  | 'preferences';

export type TimePreset = 'today' | '7d' | '30d' | 'month' | 'all' | 'custom';
export type ResolvedTheme = 'dark' | 'light';

export interface CustomTimeRange {
  start?: string;
  end?: string;
}

export interface ComputedTimeRange {
  start_ms: number;
  end_ms: number;
  all_time: boolean;
}

export function resolveThemeMode(mode: ThemeMode | string): ResolvedTheme {
  if (mode === 'light') {
    return 'light';
  }
  if (
    mode === 'system' &&
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function'
  ) {
    return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  }
  return 'dark';
}

function applyDomAndNativeTheme(mode: ThemeMode, resolved: ResolvedTheme): void {
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.theme = resolved;
  }
  if (isTauriRuntime()) {
    try {
      const nativeTheme = mode === 'system' ? null : mode;
      void Promise.resolve(getCurrentWindow().setTheme(nativeTheme)).catch(() => {});
    } catch {
      // Ignore if Tauri native window handle is unavailable (e.g. unit tests)
    }
  }
}

export function syncDocumentAndNativeTheme(mode: ThemeMode): ResolvedTheme {
  const resolved = resolveThemeMode(mode);
  applyDomAndNativeTheme(mode, resolved);
  if (useWorkspaceStore.getState().resolvedTheme !== resolved) {
    useWorkspaceStore.setState({ resolvedTheme: resolved });
  }
  return resolved;
}

export function computeTimeRange(
  preset: TimePreset,
  customRange?: CustomTimeRange
): ComputedTimeRange {
  const now = new Date();
  const endMs = now.getTime();

  switch (preset) {
    case 'today': {
      const d = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 0, 0, 0, 0);
      return { start_ms: d.getTime(), end_ms: endMs, all_time: false };
    }
    case '7d':
      return { start_ms: endMs - 7 * 86_400_000, end_ms: endMs, all_time: false };
    case '30d':
      return { start_ms: endMs - 30 * 86_400_000, end_ms: endMs, all_time: false };
    case 'month': {
      const d = new Date(now.getFullYear(), now.getMonth(), 1, 0, 0, 0, 0);
      return { start_ms: d.getTime(), end_ms: endMs, all_time: false };
    }
    case 'all':
      return { start_ms: 0, end_ms: endMs, all_time: true };
    case 'custom': {
      const s = customRange?.start ? new Date(`${customRange.start}T00:00:00`) : new Date(NaN);
      const e = customRange?.end ? new Date(`${customRange.end}T23:59:59.999`) : new Date(NaN);
      const startMs = Number.isNaN(s.getTime()) ? endMs - 7 * 86_400_000 : s.getTime();
      const resolvedEndMs = Number.isNaN(e.getTime()) ? endMs : e.getTime();
      return { start_ms: startMs, end_ms: resolvedEndMs, all_time: false };
    }
  }
}

interface WorkspaceState {
  activePage: NavPage;
  timePreset: TimePreset;
  customStartDate: string; // YYYY-MM-DD
  customEndDate: string; // YYYY-MM-DD
  providerFilter: string; // 'all' | 'claude_code' | 'codex' | 'antigravity' | ...
  deviceFilter: string; // 'all' | device_id
  currency: CurrencyCode;
  usdToCnyRate: string;
  theme: ThemeMode;
  resolvedTheme: ResolvedTheme;
  tzOffsetMinutes: number;

  setActivePage: (page: NavPage) => void;
  setTimePreset: (preset: TimePreset) => void;
  setCustomDateRange: (start: string, end: string) => void;
  setProviderFilter: (provider: string) => void;
  setDeviceFilter: (deviceId: string) => void;
  setCurrency: (currency: CurrencyCode) => void;
  setUsdToCnyRate: (rate: string) => void;
  setTheme: (theme: ThemeMode) => void;
  getFilter: () => AnalyticsFilter;
}

function toDateInputString(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

const nowDate = new Date();
const weekAgo = new Date(nowDate.getTime() - 6 * 86_400_000);

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  activePage: 'overview',
  timePreset: '7d',
  customStartDate: toDateInputString(weekAgo),
  customEndDate: toDateInputString(nowDate),
  providerFilter: 'all',
  deviceFilter: 'all',
  currency: 'USD',
  usdToCnyRate: '7.25',
  theme: 'dark',
  resolvedTheme: 'dark',
  tzOffsetMinutes: new Date().getTimezoneOffset() * -1,

  setActivePage: (activePage) => set({ activePage }),
  setTimePreset: (timePreset) => set({ timePreset }),
  setCustomDateRange: (customStartDate, customEndDate) =>
    set({ customStartDate, customEndDate, timePreset: 'custom' }),
  setProviderFilter: (providerFilter) => set({ providerFilter }),
  setDeviceFilter: (deviceFilter) => set({ deviceFilter }),
  setCurrency: (currency) => set({ currency }),
  setUsdToCnyRate: (usdToCnyRate) => set({ usdToCnyRate }),
  setTheme: (theme) => {
    const resolvedTheme = resolveThemeMode(theme);
    applyDomAndNativeTheme(theme, resolvedTheme);
    set({ theme, resolvedTheme });
  },

  getFilter: () => {
    const state = get();
    const range = computeTimeRange(state.timePreset, {
      start: state.customStartDate,
      end: state.customEndDate,
    });

    return {
      start_ms: range.start_ms,
      end_ms: range.end_ms,
      provider_id: state.providerFilter === 'all' ? null : state.providerFilter,
      device_id: state.deviceFilter === 'all' ? null : state.deviceFilter,
      tz_offset_minutes: state.tzOffsetMinutes,
      all_time: range.all_time,
    };
  },
}));
