/**
 * Effects that connect the session list to the outside world: OS notifications, the "mark the open
 * session seen" watcher, the "which session is shown" hint for the core, the refresh when the window regains focus, the notification-click events
 * from the host, the search that loads older pages, and the `sessions` / `notifications` UI routes. The
 * project groups' store follows the core (`groups.start`) and the palette gets the project entries.
 * Depends on: AppContext, hooks, state/*. Called by: Shell (through `useShellWiring`).
 * Invariants: every registration/timer is undone on unmount (StrictMode mounts effects twice);
 * older-page search waits 150 ms after typing; the search loads at most
 * `SEARCH_AUTOLOAD_PAGES` pages on its own per query.
 */
import { useEffect, useRef } from "react";

import { SEARCH_AUTOLOAD_PAGES } from "../../state/coreStore";
import { documentFocused } from "../../state/notifyStore";
import { publishContextPoint, publishContextSave } from "../preview/contextSaveEvents";
import { SEARCH_DEBOUNCE_MS } from "../search/useCoreSearch";
import { useApp } from "../shell/AppContext";
import { useCoreState, useShellState } from "../shell/hooks";
import { registerProjectCommands } from "./projectCommands";
import { registerSessionRoutes } from "./sessionRoutes";

/** Installs the wiring for the lifetime of the component. */
export function useSessionWiring(): void {
  const app = useApp();
  const { core, sessions, notifier, shell, toasts } = app;
  const filter = useShellState((s) => s.filter);
  const nextCursor = useCoreState((s) => s.index.nextCursor);
  const loading = useCoreState((s) => s.index.loading);
  const loaded = useRef(0);

  const focused = () => notifier.store.get().focusOverride ?? documentFocused();

  useEffect(() => {
    if (core === null) return;
    return notifier.watch(
      core,
      () => shell.store.get().selected,
      toasts.record,
      (id) => {
        toasts.forget(id);
      },
      (id, spec) => {
        toasts.update(id, spec);
      },
    );
  }, [core, notifier, shell, toasts]);

  useEffect(() => sessions?.watchSeen(focused), [sessions]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => sessions?.watchViewed(), [sessions]);

  useEffect(
    () =>
      core?.onSessionsGone((ids) => {
        const selected = shell.store.get().selected;
        if (selected !== null && ids.includes(selected)) shell.actions.selectSession(null);
      }),
    [core, shell],
  );

  useEffect(() => {
    const onFocus = () => {
      void core?.refreshIfStale();
      const open = shell.store.get().selected;
      if (open !== null) void sessions?.markSeen(open);
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
    };
  }, [core, sessions, shell]);

  useEffect(() => {
    if (core === null) return;
    let live = true;
    let stop: (() => void) | null = null;
    void core.client
      .onHostEvent((event) => {
        if (event.type === "browserSaveContext") publishContextSave(event.sessionId);
        else if (event.type === "browserPointContext") publishContextPoint(event.sessionId);
        else if (event.type === "openUpdates") shell.actions.openSettings("updates");
        else {
          void (async () => {
            await sessions?.open(event.sessionId);
            const retained = toasts.history
              .get()
              .some(
                (notice) =>
                  notice.id === event.notificationId && notice.sessionId === event.sessionId,
              );
            if (retained) toasts.open(event.notificationId);
            else toasts.openSession(event.sessionId);
          })();
        }
      })
      .then((off) => {
        if (live) stop = off;
        else off();
      })
      .catch(() => undefined);
    return () => {
      live = false;
      stop?.();
    };
  }, [core, sessions, shell, toasts]);

  // A search covers every page: while words are typed and Vibe has more pages, load them (capped;
  // beyond the cap the list offers a "Load more" button).
  useEffect(() => {
    if (filter.trim() === "") {
      loaded.current = 0;
      return;
    }
    if (
      core === null ||
      nextCursor === null ||
      loading ||
      loaded.current >= SEARCH_AUTOLOAD_PAGES
    ) {
      return;
    }
    const timer = setTimeout(() => {
      // Read the current query even if clear and this timer share an event-loop turn.
      if (shell.store.get().filter !== filter) return;
      loaded.current += 1;
      void core.loadMore();
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [core, shell, filter, nextCursor, loading]);

  useEffect(() => registerSessionRoutes(app), [app]);
  useEffect(() => app.projects?.groups.start(), [app.projects]);
  useEffect(() => registerProjectCommands(app), [app]);
}
