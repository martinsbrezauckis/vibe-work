/**
 * The Tools page's Marketplace tab: the view from the core's
 * `marketplace` command, the tab, filter, search and selection, the install in progress (from `marketInstall`
 * events), and the actions: install / update, stop, remove, and add a catalog MCP server (through the Connectors
 * flow, so sign-in works exactly as for a server added by address).
 * Depends on: coreStore (`run`, `listen`), connectors (server add + sign-in, the context session), shellStore,
 * createStore. Called by: createApp, the marketplace views, the `ui/marketplace` route.
 * Invariants: every command a Debug API route triggers carries the Debug API origin (`actions("debug-api")`);
 * an install from the card's Install… shows the core's offer first (`installOffers.ts`) and the detail's
 * Install sends that offer's id, so the window asks no native dialog; an update or a reinstall without an offer
 * asks the core's native dialog (the core decides, not this file); nothing here downloads or writes files.
 */
import type {
  CoreError,
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
import { createOfferFlow, isStaleOffer } from "./installOffers";
import { createPluginNotices } from "./pluginNotices";
import type { Toasts } from "./toastStore";
import type { Shell } from "./shellStore";

/** Which tab of the Tools page shows. */
export type ToolsTab = "connectors" | "marketplace" | "skills";

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
export function createMarket(deps: {
  core: CoreController;
  shell: Shell;
  connectors: Connectors;
  toasts: Toasts;
  persist: boolean;
}) {
  const { core, shell, connectors } = deps;
  const store = createStore<MarketState>(MARKET_INITIAL);
  const notify = createPluginNotices(deps.toasts, deps.persist);
  const patch = (p: Partial<MarketState>) => {
    store.set((s) => ({ ...s, ...p }));
  };
  const nameOf = (id: string) =>
    store.get().view?.entries.find((e) => e.entry.id === id)?.entry.name ?? id;

  const request = async (origin: "ui" | "debug-api", req: MarketRequest): Promise<MarketResult> => {
    const result = await core.run(
      { type: "marketplace", request: req },
      // The window facade's changes are not part of a served Debug API action (both are the window's commands).
      origin === "debug-api" || req.op === "read" ? undefined : { origin: "window" },
    );
    if (result.type !== "marketplace")
      throw new Error("unexpected answer to a marketplace request");
    return result.result;
  };

  const fail = (thrown: unknown): void => {
    const error = toCallError(thrown).error;
    // Declining the native dialog is the user's own answer: no error line. A stale offer is answered by the offer
    // block, which shows the fresh offer.
    if (error.code === "confirmationDeclined" || isStaleOffer(error)) return;
    patch({ note: error.message.charAt(0).toUpperCase() + error.message.slice(1) });
  };

  /** Starts an install or update (with the shown offer's id when there is one): null when it started. */
  const startInstall = async (
    origin: "ui" | "debug-api",
    id: string,
    offerId?: string,
  ): Promise<CoreError | null> => {
    try {
      patch({ note: null });
      const result = await request(origin, {
        op: "install",
        id,
        ...(offerId === undefined ? {} : { offerId }),
      });
      if (result.kind === "installStarted")
        patch({
          installing: { id, phase: "download", what: "", done: null, total: null, message: null },
        });
      return null;
    } catch (thrown) {
      fail(thrown);
      return toCallError(thrown).error;
    }
  };

  // The card's Install…: the offer the detail shows right above its Install button.
  const offers = createOfferFlow<"ui" | "debug-api">({
    async read(origin, id) {
      const result = await request(origin, { op: "offerInstall", id });
      if (result.kind !== "offer") throw new Error("unexpected answer to a marketplace offer");
      return result.offer;
    },
    confirm: (origin, id, _mode, offerId) =>
      store.get().installing === null
        ? startInstall(origin, id, offerId)
        : Promise.resolve("notSent"),
  });

  /** Reads the view with the context session's state. */
  const reload = async (origin: "ui" | "debug-api"): Promise<void> => {
    patch({ loading: true });
    const sessionId = contextSession(core, shell);
    const cwd = shell.store.get().newFolder;
    try {
      const result = await request(origin, {
        op: "read",
        ...(sessionId === null && cwd !== null ? { cwd } : {}),
        ...(sessionId === null ? {} : { sessionId }),
      });
      if (result.kind === "view") {
        notify(result.view.discovered);
        patch({ view: result.view, error: null, installing: result.view.installing });
      }
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

  /** Selects an entry (null closes the detail); an offer belongs to the entry it was read for. */
  const select = (id: string | null): void => {
    if (id !== store.get().selected) offers.actions("ui").clear();
    patch({ selected: id, note: null });
  };

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
      select(id);
    },
    /** Opens Tools at a discovered plugin card without changing its state. */
    async showPlugin(path: string): Promise<void> {
      patch({ tab: "marketplace", filter: "all", search: "", selected: null });
      const a = origin === "ui" ? connectors.actions : connectors.debugApi;
      if (!connectors.store.get().open) await a.open();
      await reload(origin);
      requestAnimationFrame(() => {
        const row = [...document.querySelectorAll<HTMLElement>(".local-plugin")].find(
          (node) => node.dataset.itemId === path,
        );
        row?.scrollIntoView({ block: "center" });
        row?.querySelector<HTMLButtonElement>("button")?.focus();
      });
    },
    /** Installs or updates a pack or plugin without an offer (the window asks the native dialog first). */
    async install(id: string): Promise<boolean> {
      return (await startInstall(origin, id)) === null;
    },
    /** The card's Install…: selects the card (opens its detail) and shows what Install would put here. */
    async offerInstall(id: string): Promise<void> {
      select(id);
      await offers.actions(origin).open(id, "install");
    },
    /**
     * The detail's Install: with the offer on screen it installs exactly that offer; without one (the detail was
     * opened from the card itself, or the read failed) it shows the offer first, so the click that installs is
     * always on what the user sees.
     */
    async installShown(id: string): Promise<boolean> {
      const open = offers.store.get().open[id];
      if (open?.phase === "shown") return offers.actions(origin).confirm(id);
      if (open === undefined) await offers.actions(origin).open(id, "install");
      return false;
    },
    /** Escape on the offer: closes it, focus back to the card's Install…. */
    cancelOffer(id: string): void {
      offers.actions(origin).cancel(id);
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
    /** Changes only the owned plugin folder the core resolved; Remove asks a native dialog. */
    async changePlugin(
      op: "turnOffPlugin" | "turnOnPlugin" | "removePlugin",
      path: string,
    ): Promise<boolean> {
      const sessionId = contextSession(core, shell);
      const cwd = shell.store.get().newFolder;
      const name = store.get().view?.discovered.find((p) => p.path === path)?.name ?? path;
      try {
        const result = await request(origin, {
          op,
          path,
          ...(sessionId === null ? {} : { sessionId }),
          ...(sessionId === null && cwd !== null ? { cwd } : {}),
        });
        if (result.kind === "view")
          patch({
            view: result.view,
            note:
              result.notice ??
              `${name} is ${op === "turnOffPlugin" ? "turned off" : op === "turnOnPlugin" ? "turned on" : "removed"}. New chats use the current plugins.`,
          });
        await reload(origin);
        return true;
      } catch (thrown) {
        fail(thrown);
        const error = toCallError(thrown).error;
        if (error.code !== "confirmationDeclined")
          deps.toasts.push({ kind: "error", bellKind: "failure", text: error.message });
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

  return {
    store,
    /** The open install offers (the detail's offer block and the `ui/marketplace` route read them). */
    offers: offers.store,
    actions: actions("ui"),
    debugApi: actions("debug-api"),
    /** Discovery remains active while Tools is closed; every listener is released on unmount. */
    watch(): () => void {
      let busy = false;
      let queued: ReturnType<typeof setTimeout> | null = null;
      let queuedDeadline = 0;
      let lastRead = 0;
      // A read asked for while another runs: the running one may have looked before the change that asked.
      let again = false;
      let stopped = false;
      const read = async () => {
        if (busy) {
          again = true;
          return;
        }
        busy = true;
        lastRead = Date.now();
        try {
          await reload("ui");
        } finally {
          busy = false;
          if (again && !stopped) {
            again = false;
            void read();
          }
        }
      };
      // Each read starts Vibe's plugin resolver (a Python process), so a working chat's stream of session
      // changes may trigger at most one read every few seconds; navigation and Vibe restarts stay prompt.
      const schedule = (delay = 300) => {
        const deadline = Date.now() + delay;
        if (queued !== null) {
          if (deadline >= queuedDeadline) return;
          clearTimeout(queued);
        }
        queuedDeadline = deadline;
        queued = setTimeout(() => {
          queued = null;
          queuedDeadline = 0;
          void read();
        }, delay);
      };
      const stopShell = shell.store.subscribe(() => {
        schedule();
      });
      const stopCore = core.listen((event) => {
        if (event.type === "server") schedule();
        else if (event.type === "change")
          schedule(Math.max(300, CHANGE_READ_GAP_MS - (Date.now() - lastRead)));
      });
      const interval = setInterval(() => void read(), 15_000);
      void read();
      return () => {
        stopped = true;
        clearInterval(interval);
        if (queued !== null) clearTimeout(queued);
        stopShell();
        stopCore();
      };
    },
  } as const;
}

/** Smallest gap between plugin reads caused by session changes (a turn emits many). */
const CHANGE_READ_GAP_MS = 5_000;

/** The marketplace controller. */
export type Market = ReturnType<typeof createMarket>;
