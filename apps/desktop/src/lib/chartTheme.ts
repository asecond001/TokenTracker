import { useMemo } from 'react';
import { useWorkspaceStore, type ResolvedTheme } from '../stores/workspaceStore';

export interface ChartTheme {
  isLight: boolean;
  textPrimary: string;
  textSecondary: string;
  textMuted: string;
  borderSubtle: string;
  tooltipBg: string;
  tooltipBorder: string;
  tooltipText: string;
  splitLineColor: string;
  axisLineColor: string;
  dashedTrendColor: string;
  heatmapEmptyColor: string;
  heatmapBorderColor: string;
  heatmapRamp: [string, string, string, string];
}

const DARK_CHART_THEME: ChartTheme = {
  isLight: false,
  textPrimary: '#eceff4',
  textSecondary: '#98a2b3',
  textMuted: '#6b7280',
  borderSubtle: '#232733',
  tooltipBg: '#161922',
  tooltipBorder: '#2d3343',
  tooltipText: '#eceff4',
  splitLineColor: '#1f232e',
  axisLineColor: '#2b3040',
  dashedTrendColor: '#eceff4',
  heatmapEmptyColor: '#161b22',
  heatmapBorderColor: '#0c0d10',
  heatmapRamp: ['#0e4429', '#006d32', '#26a641', '#39d353'],
};

const LIGHT_CHART_THEME: ChartTheme = {
  isLight: true,
  textPrimary: '#111827',
  textSecondary: '#475467',
  textMuted: '#667085',
  borderSubtle: '#e2e6ef',
  tooltipBg: '#ffffff',
  tooltipBorder: '#e2e6ef',
  tooltipText: '#111827',
  splitLineColor: '#eef1f6',
  axisLineColor: '#d5dbe7',
  dashedTrendColor: '#374151',
  heatmapEmptyColor: '#ebedf0',
  heatmapBorderColor: '#ffffff',
  heatmapRamp: ['#9be9a8', '#40c463', '#30a14e', '#216e39'],
};

export function getChartTheme(resolvedTheme: ResolvedTheme): ChartTheme {
  return resolvedTheme === 'light' ? LIGHT_CHART_THEME : DARK_CHART_THEME;
}

export function useChartTheme(): ChartTheme {
  const resolvedTheme = useWorkspaceStore((state) => state.resolvedTheme);
  return useMemo(() => getChartTheme(resolvedTheme), [resolvedTheme]);
}
