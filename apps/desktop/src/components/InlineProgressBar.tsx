export interface InlineProgressBarProps {
  percent: number;
  alertLevel?: string;
  accentColor?: string;
  heightPx?: number;
  showTrack?: boolean;
}

export function InlineProgressBar({
  percent,
  alertLevel = 'normal',
  accentColor,
  heightPx = 5,
  showTrack = true,
}: InlineProgressBarProps) {
  const clamped = Math.max(0, Math.min(100, Number.isFinite(percent) ? percent : 0));

  let fillColor = accentColor ?? 'var(--accent-emerald)';
  if (!accentColor) {
    if (alertLevel === 'exhausted' || alertLevel === 'critical' || clamped >= 95) {
      fillColor = 'var(--accent-rose)';
    } else if (alertLevel === 'warning' || clamped >= 80) {
      fillColor = 'var(--accent-amber)';
    }
  }

  return (
    <div
      className="w-full overflow-hidden rounded-full"
      style={{
        height: `${heightPx}px`,
        backgroundColor: showTrack ? 'var(--border-hairline)' : 'transparent',
        transition: 'background-color 180ms cubic-bezier(0.16, 1, 0.3, 1)',
      }}
    >
      <div
        className="h-full rounded-full"
        style={{
          width: `${clamped}%`,
          backgroundColor: fillColor,
          transition:
            'width 280ms cubic-bezier(0.16, 1, 0.3, 1), background-color 220ms cubic-bezier(0.16, 1, 0.3, 1)',
        }}
      />
    </div>
  );
}
