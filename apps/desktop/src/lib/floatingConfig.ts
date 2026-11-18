import { useEffect, useState } from 'react';

/** Building blocks the floating monitor can show, in user-chosen order. */
export type FloatingModuleId =
  | 'tokens'
  | 'cost'
  | 'speed'
  | 'requests'
  | 'cache'
  | 'sessions'
  | 'quotas'
  | 'pulse'
  | 'providers'
  | 'models';

export type FloatingModuleKind = 'stat' | 'panel';

export interface FloatingModuleMeta {
  id: FloatingModuleId;
  label: string;
  /** `stat` modules render in the top metric strip; `panel` modules in the grid below. */
  kind: FloatingModuleKind;
  /** Whether the module needs the (heavier) today overview query. */
  needsOverview: boolean;
}

export const FLOATING_MODULES: ReadonlyArray<FloatingModuleMeta> = [
  { id: 'tokens', label: 'Token 用量', kind: 'stat', needsOverview: false },
  { id: 'cost', label: '等效费用', kind: 'stat', needsOverview: false },
  { id: 'speed', label: '生成速率', kind: 'stat', needsOverview: false },
  { id: 'requests', label: '请求次数', kind: 'stat', needsOverview: false },
  { id: 'cache', label: '缓存命中', kind: 'stat', needsOverview: true },
  { id: 'sessions', label: '活跃会话', kind: 'panel', needsOverview: false },
  { id: 'quotas', label: '配额窗口', kind: 'panel', needsOverview: false },
  { id: 'pulse', label: '今日脉搏', kind: 'panel', needsOverview: true },
  { id: 'providers', label: '提供商占比', kind: 'panel', needsOverview: false },
  { id: 'models', label: '主力模型', kind: 'panel', needsOverview: true },
];

const MODULE_IDS: ReadonlySet<string> = new Set(FLOATING_MODULES.map((m) => m.id));

export function floatingModuleMeta(id: FloatingModuleId): FloatingModuleMeta {
  return FLOATING_MODULES.find((m) => m.id === id) ?? FLOATING_MODULES[0];
}

export interface FloatingWindowConfig {
  /** Enabled modules, in display order. Never empty. */
  modules: FloatingModuleId[];
  opacity: number;
  compact: boolean;
}

const STORAGE_KEY = 'tokenvane.floating_config.v1';
const CUSTOM_EVENT_NAME = 'tokenvane:floating-config-changed';

export const DEFAULT_FLOATING_MODULES: ReadonlyArray<FloatingModuleId> = [
  'tokens',
  'cost',
  'speed',
  'requests',
  'sessions',
  'quotas',
  'pulse',
  'providers',
];

export const DEFAULT_FLOATING_CONFIG: FloatingWindowConfig = {
  modules: [...DEFAULT_FLOATING_MODULES],
  opacity: 0.88,
  compact: false,
};

/** Pre-0.4.6 boolean toggles, migrated into `modules`. */
interface LegacyFloatingFlags {
  showTokens?: boolean;
  showCost?: boolean;
  showSpeed?: boolean;
  showSession?: boolean;
  showSubscriptions?: boolean;
  sizePreset?: string;
}

type RawFloatingConfig = Partial<FloatingWindowConfig> & LegacyFloatingFlags;

function migrateLegacyModules(raw: LegacyFloatingFlags): FloatingModuleId[] | null {
  const hasLegacy =
    raw.showTokens !== undefined ||
    raw.showCost !== undefined ||
    raw.showSpeed !== undefined ||
    raw.showSession !== undefined ||
    raw.showSubscriptions !== undefined;
  if (!hasLegacy) return null;
  const out: FloatingModuleId[] = [];
  if (raw.showTokens ?? true) out.push('tokens');
  if (raw.showCost ?? true) out.push('cost');
  if (raw.showSpeed ?? true) out.push('speed');
  if (raw.showSession ?? true) out.push('sessions');
  if (raw.showSubscriptions ?? true) out.push('quotas');
  return out;
}

export function normalizeFloatingConfig(
  raw: RawFloatingConfig | null | undefined
): FloatingWindowConfig {
  let modules: FloatingModuleId[];
  if (Array.isArray(raw?.modules)) {
    const seen = new Set<string>();
    modules = raw.modules.filter((id): id is FloatingModuleId => {
      if (typeof id !== 'string' || !MODULE_IDS.has(id) || seen.has(id)) return false;
      seen.add(id);
      return true;
    });
  } else {
    modules = (raw && migrateLegacyModules(raw)) ?? [...DEFAULT_FLOATING_MODULES];
  }
  // Ensure at least one module remains visible.
  if (modules.length === 0) modules = ['tokens'];

  const rawOpacity =
    typeof raw?.opacity === 'number' && Number.isFinite(raw.opacity)
      ? raw.opacity
      : DEFAULT_FLOATING_CONFIG.opacity;
  const opacity = Math.min(1, Math.max(0.55, rawOpacity));
  const compact = typeof raw?.compact === 'boolean' ? raw.compact : raw?.sizePreset === 'compact';

  return { modules, opacity, compact };
}

