interface InlineProgressBarProps {
  percent: number;
  alertLevel?: string;
  heightPx?: number;
  showTrack?: boolean;
}

export function InlineProgressBar({
  percent,
  alertLevel = 'normal',
  heightPx = 5,
  showTrack = true,
}: InlineProgressBarProps) {
  const clamped = Math.max(0, Math.min(100, Number.isFinite(percent) ? percent : 0));

  let fillColor = 'var(--accent-emerald)';
  if (alertLevel === 'exhausted' || alertLevel === 'critical' || clamped >= 95) {
    fillColor = 'var(--accent-rose)';
  } else if (alertLevel === 'warning' || clamped >= 80) {
    fillColor = 'var(--accent-amber)';
  }

  return (
    <div
      className="w-full overflow-hidden rounded-full"
      style={{
        height: `${heightPx}px`,
        backgroundColor: showTrack ? 'var(--border-subtle)' : 'transparent',
      }}
    >
      <div
        className="h-full rounded-full transition-[width] duration-300"
        style={{
          width: `${clamped}%`,
          backgroundColor: fillColor,
        }}
      />
    </div>
  );
}
