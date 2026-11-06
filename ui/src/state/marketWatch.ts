/** Tools inventory invalidation: visible-page fallback and project/file events, independent of chat streaming. */
import type { CoreController } from "./coreStore";
import { contextSession, type Connectors } from "./connectors";
import type { Shell } from "./shellStore";
import type { MarketState } from "./market";
import type { Store } from "./createStore";

export function watchMarket(
  core: CoreController,
  shell: Shell,
  connectors: Connectors,
  market: Store<MarketState>,
  reload: (allowed: () => boolean) => Promise<void>,
): () => void {
  let stopped = false;
  let queued: ReturnType<typeof setTimeout> | null = null;
  let fallback: ReturnType<typeof setInterval> | null = null;
  const visible = () => !stopped && connectors.store.get().open;
  const cancel = () => {
    if (queued !== null) clearTimeout(queued);
    queued = null;
  };
  const schedule = () => {
    if (!visible() || queued !== null) return;
    queued = setTimeout(() => {
      queued = null;
      if (visible()) void reload(visible);
    }, 300);
  };
  const context = () => {
    const id = contextSession(core, shell);
    const session = id === null ? undefined : core.store.get().sessions[id]?.session;
    return JSON.stringify([
      id,
      shell.store.get().newFolder,
      session?.["cwd"],
      session?.["workspaceRoots"],
    ]);
  };
  let previous = context();
  const contextChanged = () => {
    const current = context();
    if (current === previous) return;
    previous = current;
    schedule();
  };
  const visibility = () => {
    if (visible()) {
      if (fallback === null) {
        schedule();
        // Home/plugin folders can change outside the project watcher. Read at most once a minute while shown.
        fallback = setInterval(schedule, 60_000);
      }
    } else {
      cancel();
      if (fallback !== null) clearInterval(fallback);
      fallback = null;
    }
  };
  const stopPage = connectors.store.subscribe(visibility);
  const stopShell = shell.store.subscribe(contextChanged);
  const stopMarket = market.subscribe(() => {
    // A direct page load or app mutation already reads the inventory scheduled for this context.
    if (market.get().loading) cancel();
  });
  const stopCore = core.listen((event) => {
    if (event.type === "server" || event.type === "lagged") schedule();
    else if (event.type === "changesUpdated" || event.type === "filesChanged") {
      if (event.sessionId === null || event.sessionId === contextSession(core, shell)) schedule();
    } else if (event.type === "change" || event.type === "snapshot") contextChanged();
  });
  visibility();
  return () => {
    stopped = true;
    visibility();
    stopPage();
    stopShell();
    stopMarket();
    stopCore();
  };
}
