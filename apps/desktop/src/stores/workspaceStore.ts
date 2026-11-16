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

function parseLocalYmd(ymd: string): { year: number; month: number; day: number } | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(ymd.trim());
  if (!match) {
    return null;
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const probe = new Date(year, month - 1, day);
  if (
    Number.isNaN(probe.getTime()) ||
    probe.getFullYear() !== year ||
    probe.getMonth() !== month - 1 ||
    probe.getDate() !== day
  ) {
    return null;
  }
  return { year, month, day };
}

function localDayStartMs(year: number, month: number, day: number): number {
  return new Date(year, month - 1, day, 0, 0, 0, 0).getTime();
}

function localDayEndMs(year: number, month: number, day: number): number {
  return new Date(year, month - 1, day, 23, 59, 59, 999).getTime();
}

function formatYmdParts(year: number, month: number, day: number): string {
  return `${year}-${String(month).padStart(2, '0')}-${String(day).padStart(2, '0')}`;
}

export function getPresetDateInputs(
  preset: TimePreset,
  customRange?: CustomTimeRange,
  now: Date = new Date()
): { start: string; end: string } {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const todayStr = toDateInputString(today);
  switch (preset) {
    case 'today':
      return { start: todayStr, end: todayStr };
    case '7d': {
      const start = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 6);
      return { start: toDateInputString(start), end: todayStr };
    }
    case '30d': {
      const start = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 29);
      return { start: toDateInputString(start), end: todayStr };
    }
    case 'month': {
      const start = new Date(today.getFullYear(), today.getMonth(), 1);
      return { start: toDateInputString(start), end: todayStr };
    }
    case 'all': {
      const fallbackStart = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 6);
      return {
        start: customRange?.start || toDateInputString(fallbackStart),
        end: customRange?.end || todayStr,
      };
    }
    case 'custom': {
      const parsedStart = customRange?.start ? parseLocalYmd(customRange.start) : null;
      const parsedEnd = customRange?.end ? parseLocalYmd(customRange.end) : null;
      if (parsedStart && parsedEnd) {
        const sMs = localDayStartMs(parsedStart.year, parsedStart.month, parsedStart.day);
        const eMs = localDayStartMs(parsedEnd.year, parsedEnd.month, parsedEnd.day);
        const sStr = formatYmdParts(parsedStart.year, parsedStart.month, parsedStart.day);
        const eStr = formatYmdParts(parsedEnd.year, parsedEnd.month, parsedEnd.day);
        return sMs <= eMs ? { start: sStr, end: eStr } : { start: eStr, end: sStr };
      }
      const fallbackStart = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 6);
      return { start: toDateInputString(fallbackStart), end: todayStr };
    }
  }
}

export function formatResolvedDateSpan(
  preset: TimePreset,
  customRange?: CustomTimeRange,
  now: Date = new Date()
): string {
  if (preset === 'all') {
    return '全量历史';
  }
  const { start, end } = getPresetDateInputs(preset, customRange, now);
  return start === end ? start : `${start} ~ ${end}`;
}

export function computeTimeRange(
  preset: TimePreset,
  customRange?: CustomTimeRange,
  now: Date = new Date()
): ComputedTimeRange {
  const endMs = now.getTime();
  const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 0, 0, 0, 0);
  const todayEndMs = localDayEndMs(now.getFullYear(), now.getMonth() + 1, now.getDate());

  switch (preset) {
    case 'today': {
      return { start_ms: todayStart.getTime(), end_ms: endMs, all_time: false };
    }
    case '7d': {
      const start = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 6, 0, 0, 0, 0);
      return { start_ms: start.getTime(), end_ms: todayEndMs, all_time: false };
    }
    case '30d': {
      const start = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 29, 0, 0, 0, 0);
      return { start_ms: start.getTime(), end_ms: todayEndMs, all_time: false };
    }
    case 'month': {
      const start = new Date(now.getFullYear(), now.getMonth(), 1, 0, 0, 0, 0);
      return { start_ms: start.getTime(), end_ms: todayEndMs, all_time: false };
    }
    case 'all':
      return { start_ms: 0, end_ms: endMs, all_time: true };
    case 'custom': {
      const parsedStart = customRange?.start ? parseLocalYmd(customRange.start) : null;
      const parsedEnd = customRange?.end ? parseLocalYmd(customRange.end) : null;
      if (parsedStart && parsedEnd) {
        const sMs = localDayStartMs(parsedStart.year, parsedStart.month, parsedStart.day);
        const eMs = localDayStartMs(parsedEnd.year, parsedEnd.month, parsedEnd.day);
        if (sMs <= eMs) {
          return {
            start_ms: sMs,
            end_ms: localDayEndMs(parsedEnd.year, parsedEnd.month, parsedEnd.day),
            all_time: false,
          };
        }
        return {
          start_ms: eMs,
          end_ms: localDayEndMs(parsedStart.year, parsedStart.month, parsedStart.day),
          all_time: false,
        };
      }
      const fallbackStart = new Date(
        now.getFullYear(),
        now.getMonth(),
        now.getDate() - 6,
        0,
        0,
        0,
        0
      );
      return {
        start_ms: parsedStart
          ? localDayStartMs(parsedStart.year, parsedStart.month, parsedStart.day)
          : fallbackStart.getTime(),
        end_ms: parsedEnd
          ? localDayEndMs(parsedEnd.year, parsedEnd.month, parsedEnd.day)
          : todayEndMs,
        all_time: false,
      };
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
const weekAgo = new Date(nowDate.getFullYear(), nowDate.getMonth(), nowDate.getDate() - 6, 0, 0, 0, 0);

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
