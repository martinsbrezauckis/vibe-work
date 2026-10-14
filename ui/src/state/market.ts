/**
 * The Tools page's Marketplace tab: the view from the core's
 * `marketplace` command, the tab, filter, search and selection, the install in progress (from `marketInstall`
 * events), and the actions: install / update, stop, remove, and add a catalog MCP server (through the Connectors
 * flow, so sign-in works exactly as for a server added by address).
 * Depends on: coreStore (`run`, `listen`), connectors (server add + sign-in, the context session), shellStore,
 * createStore. Called by: createApp, the marketplace views, the `ui/marketplace` route.
 * Invariants: every command a Debug API route triggers carries the Debug API origin (`actions("debug-api")`);
 * the window's install asks the core's native dialog (the core decides, not this file); nothing here downloads
 * or writes files.
 */
import type {
  CoreEvent,
  InstallProgress,
  MarketRequest,
  MarketResult,
  MarketView,
} from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { MarketFilter } from "../features/marketplace/model";
import type { Connectors } from "./connectors";
import { contextSession } from "./connectors";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Shell } from "./shellStore";

/** Which tab of the Tools page shows. */
export type ToolsTab = "connectors" | "marketplace";

/** Everything the tab shows. */
export interface MarketState {
  tab: ToolsTab;
  view: MarketView | null;
  loading: boolean;
  error: string | null;
  filter: MarketFilter;
  search: string;
  /** The selected entry id. */
  selected: string | null;
  /** The install in progress (also between reads). */
  installing: InstallProgress | null;
  /** A short outcome line ("Context7 is installed."). */
  note: string | null;
}

export const MARKET_INITIAL: MarketState = {
  tab: "connectors",
  view: null,
  loading: false,
  error: null,
  filter: "all",
  search: "",
  selected: null,
  installing: null,
  note: null,
};

/** Creates the marketplace controller. */
export function createMarket(deps: { core: CoreController; shell: Shell; connectors: Connectors }) {
  const { core, shell, connectors } = deps;
  const store = createStore<MarketState>(MARKET_INITIAL);
  const patch = (p: Partial<MarketState>) => {
    store.set((s) => ({ ...s, ...p }));
  };
  const nameOf = (id: string) =>
    store.get().view?.entries.find((e) => e.entry.id === id)?.entry.name ?? id;

  const request = async (origin: "ui" | "debug-api", req: MarketRequest): Promise<MarketResult> => {
    const result = await core.run(
      { type: "marketplace", request: req },
      origin === "debug-api" ? { origin } : undefined,
    );
    if (result.type !== "marketplace")
      throw new Error("unexpected answer to a marketplace request");
    return result.result;
  };

  const fail = (thrown: unknown): void => {
    const error = toCallError(thrown).error;
    // Declining the native dialog is the user's own answer: no error line.
    if (error.code === "confirmationDeclined") return;
    patch({ note: error.message.charAt(0).toUpperCase() + error.message.slice(1) });
  };

  /** Reads the view with the context session's state. */
  const reload = async (origin: "ui" | "debug-api"): Promise<void> => {
    patch({ loading: true });
    const sessionId = contextSession(core, shell);
    try {
      const result = await request(origin, {
        op: "read",
        ...(sessionId === null ? {} : { sessionId }),
      });
      if (result.kind === "view")
        patch({ view: result.view, error: null, installing: result.view.installing });
    } catch (thrown) {
      patch({ error: toCallError(thrown).error.message });
    } finally {
      patch({ loading: false });
    }
  };

  // Install progress and outcomes arrive as events (the install runs in the core's background).
  core.listen((event: CoreEvent) => {
    if (event.type !== "marketInstall") return;
    const p = event.progress;
    if (p.phase === "installed" || p.phase === "failed" || p.phase === "cancelled") {
      const name = nameOf(p.id);
      patch({
        installing: null,
        note:
          p.phase === "installed"
            ? `${name} is installed. New sessions use it; open sessions pick it up when Vibe reloads their plugins.`
            : p.phase === "failed"
              ? `${name} was not installed: ${p.message ?? "unknown error"}`
              : `Stopped installing ${name}.`,
      });
      void reload("ui");
    } else {
      patch({ installing: p });
    }
  });

  const actions = (origin: "ui" | "debug-api") => ({
    /** Shows a tab of the Tools page (opens the page). */
    async showTab(tab: ToolsTab): Promise<void> {
      patch({ tab });
      const a = origin === "ui" ? connectors.actions : connectors.debugApi;
      if (!connectors.store.get().open) await a.open();
      if (tab === "marketplace") await reload(origin);
    },
    load: () => reload(origin),
    setFilter(filter: MarketFilter): void {
      patch({ filter });
    },
    setSearch(search: string): void {
      patch({ search });
    },
    select(id: string | null): void {
      patch({ selected: id, note: null });
    },
    /** Installs or updates a pack or plugin (the window asks the native dialog first). */
    async install(id: string): Promise<boolean> {
      try {
        patch({ note: null });
        const result = await request(origin, { op: "install", id });
        if (result.kind === "installStarted") {
          patch({
            installing: { id, phase: "download", what: "", done: null, total: null, message: null },
          });
          return true;
        }
        return false;
      } catch (thrown) {
        fail(thrown);
        return false;
      }
    },
    async cancel(): Promise<void> {
      try {
        const result = await request(origin, { op: "cancelInstall" });
        if (result.kind === "view") patch({ view: result.view, installing: null });
      } catch (thrown) {
        fail(thrown);
      }
    },
    async remove(id: string): Promise<boolean> {
      try {
        const result = await request(origin, { op: "remove", id });
        if (result.kind === "view") patch({ view: result.view });
        patch({
          note:
            result.kind === "view" && result.notice ? result.notice : `${nameOf(id)} is removed.`,
        });
        await reload(origin);
        return true;
      } catch (thrown) {
        fail(thrown);
        return false;
      }
    },
    /** Adds a catalog server through the Connectors flow (address check, dialog, sign-in in the browser). */
    async addServer(id: string): Promise<string | null> {
      const entry = store.get().view?.entries.find((e) => e.entry.id === id)?.entry;
      const url = entry?.server?.url;
      if (entry === undefined || url === undefined) return null;
      const a = origin === "ui" ? connectors.actions : connectors.debugApi;
      const name = await a.addServer(url, id);
      if (name !== null)
        patch({ note: `${entry.name} is added. Finish its sign-in in the browser.` });
      return name;
    },
    /**
     * a catalog server that needs an API key opens the Add MCP server dialog on the Connectors tab, with
     * the address and name filled in and "needs an API key" ticked (the key is chosen there; nothing is sent yet).
     */
    async addKeyServer(id: string): Promise<void> {
      const entry = store.get().view?.entries.find((e) => e.entry.id === id)?.entry;
      const url = entry?.server?.url;
      if (entry === undefined || url === undefined) return;
      patch({ tab: "connectors" });
      const a = origin === "ui" ? connectors.actions : connectors.debugApi;
      if (!connectors.store.get().open) await a.open();
      a.showAdd(true, { url, name: id, key: true });
    },
  });

  return { store, actions: actions("ui"), debugApi: actions("debug-api") } as const;
}

/** The marketplace controller. */
export type Market = ReturnType<typeof createMarket>;
