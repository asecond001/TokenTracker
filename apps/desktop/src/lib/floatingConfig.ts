import { useEffect, useState } from 'react';

export type FloatingSizePreset = 'compact' | 'sm' | 'md' | 'lg';

export const FLOATING_SIZE_PRESETS: ReadonlyArray<{
  id: FloatingSizePreset;
  label: string;
}> = [
  { id: 'compact', label: '胶囊' },
  { id: 'sm', label: '小' },
  { id: 'md', label: '标准' },
  { id: 'lg', label: '大' },
];

export interface FloatingWindowConfig {
  showTokens: boolean;
  showCost: boolean;
  showSpeed: boolean;
  showSession: boolean;
  showSubscriptions: boolean;
  opacity: number;
  sizePreset: FloatingSizePreset;
}

const STORAGE_KEY = 'tokentracker.floating_config.v1';
const CUSTOM_EVENT_NAME = 'tokentracker:floating-config-changed';

export const DEFAULT_FLOATING_CONFIG: FloatingWindowConfig = {
  showTokens: true,
  showCost: true,
  showSpeed: true,
  showSession: true,
  showSubscriptions: true,
  opacity: 0.88,
  sizePreset: 'md',
};

export function normalizeFloatingConfig(
  raw: Partial<FloatingWindowConfig> | null | undefined
): FloatingWindowConfig {
  const showTokens = raw?.showTokens ?? DEFAULT_FLOATING_CONFIG.showTokens;
  const showCost = raw?.showCost ?? DEFAULT_FLOATING_CONFIG.showCost;
  const showSpeed = raw?.showSpeed ?? DEFAULT_FLOATING_CONFIG.showSpeed;
  const showSession = raw?.showSession ?? DEFAULT_FLOATING_CONFIG.showSession;
  const showSubscriptions = raw?.showSubscriptions ?? DEFAULT_FLOATING_CONFIG.showSubscriptions;
  const rawOpacity =
    typeof raw?.opacity === 'number' && Number.isFinite(raw.opacity)
      ? raw.opacity
      : DEFAULT_FLOATING_CONFIG.opacity;
  const opacity = Math.min(1, Math.max(0.55, rawOpacity));
  const sizePreset: FloatingSizePreset =
    raw?.sizePreset === 'compact' ||
    raw?.sizePreset === 'sm' ||
    raw?.sizePreset === 'md' ||
    raw?.sizePreset === 'lg'
      ? raw.sizePreset
      : 'md';

  // Ensure at least one element remains visible
  const anyVisible = showTokens || showCost || showSpeed || showSession || showSubscriptions;
  return {
    showTokens: anyVisible ? showTokens : true,
    showCost,
    showSpeed,
    showSession,
    showSubscriptions,
    opacity,
    sizePreset,
  };
}

export function loadFloatingConfig(): FloatingWindowConfig {
  if (typeof window === 'undefined' || !window.localStorage) {
    return DEFAULT_FLOATING_CONFIG;
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return DEFAULT_FLOATING_CONFIG;
    }
    const parsed = JSON.parse(raw) as Partial<FloatingWindowConfig>;
    return normalizeFloatingConfig(parsed);
  } catch {
    return DEFAULT_FLOATING_CONFIG;
  }
}

export function saveFloatingConfig(
  patch: Partial<FloatingWindowConfig>
): FloatingWindowConfig {
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

export function computeFloatingDimensions(cfg: FloatingWindowConfig): {
  width: number;
  height: number;
} {
  if (cfg.sizePreset === 'compact') {
    return { width: 288, height: 52 };
  }
  const baseWidth = cfg.sizePreset === 'sm' ? 268 : cfg.sizePreset === 'lg' ? 356 : 304;
  let height = 46; // Header bar + padding
  if (cfg.showTokens || cfg.showCost || cfg.showSpeed) {
    height += cfg.sizePreset === 'sm' ? 54 : cfg.sizePreset === 'lg' ? 68 : 60;
  }
  if (cfg.showSession) {
    height += cfg.sizePreset === 'sm' ? 44 : 50;
  }
  if (cfg.showSubscriptions) {
    height += cfg.sizePreset === 'sm' ? 76 : cfg.sizePreset === 'lg' ? 98 : 86;
  }
  return {
    width: baseWidth,
    height: Math.max(96, height),
  };
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
