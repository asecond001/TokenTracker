import '@testing-library/jest-dom/vitest';
import { createElement } from 'react';
import { vi } from 'vitest';
import type { EChartMockOption } from './testUtils';

// Mock the ECharts canvas renderer for every jsdom test file while exposing
// series names & option JSON for assertions.
vi.mock('../components/EChartBox', () => ({
  EChartBox: ({ height, option }: { height?: number; option?: EChartMockOption }) => {
    const seriesNames = Array.isArray(option?.series)
      ? option.series.map((s) => String(s.name ?? '')).join(',')
      : '';
    return createElement('div', {
      'data-testid': 'echart-stub',
      'data-series-names': seriesNames,
      'data-option-json': JSON.stringify(option ?? {}),
      style: { height: `${height ?? 240}px` },
    });
  },
}));

if (typeof globalThis.ResizeObserver === 'undefined') {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver = ResizeObserverStub;
}

if (typeof window !== 'undefined' && typeof window.matchMedia !== 'undefined') {
  // Preserve native implementation when available
} else if (typeof window !== 'undefined') {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}

// jsdom lacks PointerEvent; without it fireEvent.pointer* drops clientX / button / pointerId.
if (typeof globalThis.PointerEvent === 'undefined') {
  class PointerEventStub extends MouseEvent {
    readonly pointerId: number;
    readonly pointerType: string;
    readonly isPrimary: boolean;

    constructor(type: string, init: PointerEventInit = {}) {
      super(type, init);
      this.pointerId = init.pointerId ?? 1;
      this.pointerType = init.pointerType ?? 'mouse';
      this.isPrimary = init.isPrimary ?? true;
    }
  }
  globalThis.PointerEvent = PointerEventStub as unknown as typeof PointerEvent;
}
