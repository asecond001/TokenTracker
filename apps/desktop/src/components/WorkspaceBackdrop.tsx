import { useQuery } from '@tanstack/react-query';
import { ipc } from '../lib/ipc';
import { useWorkspaceStore, type ResolvedTheme } from '../stores/workspaceStore';
import type { AppPreferences } from '../types/ipc';

export type BackgroundPresetId =
  | 'aurora_slate'
  | 'obsidian_mist'
  | 'nordic_frost'
  | 'amber_dusk';

export interface BackgroundGradientPreset {
  id: BackgroundPresetId;
  label: string;
  subtitle: string;
  gradients: Record<ResolvedTheme, string>;
}

export const BACKGROUND_GRADIENT_PRESETS: readonly BackgroundGradientPreset[] = [
  {
    id: 'aurora_slate',
    label: '极光石板',
    subtitle: '靛蓝 / 翠绿冷调微光',
    gradients: {
      dark: [
        'radial-gradient(68% 58% at 14% 8%, rgba(99, 102, 241, 0.85) 0%, rgba(99, 102, 241, 0) 70%)',
        'radial-gradient(60% 54% at 86% 12%, rgba(16, 185, 129, 0.62) 0%, rgba(16, 185, 129, 0) 68%)',
        'radial-gradient(70% 60% at 52% 96%, rgba(56, 189, 248, 0.44) 0%, rgba(56, 189, 248, 0) 72%)',
      ].join(', '),
      light: [
        'radial-gradient(68% 58% at 14% 8%, rgba(79, 70, 229, 0.42) 0%, rgba(79, 70, 229, 0) 70%)',
        'radial-gradient(60% 54% at 86% 12%, rgba(5, 150, 105, 0.34) 0%, rgba(5, 150, 105, 0) 68%)',
        'radial-gradient(70% 60% at 52% 96%, rgba(2, 132, 199, 0.28) 0%, rgba(2, 132, 199, 0) 72%)',
      ].join(', '),
    },
  },
  {
    id: 'obsidian_mist',
    label: '黑曜薄雾',
    subtitle: '灰紫 / 岩灰低饱和雾光',
    gradients: {
      dark: [
        'radial-gradient(64% 58% at 20% 10%, rgba(139, 92, 246, 0.68) 0%, rgba(139, 92, 246, 0) 70%)',
        'radial-gradient(64% 58% at 82% 24%, rgba(100, 116, 139, 0.78) 0%, rgba(100, 116, 139, 0) 70%)',
        'radial-gradient(72% 55% at 45% 92%, rgba(71, 85, 105, 0.62) 0%, rgba(71, 85, 105, 0) 72%)',
      ].join(', '),
      light: [
        'radial-gradient(64% 58% at 20% 10%, rgba(109, 40, 217, 0.32) 0%, rgba(109, 40, 217, 0) 70%)',
        'radial-gradient(64% 58% at 82% 24%, rgba(100, 116, 139, 0.42) 0%, rgba(100, 116, 139, 0) 70%)',
        'radial-gradient(72% 55% at 45% 92%, rgba(148, 163, 184, 0.46) 0%, rgba(148, 163, 184, 0) 72%)',
      ].join(', '),
    },
  },
  {
    id: 'nordic_frost',
    label: '北欧霜蓝',
    subtitle: '冰青 / 钴蓝清冽视域',
    gradients: {
      dark: [
        'radial-gradient(66% 58% at 16% 14%, rgba(56, 189, 248, 0.76) 0%, rgba(56, 189, 248, 0) 70%)',
        'radial-gradient(58% 54% at 84% 18%, rgba(59, 130, 246, 0.68) 0%, rgba(59, 130, 246, 0) 70%)',
        'radial-gradient(68% 58% at 50% 94%, rgba(148, 163, 184, 0.40) 0%, rgba(148, 163, 184, 0) 72%)',
      ].join(', '),
      light: [
        'radial-gradient(66% 58% at 16% 14%, rgba(2, 132, 199, 0.38) 0%, rgba(2, 132, 199, 0) 70%)',
        'radial-gradient(58% 54% at 84% 18%, rgba(37, 99, 235, 0.32) 0%, rgba(37, 99, 235, 0) 70%)',
        'radial-gradient(68% 58% at 50% 94%, rgba(14, 165, 233, 0.24) 0%, rgba(14, 165, 233, 0) 72%)',
      ].join(', '),
    },
  },
  {
    id: 'amber_dusk',
    label: '琥珀暮光',
    subtitle: '暖金 / 霞红克制氛围',
    gradients: {
      dark: [
        'radial-gradient(64% 56% at 18% 12%, rgba(245, 158, 11, 0.72) 0%, rgba(245, 158, 11, 0) 70%)',
        'radial-gradient(58% 52% at 86% 16%, rgba(244, 63, 94, 0.52) 0%, rgba(244, 63, 94, 0) 68%)',
        'radial-gradient(68% 58% at 52% 94%, rgba(99, 102, 241, 0.42) 0%, rgba(99, 102, 241, 0) 72%)',
      ].join(', '),
      light: [
        'radial-gradient(64% 56% at 18% 12%, rgba(217, 119, 6, 0.38) 0%, rgba(217, 119, 6, 0) 70%)',
        'radial-gradient(58% 52% at 86% 16%, rgba(225, 29, 72, 0.26) 0%, rgba(225, 29, 72, 0) 68%)',
        'radial-gradient(68% 58% at 52% 94%, rgba(79, 70, 229, 0.22) 0%, rgba(79, 70, 229, 0) 72%)',
      ].join(', '),
    },
  },
];

