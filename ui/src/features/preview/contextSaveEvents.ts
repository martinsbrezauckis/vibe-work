/**
 * Routes a native browser context-menu click (Save to vault, Point Vibe at this) to the visible Browser panel by
 * the page's id only: the session id for its app preview, the page key for a web page.
 * Depends on: the shell's single host-event subscription. Called by: useSessionWiring, SaveVault and PreviewTab.
 * Invariant: neither chosen page data nor a choice handle crosses this event channel.
 */
const listeners = new Set<(sessionId: string) => void>();

/** Publishes a user-click signal from the host to the mounted Browser panel. */
export function publishContextSave(sessionId: string): void {
  for (const listener of listeners) listener(sessionId);
}

/** Subscribes the Browser panel to native context-menu clicks. */
export function onContextSave(listener: (sessionId: string) => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

const pointListeners = new Set<(sessionId: string) => void>();

/** Publishes a Point Vibe at this click in a page's own menu to the mounted Browser panel. */
export function publishContextPoint(sessionId: string): void {
  for (const listener of pointListeners) listener(sessionId);
}

/** Subscribes the Browser panel to Point Vibe at this clicks in the page's own menu. */
export function onContextPoint(listener: (sessionId: string) => void): () => void {
  pointListeners.add(listener);
  return () => {
    pointListeners.delete(listener);
  };
}
