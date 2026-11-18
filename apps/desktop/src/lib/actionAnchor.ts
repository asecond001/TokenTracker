/**
 * Tracks the most recently activated button so feedback (success / error
 * notices) can be rendered right next to the control that triggered it,
 * instead of in a page-level banner far away from the user's focus.
 */

let lastActionElement: HTMLElement | null = null;
let installed = false;

const ACTION_SELECTOR = 'button, [role="button"], a[href], label';

function rememberFromEvent(event: Event): void {
  const target = event.target;
  if (!(target instanceof Element)) {
    return;
  }
  const actionable = target.closest<HTMLElement>(ACTION_SELECTOR);
  if (actionable) {
    lastActionElement = actionable;
  }
}

function rememberFromKey(event: KeyboardEvent): void {
  if (event.key === 'Enter' || event.key === ' ') {
    rememberFromEvent(event);
  }
}

export function installActionAnchorTracking(): void {
  if (installed || typeof document === 'undefined') {
    return;
  }
  installed = true;
  document.addEventListener('pointerdown', rememberFromEvent, true);
  document.addEventListener('click', rememberFromEvent, true);
  document.addEventListener('keydown', rememberFromKey, true);
}

/** Returns the last activated control if it is still attached to the document. */
export function getLastActionAnchor(): HTMLElement | null {
  if (lastActionElement && lastActionElement.isConnected) {
    return lastActionElement;
  }
  return null;
}

/** Test helper. */
export function resetActionAnchor(): void {
  lastActionElement = null;
}
