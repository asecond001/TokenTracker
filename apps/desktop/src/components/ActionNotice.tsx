import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { CheckCircle2, Info, X, XCircle } from 'lucide-react';
import { getLastActionAnchor } from '../lib/actionAnchor';

export type ActionNoticeKind = 'success' | 'error' | 'info';

export interface ActionNoticeProps {
  /** Message to show; `null` hides the notice. */
  text: ReactNode | null;
  kind?: ActionNoticeKind;
  onClose: () => void;
  /**
   * Explicit anchor element. Defaults to the control the user most recently
   * activated, so the notice appears beside the button that triggered it.
   */
  anchor?: HTMLElement | null;
  /** Auto-dismiss delay; errors stay until closed when omitted. */
  autoHideMs?: number;
}

interface Position {
  top: number;
  left: number;
  placement: 'below' | 'above';
}

const GAP_PX = 6;
const EDGE_PX = 8;
const DEFAULT_SUCCESS_HIDE_MS = 5000;

function kindColor(kind: ActionNoticeKind): string {
  if (kind === 'error') return 'var(--accent-rose)';
  if (kind === 'info') return 'var(--text-secondary)';
  return 'var(--accent-emerald)';
}

function KindIcon({ kind }: { kind: ActionNoticeKind }) {
  const className = 'w-3.5 h-3.5 shrink-0 mt-px';
  if (kind === 'error') return <XCircle className={className} />;
  if (kind === 'info') return <Info className={className} />;
  return <CheckCircle2 className={className} />;
}

/**
 * Feedback popover rendered next to the triggering control (falls back to an
 * inline strip when no anchor is available, e.g. for background refreshes).
 */
export function ActionNotice({
  text,
  kind = 'success',
  onClose,
  anchor,
  autoHideMs,
}: ActionNoticeProps) {
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null);
  const [position, setPosition] = useState<Position | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);
  const visible = text !== null && text !== undefined && text !== '';

  // Capture the anchor each time a new message appears.
  useLayoutEffect(() => {
    if (!visible) {
      setAnchorEl(null);
      setPosition(null);
      return;
    }
    setAnchorEl(anchor ?? getLastActionAnchor());
  }, [visible, text, anchor]);

  const reposition = useCallback(() => {
    const pop = popoverRef.current;
    if (!anchorEl || !pop || !anchorEl.isConnected) {
      return;
    }
    const rect = anchorEl.getBoundingClientRect();
    const popRect = pop.getBoundingClientRect();
    const viewportW = window.innerWidth;
    const viewportH = window.innerHeight;

    const maxLeft = Math.max(EDGE_PX, viewportW - popRect.width - EDGE_PX);
    const left = Math.min(Math.max(EDGE_PX, rect.left), maxLeft);

    const belowTop = rect.bottom + GAP_PX;
    const fitsBelow = belowTop + popRect.height <= viewportH - EDGE_PX;
    const aboveTop = rect.top - GAP_PX - popRect.height;
    const placement: Position['placement'] = fitsBelow || aboveTop < EDGE_PX ? 'below' : 'above';
    const top = placement === 'below' ? Math.min(belowTop, viewportH - popRect.height - EDGE_PX) : aboveTop;

    setPosition((prev) =>
      prev && prev.top === top && prev.left === left && prev.placement === placement
        ? prev
        : { top, left, placement }
    );
  }, [anchorEl]);

  useLayoutEffect(() => {
    if (!visible || !anchorEl) {
      return;
    }
    reposition();
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    return () => {
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
    };
  }, [visible, anchorEl, reposition, text]);

  const hideDelay = autoHideMs ?? (kind === 'error' ? undefined : DEFAULT_SUCCESS_HIDE_MS);
  useEffect(() => {
    if (!visible || hideDelay === undefined) {
      return;
    }
    const timer = window.setTimeout(onClose, hideDelay);
    return () => window.clearTimeout(timer);
  }, [visible, hideDelay, onClose, text]);

  useEffect(() => {
    if (!visible) {
      return;
    }
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [visible, onClose]);

  if (!visible) {
    return null;
  }

  const body = (
    <>
      <KindIcon kind={kind} />
      <span className="min-w-0 flex-1 break-words leading-snug">{text}</span>
      <button
        type="button"
        aria-label="关闭提示"
        onClick={onClose}
        className="btn-interactive -mr-1 p-0.5 rounded-md cursor-pointer shrink-0"
        style={{ color: 'var(--text-muted)' }}
      >
        <X className="w-3 h-3" />
      </button>
    </>
  );

  if (!anchorEl || typeof document === 'undefined') {
    return (
      <div
        role={kind === 'error' ? 'alert' : 'status'}
        data-testid="action-notice"
        data-kind={kind}
        className="flex items-start gap-2 px-3 py-2 text-[12px] rounded-lg"
        style={{
          backgroundColor: 'var(--bg-elevated)',
          border: '1px solid var(--border-subtle)',
          color: kindColor(kind),
        }}
      >
        {body}
      </div>
    );
  }

  return createPortal(
    <div
      ref={popoverRef}
      role={kind === 'error' ? 'alert' : 'status'}
      data-testid="action-notice"
      data-kind={kind}
      data-placement={position?.placement ?? 'below'}
      className="liquid-popover fixed z-[80] flex items-start gap-2 px-3 py-2 text-[12px] max-w-[min(420px,calc(100vw-16px))]"
      style={{
        top: position?.top ?? -9999,
        left: position?.left ?? -9999,
        color: kindColor(kind),
        borderColor: kind === 'error' ? 'var(--accent-rose)' : undefined,
      }}
    >
      {body}
    </div>,
    document.body
  );
}
