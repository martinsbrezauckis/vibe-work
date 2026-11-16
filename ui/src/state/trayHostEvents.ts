/**
 * Routes the native tray host's coming and going (a Linux StatusNotifier host) from the shell's single host-event
 * subscription to the close-to-tray setting, which reads its row again instead of asking on a timer.
 * Depends on: nothing. Called by: useSessionWiring (publishes) and useCloseToTray (listens).
 */
const listeners = new Set<() => void>();

/** Tells every mounted close-to-tray setting that the tray host came or went. */
export function publishTrayHost(): void {
  for (const listener of listeners) listener();
}

/** Calls `listener` whenever the tray host comes or goes; returns the unsubscribe. */
export function onTrayHost(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
