/** Connects native close events to voice cleanup and the notification preference. Called by the shell;
 * cleanup handles StrictMode's pending subscription. Native Close alone stops voice, never minimize/blur. */
import type { App } from "../features/shell/AppContext";

/** Subscribes for the shell's lifetime; notifications follow the existing user setting. */
export function startDesktopLifecycle(app: App): () => void {
  const desktop = app.core?.client.desktop;
  if (desktop === undefined) return () => undefined;
  let live = true;
  let stop: (() => void) | undefined;
  let revision = 0;
  const subscribe = () => {
    const mine = ++revision;
    void desktop
      .subscribe(() => {
        app.voice?.stopForClose();
      }, app.notifier.store.get().level !== "errors")
      .then((off) => {
        if (live && mine === revision) {
          stop?.();
          stop = off;
        } else off();
      })
      .catch(() => undefined);
  };
  subscribe();
  const unwatch = app.notifier.store.subscribe(subscribe);
  return () => {
    live = false;
    unwatch();
    stop?.();
  };
}
