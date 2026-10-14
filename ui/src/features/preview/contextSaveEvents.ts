/**
 * Routes a native browser context-menu click to the visible save control by session id only.
 * Depends on: the shell's single host-event subscription. Called by: useSessionWiring and SaveVault.
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
