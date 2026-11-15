import { create } from 'zustand';
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

export type TimePreset = 'today' | '7d' | '30d' | 'month' | 'custom';

interface WorkspaceState {
  activePage: NavPage;
  timePreset: TimePreset;
  customStartDate: string; // YYYY-MM-DD
  customEndDate: string; // YYYY-MM-DD
  providerFilter: string; // 'all' | 'claude_code' | 'codex' | 'antigravity'
  currency: CurrencyCode;
  usdToCnyRate: string;
  theme: ThemeMode;
  tzOffsetMinutes: number;

  setActivePage: (page: NavPage) => void;
  setTimePreset: (preset: TimePreset) => void;
  setCustomDateRange: (start: string, end: string) => void;
  setProviderFilter: (provider: string) => void;
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
  currency: 'USD',
  usdToCnyRate: '7.25',
  theme: 'dark',
  tzOffsetMinutes: new Date().getTimezoneOffset() * -1,

  setActivePage: (activePage) => set({ activePage }),
  setTimePreset: (timePreset) => set({ timePreset }),
  setCustomDateRange: (customStartDate, customEndDate) =>
    set({ customStartDate, customEndDate, timePreset: 'custom' }),
  setProviderFilter: (providerFilter) => set({ providerFilter }),
  setCurrency: (currency) => set({ currency }),
  setUsdToCnyRate: (usdToCnyRate) => set({ usdToCnyRate }),
  setTheme: (theme) => set({ theme }),

  getFilter: () => {
    const state = get();
    const now = new Date();
    const endMs = now.getTime();
    let startMs: number;
    let resolvedEndMs = endMs;

    switch (state.timePreset) {
      case 'today': {
        const d = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 0, 0, 0, 0);
        startMs = d.getTime();
        break;
      }
      case '7d':
        startMs = endMs - 7 * 86_400_000;
        break;
      case '30d':
        startMs = endMs - 30 * 86_400_000;
        break;
      case 'month': {
        const d = new Date(now.getFullYear(), now.getMonth(), 1, 0, 0, 0, 0);
        startMs = d.getTime();
        break;
      }
      case 'custom': {
        const s = new Date(`${state.customStartDate}T00:00:00`);
        const e = new Date(`${state.customEndDate}T23:59:59.999`);
        startMs = Number.isNaN(s.getTime()) ? endMs - 7 * 86_400_000 : s.getTime();
        resolvedEndMs = Number.isNaN(e.getTime()) ? endMs : e.getTime();
        break;
      }
    }

    return {
      start_ms: startMs,
      end_ms: resolvedEndMs,
      provider_id: state.providerFilter === 'all' ? null : state.providerFilter,
      tz_offset_minutes: state.tzOffsetMinutes,
    };
  },
}));
