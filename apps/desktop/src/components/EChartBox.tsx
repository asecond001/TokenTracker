import { useEffect, useRef } from 'react';
import * as echarts from 'echarts/core';
import { BarChart, HeatmapChart, LineChart, ScatterChart } from 'echarts/charts';
import {
  CalendarComponent,
  GridComponent,
  LegendComponent,
  TooltipComponent,
  VisualMapPiecewiseComponent,
} from 'echarts/components';
import { CanvasRenderer } from 'echarts/renderers';
import type { EChartsOption } from 'echarts';

// Register only the charts/components the workspace actually renders so Vite
// can tree-shake the rest of ECharts out of the bundle.
echarts.use([
  BarChart,
  LineChart,
  HeatmapChart,
  ScatterChart,
  GridComponent,
  TooltipComponent,
  LegendComponent,
  VisualMapPiecewiseComponent,
  CalendarComponent,
  CanvasRenderer,
]);

export interface EChartBoxProps {
  option: EChartsOption;
  height?: number;
  className?: string;
  notMerge?: boolean;
  lazyUpdate?: boolean;
}

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}

/**
 * Full-width borderless ECharts container with automatic resize & theme-aware sizing.
 */
export function EChartBox({
  option,
  height = 260,
  className = '',
  notMerge = false,
  lazyUpdate = true,
}: EChartBoxProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<echarts.EChartsType | null>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const chart = echarts.init(el, undefined, { renderer: 'canvas' });
    chartRef.current = chart;

    let rafId: number | null = null;
    const scheduleResize = () => {
      if (typeof requestAnimationFrame !== 'function') {
        if (!chart.isDisposed() && el.clientWidth > 0 && el.clientHeight > 0) {
          chart.resize();
        }
        return;
      }
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
      }
      rafId = requestAnimationFrame(() => {
        rafId = null;
        if (!chart.isDisposed() && el.clientWidth > 0 && el.clientHeight > 0) {
          chart.resize();
        }
      });
    };

    let ro: ResizeObserver | null = null;
    if (typeof ResizeObserver !== 'undefined') {
      ro = new ResizeObserver((entries) => {
        const entry = entries[0];
        if (!entry) return;
        const { width, height: rectHeight } = entry.contentRect;
        if (width <= 0 || rectHeight <= 0) return;
        scheduleResize();
      });
      ro.observe(el);
    } else if (typeof window !== 'undefined') {
      window.addEventListener('resize', scheduleResize);
    }

    return () => {
      if (rafId !== null && typeof cancelAnimationFrame === 'function') {
        cancelAnimationFrame(rafId);
      }
      if (ro) {
        ro.disconnect();
      } else if (typeof window !== 'undefined') {
        window.removeEventListener('resize', scheduleResize);
      }
      if (!chart.isDisposed()) {
        chart.dispose();
      }
      chartRef.current = null;
    };
  }, []);

  useEffect(() => {
    const chart = chartRef.current;
    if (!chart || chart.isDisposed()) return;

    const reducedMotion = prefersReducedMotion();
    const nextOption: EChartsOption = reducedMotion
      ? {
          ...option,
          animation: false,
          animationDuration: 0,
          animationDurationUpdate: 0,
        }
      : {
          animationDuration: 280,
          animationDurationUpdate: 260,
          animationEasing: 'cubicOut',
          animationEasingUpdate: 'cubicOut',
          ...option,
        };

    chart.setOption(nextOption, {
      notMerge,
      lazyUpdate,
      replaceMerge: notMerge ? undefined : ['series'],
    });
  }, [option, notMerge, lazyUpdate]);

  useEffect(() => {
    const chart = chartRef.current;
    const el = containerRef.current;
    if (!chart || !el || chart.isDisposed()) return;
    if (el.clientWidth > 0 && el.clientHeight > 0) {
      chart.resize();
    }
  }, [height]);

  return (
    <div
      ref={containerRef}
      style={{ height: `${height}px`, width: '100%' }}
      className={className}
    />
  );
}
