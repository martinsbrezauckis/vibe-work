/**
 * Asks an AskCard for the keyboard focus from anywhere ("Go to the waiting prompt", Ctrl/Cmd+Shift+A). The card may
 * not be in the page yet: its session is opening, or the transcript's list has not drawn that row. A request then waits
 * until the card registers, and the transcript that holds the row scrolls it into view.
 * Depends on: nothing. Called by: waiting.ts (requests), AskCard (registers), Transcript (scrolls to a waiting
 * request).
 * Invariants: one pending request at a time (the newest wins); a request is served once and lapses after
 * `PENDING_MS`; registering returns its own removal.
 */

/** How long a request waits for its card (ms): a session switch and a scroll fit well inside. */
const PENDING_MS = 5000;

const focusers = new Map<string, () => void>();
let pending: { id: string; at: number } | null = null;
const listeners = new Set<(id: string) => void>();

/** Focuses card `id` now, or as soon as it is drawn. */
export function requestCardFocus(id: string): void {
  const focus = focusers.get(id);
  if (focus !== undefined) {
    pending = null;
    focus();
    return;
  }
  pending = { id, at: Date.now() };
  for (const listener of [...listeners]) listener(id);
}

/** The card a request waits for, if any (and not lapsed). */
export function pendingCardFocus(): string | null {
  if (pending === null || Date.now() - pending.at > PENDING_MS) return null;
  return pending.id;
}

/** Tells `listener` the card id of every request that has to wait. Returns the removal. */
export function onCardFocusRequest(listener: (id: string) => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Registers card `id`; a waiting request for it is served now. Returns the removal. */
export function registerCardFocus(id: string, focus: () => void): () => void {
  focusers.set(id, focus);
  if (pendingCardFocus() === id) {
    pending = null;
    focus();
  }
  return () => {
    if (focusers.get(id) === focus) focusers.delete(id);
  };
}
