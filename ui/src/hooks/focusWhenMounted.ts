/**
 * One place for every deferred focus move: a target that appears after the current render (a row the core's event
 * re-renders, a virtual transcript row scrolled into view, a lazily mounted explorer or plugin card) is looked up on
 * each animation frame until it exists, then focused. The wait is bounded; a target that never appears runs the
 * request's fallback. Only one request is pending at a time: a newer one replaces it, so two deferred moves never
 * fight over focus, and the returned function cancels it (call it when the component that asked unmounts).
 */

export interface FocusRequest {
  /** Looks the target up again on each frame; `false` means it will not appear (the fallback runs at once). */
  find: () => HTMLElement | null | undefined | false;
  /** Frames to wait for the target (default 30, about half a second). */
  frames?: number;
  /** Scrolls the target to the middle of its scroll area before it takes focus. */
  center?: boolean;
  /** Focus without the browser's own scroll into view (the caller already placed the target). */
  preventScroll?: boolean;
  /** Runs on the target just before it takes focus. */
  prepare?: (target: HTMLElement) => void;
  /** Runs when the target never appeared within `frames`. */
  fallback?: () => void;
  /** Runs once the request settled (focused or fell back), never after a cancel. */
  settled?: () => void;
}

const DEFAULT_FRAMES = 30;
let pending: { cancel: () => void } | null = null;

/** Starts `request`, replacing any pending one; returns its cancel. */
export function focusWhenMounted(request: FocusRequest): () => void {
  pending?.cancel();
  let frame = 0;
  let left = request.frames ?? DEFAULT_FRAMES;
  let live = true;
  const entry = {
    cancel: () => {
      live = false;
      window.cancelAnimationFrame(frame);
      if (pending === entry) pending = null;
    },
  };
  const settle = () => {
    if (pending === entry) pending = null;
    live = false;
    request.settled?.();
  };
  const step = () => {
    if (!live) return;
    const target = request.find();
    if (target) {
      if (request.center === true) target.scrollIntoView({ block: "center" });
      request.prepare?.(target);
      target.focus({ preventScroll: request.preventScroll === true });
      settle();
      return;
    }
    left -= 1;
    if (target === false || left <= 0) {
      request.fallback?.();
      settle();
      return;
    }
    frame = window.requestAnimationFrame(step);
  };
  pending = entry;
  frame = window.requestAnimationFrame(step);
  return entry.cancel;
}
