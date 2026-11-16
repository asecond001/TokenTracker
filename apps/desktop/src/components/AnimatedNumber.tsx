import { useEffect, useRef, useState } from 'react';

export interface AnimatedNumberProps {
  value: number;
  formatter: (n: number) => string;
  className?: string;
}

const DURATION_MS = 420;

function easeOutQuint(t: number): number {
  return 1 - Math.pow(1 - Math.min(1, Math.max(0, t)), 5);
}

function isAutomatedTestEnv(): boolean {
  return typeof navigator !== 'undefined' && /jsdom/i.test(navigator.userAgent ?? '');
}

export function AnimatedNumber({ value, formatter, className }: AnimatedNumberProps) {
  const safeInitial = Number.isFinite(value) ? value : 0;
  const [displayValue, setDisplayValue] = useState<number>(safeInitial);
  const currentRef = useRef<number>(safeInitial);

  useEffect(() => {
    if (!Number.isFinite(value)) {
      return;
    }

    const from = currentRef.current;
    const to = value;
    if (from === to) {
      return;
    }

    if (isAutomatedTestEnv() || typeof requestAnimationFrame !== 'function') {
      currentRef.current = to;
      setDisplayValue(to);
      return;
    }

    let rafId = 0;
    let startTs: number | null = null;

    const tick = (now: number) => {
      if (startTs === null) {
        startTs = now;
      }
      const elapsed = now - startTs;
      const progress = Math.min(1, Math.max(0, elapsed / DURATION_MS));
      const eased = easeOutQuint(progress);
      const next = from + (to - from) * eased;

      currentRef.current = next;
      setDisplayValue(next);

      if (progress < 1) {
        rafId = requestAnimationFrame(tick);
      } else {
        currentRef.current = to;
        setDisplayValue(to);
      }
    };

    rafId = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(rafId);
    };
  }, [value]);

  const mergedClassName = className ? `tabular-nums ${className}` : 'tabular-nums';
  const text = Number.isFinite(value) ? formatter(displayValue) : formatter(value);

  return <span className={mergedClassName}>{text}</span>;
}