function clampNumber(value: number, min: number, max: number, fallback: number): number {
  if (!Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, value));
}

function resolveGradientPreset(presetId: string | undefined): BackgroundGradientPreset {
  return (
    BACKGROUND_GRADIENT_PRESETS.find((preset) => preset.id === presetId) ??
    BACKGROUND_GRADIENT_PRESETS[0]
  );
}

export interface WorkspaceBackdropProps {
  preferences?: AppPreferences | null;
}

export function WorkspaceBackdrop({ preferences }: WorkspaceBackdropProps) {
  const resolvedTheme = useWorkspaceStore((state) => state.resolvedTheme);

  const bgMode = preferences?.bg_mode ?? 'none';
  const bgImagePath = preferences?.bg_image_path ?? null;
  const isImageActive = bgMode === 'image' && Boolean(bgImagePath);

  const { data: imageDataUrl } = useQuery({
    queryKey: ['bg-image-data-url', bgImagePath],
    queryFn: () => ipc.getBackgroundImageDataUrl(),
    enabled: isImageActive,
    staleTime: Number.POSITIVE_INFINITY,
  });

  if (!preferences || bgMode === 'none') {
    return null;
  }

  const opacity = clampNumber(preferences.bg_opacity, 0.05, 0.6, 0.18);
  const blurPx = Math.round(clampNumber(preferences.bg_blur_px, 0, 32, 16));

  if (bgMode === 'gradient') {
    const preset = resolveGradientPreset(preferences.bg_preset_id);
    return (
      <div
        aria-hidden="true"
        data-testid="workspace-backdrop-gradient"
        className="pointer-events-none absolute inset-0 z-0 overflow-hidden"
      >
        <div
          className="absolute inset-0"
          style={{
            backgroundImage: preset.gradients[resolvedTheme],
            opacity,
            filter: blurPx > 0 ? `blur(${blurPx}px)` : undefined,
            transform: blurPx > 0 ? 'scale(1.06)' : undefined,
            transition: 'opacity 180ms cubic-bezier(0.16, 1, 0.3, 1)',
          }}
        />
      </div>
    );
  }

  if (isImageActive && imageDataUrl) {
    return (
      <div
        aria-hidden="true"
        data-testid="workspace-backdrop-image"
        className="pointer-events-none absolute inset-0 z-0 overflow-hidden"
      >
        <img
          src={imageDataUrl}
          alt=""
          draggable={false}
          className="w-full h-full object-cover"
          style={{
            filter: blurPx > 0 ? `blur(${blurPx}px)` : undefined,
            transform: 'scale(1.04)',
          }}
        />
        <div
          className="absolute inset-0"
          style={{
            backgroundColor: 'var(--bg-canvas)',
            opacity: 1 - opacity,
            transition: 'opacity 180ms cubic-bezier(0.16, 1, 0.3, 1)',
          }}
        />
      </div>
    );
  }

  return null;
}