export function isModuleEnabled(cfg: FloatingWindowConfig, id: FloatingModuleId): boolean {
  return cfg.modules.includes(id);
}

/** Toggles a module on (appended at the end) or off; keeps at least one module. */
export function toggleFloatingModule(
  modules: ReadonlyArray<FloatingModuleId>,
  id: FloatingModuleId
): FloatingModuleId[] {
  if (modules.includes(id)) {
    const next = modules.filter((m) => m !== id);
    return next.length > 0 ? next : [...modules];
  }
  return [...modules, id];
}

/** Moves an enabled module one slot earlier (`-1`) or later (`+1`). */
export function moveFloatingModule(
  modules: ReadonlyArray<FloatingModuleId>,
  id: FloatingModuleId,
  delta: -1 | 1
): FloatingModuleId[] {
  const idx = modules.indexOf(id);
  const target = idx + delta;
  if (idx < 0 || target < 0 || target >= modules.length) return [...modules];
  const next = [...modules];
  [next[idx], next[target]] = [next[target], next[idx]];
  return next;
}

export function loadFloatingConfig(): FloatingWindowConfig {
  if (typeof window === 'undefined' || !window.localStorage) {
    return normalizeFloatingConfig(DEFAULT_FLOATING_CONFIG);
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return normalizeFloatingConfig(DEFAULT_FLOATING_CONFIG);
    }
    return normalizeFloatingConfig(JSON.parse(raw) as RawFloatingConfig);
  } catch {
    return normalizeFloatingConfig(DEFAULT_FLOATING_CONFIG);
  }
}

export function saveFloatingConfig(patch: Partial<FloatingWindowConfig>): FloatingWindowConfig {
  const current = loadFloatingConfig();
  const next = normalizeFloatingConfig({ ...current, ...patch });
  if (typeof window !== 'undefined' && window.localStorage) {
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    } catch {
      // Ignore storage quota restrictions
    }
    window.dispatchEvent(new CustomEvent(CUSTOM_EVENT_NAME, { detail: next }));
  }
  return next;
}

/** Wide windows lay panels out in this many columns. */
export function floatingPanelColumns(width: number): number {
  if (width >= 760) return 3;
  if (width >= 470) return 2;
  return 1;
}

export function computeFloatingDimensions(
  cfg: FloatingWindowConfig,
  currentWidth = 304
): {
  width: number;
  height: number;
} {
  const width = Math.min(960, Math.max(220, Math.round(currentWidth || 304)));
  if (cfg.compact) {
    return { width: Math.max(260, width), height: 52 };
  }
  const stats = cfg.modules.filter((id) => floatingModuleMeta(id).kind === 'stat').length;
  const panels = cfg.modules.filter((id) => floatingModuleMeta(id).kind === 'panel').length;
  const statsPerRow = width >= 470 ? 4 : 2;
  const columns = floatingPanelColumns(width);
  let height = 46; // Header bar + padding
  if (stats > 0) height += Math.ceil(stats / statsPerRow) * 52;
  if (panels > 0) height += Math.ceil(panels / columns) * 92;
  return { width, height: Math.min(900, Math.max(96, height)) };
}

export function useFloatingConfig(): [
  FloatingWindowConfig,
  (patch: Partial<FloatingWindowConfig>) => FloatingWindowConfig,
] {
  const [config, setConfig] = useState<FloatingWindowConfig>(() => loadFloatingConfig());

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const onStorage = (e: StorageEvent) => {
      if (!e.key || e.key === STORAGE_KEY) {
        setConfig(loadFloatingConfig());
      }
    };
    const onCustom = (e: Event) => {
      const ce = e as CustomEvent<FloatingWindowConfig>;
      setConfig(ce.detail ? normalizeFloatingConfig(ce.detail) : loadFloatingConfig());
    };
    window.addEventListener('storage', onStorage);
    window.addEventListener(CUSTOM_EVENT_NAME, onCustom);
    return () => {
      window.removeEventListener('storage', onStorage);
      window.removeEventListener(CUSTOM_EVENT_NAME, onCustom);
    };
  }, []);

  const update = (patch: Partial<FloatingWindowConfig>) => {
    const saved = saveFloatingConfig(patch);
    setConfig(saved);
    return saved;
  };

  return [config, update];
}
