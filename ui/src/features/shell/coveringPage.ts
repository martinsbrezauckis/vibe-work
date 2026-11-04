/** The page covering the mounted work view. Reads existing page stores; Content, Home and Debug API
 * share this precedence and never keep a second copy of whether the work view is covered.
 */
import { useSyncExternalStore } from "react";
import { type App, useApp } from "./AppContext";

/** The visible page, or null when the work view is uncovered. */
export function coveringPage(app: App): "settings" | "projects" | "tools" | "libraries" | null {
  if (app.shell.store.get().settings.open) return "settings";
  if (app.projects?.store.get().open) return "projects";
  if (app.connectors?.store.get().open) return "tools";
  if (app.libraries?.store.get().open) return "libraries";
  return null;
}

/** Subscribes to every writer of the covering page; the snapshot is a stable primitive. */
export function useCoveringPage(): ReturnType<typeof coveringPage> {
  const app = useApp();
  return useSyncExternalStore(
    (listener) => {
      const stops = [
        app.shell.store.subscribe(listener),
        app.projects?.store.subscribe(listener),
        app.connectors?.store.subscribe(listener),
        app.libraries?.store.subscribe(listener),
      ];
      return () => {
        for (const stop of stops) stop?.();
      };
    },
    () => coveringPage(app),
  );
}
