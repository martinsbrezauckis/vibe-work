/**
 * The core controller: owns the core model store, feeds it from the client (event stream,
 * status, session listing) and exposes the one `run` entry point components use to send commands.
 * Components read `store` only.
 * Depends on: protocol/client types, coreModel, notifications. Called by: main.tsx, the shell.
 * Invariants:
 * - Subscribe first, then read status and the listing, so no event between the reads is missed.
 * - A startup failure is data (`server.phase = "unavailable"` with the reason), never an
 *   exception: the window always renders.
 * - Re-reads after `lagged` or a server (re)start are coalesced: one in flight at a time.
 * - The session index (`sessionIndex.ts`): a refresh re-reads as many `session/list` pages as the
 *   index covers (at least one) and applies them in one step, so the list never flickers and a
 *   deleted session disappears once the listing is complete. There is no polling timer: refreshes
 *   happen at start, after a server (re)start, after `lagged`, when the window regains focus
 *   (`refreshIfStale`, at most every `FOCUS_REFRESH_MIN_MS`), and after the app's own actions.
 */
import type { TraceOutcome } from "./commandTrace";
import type { Command, CommandResult, CoreEvent, JsonValue } from "../protocol/core";
import {
  CoreCallError,
  type CoreClient,
  type ExecuteOptions,
  toCallError,
} from "../protocol/client/types";
import {
  type CoreState,
  applyEvent,
  applyResult,
  applyNotInstalled,
  applyUnavailable,
  notInstalled,
  initialCore,
} from "./coreModel";
import { createStore } from "./createStore";
import { type ToastSpec, notificationFor } from "./notifications";
import {
  type ListingPass,
  applyListingPass,
  indexFailed,
  indexLoading,
  patchSessionFields,
  removeSession,
  setIncludeArchived,
} from "./sessionIndex";

/** Sessions per `session/list` page (Vibe allows up to 500). */
export const PAGE_SIZE = 100;
/** The window regaining focus refreshes the list only when the last refresh is older than this. */
const FOCUS_REFRESH_MIN_MS = 5000;
/** Delay before the refresh that follows a turn on a local-only session. */
export const LOCAL_REFRESH_DELAY_MS = 1500;
/** Most pages the search loads on its own for one query (the rest is a "load more" click). */
export const SEARCH_AUTOLOAD_PAGES = 10;

/** The running controller. */
export interface CoreController {
  /** The model components read. */
  readonly store: ReturnType<typeof createStore<CoreState>>;
  /** The transport (the shell registers UI-control handling on it). */
  readonly client: CoreClient;
  /** Subscribes and does the first reads. Resolves when the model has its first state. */
  start(): Promise<void>;
  /** Re-reads status and the session listing. */
  refresh(): Promise<void>;
  /** Refreshes unless a refresh ran within `FOCUS_REFRESH_MIN_MS` (window focus uses this). */
  refreshIfStale(): Promise<void>;
  /** Loads the next `session/list` page, if there is one. */
  loadMore(): Promise<void>;
  /** Turns the archive filter on or off; Vibe is asked for archived sessions only while on. */
  setIncludeArchived(include: boolean): Promise<void>;
  /** Sets fields of a session after a successful row action (see `patchSessionFields`). */
  patchSession(id: string, fields: Parameters<typeof patchSessionFields>[2]): void;
  /** Drops a deleted session from the model. */
  forget(id: string): void;
  /** Keeps a reply's runtime. Only an accepted access switch supplies a new display mode. */
  setRuntime(id: string, runtime: JsonValue, accessMode?: string): void;
  /**
   * Sends a command; the result is folded into the model. Rejects with `CoreCallError`. Every command is the
   * window's; `options.origin: "window"` only keeps it out of a served Debug API action's reply wait.
   */
  run(command: Command, options?: ExecuteOptions): Promise<CommandResult>;
  /** Keeps a UI-control action's request open through preparation and lazy-loading awaits. */
  trackAction<T>(action: Promise<T>): Promise<T>;
  /** Stops listening. */
  stop(): void;
  /**
   * Adds a listener for every core event (after the model applied it); returns the function that
   * removes it. Feature stores that keep their own state (terminals) use it.
   */
  listen(listener: (event: CoreEvent) => void): () => void;
  /**
   * Calls `listener` with the ids of sessions that left the model (a delete, here or in the
   * CLI), once each, so feature stores drop what they keep per session. Not while the list is loading (a page that
   * does not list a session yet is not a delete). Returns the function that stops it.
   */
  onSessionsGone(listener: (ids: readonly string[]) => void): () => void;
}

/**
 * Creates the controller. `notify` receives the toasts the events call for. `serving` (the UI-control
 * registry's `servingAction`) gives the page-local key of the Debug API action being served, `null` when none is;
 * `track` (the registry's `track`) records each command sent meanwhile, so the action's reply waits for it.
 * `observe` captures dispatch metadata and returns the callback that records this call's completion.
 */
export function createCore(
  rawClient: CoreClient,
  notify: (toast: ToastSpec) => void,
  serving: () => string | null = () => null,
  track: <T>(action: string, promise: Promise<T>) => Promise<T> = (_action, promise) => promise,
  observe: (
    command: Command,
  ) => ((outcome: Exclude<TraceOutcome, { status: "pending" }>) => void) | undefined = () =>
    undefined,
  trackAction: <T>(action: string, promise: Promise<T>) => Promise<T> = track,
): CoreController {
  // Every command goes out as the window's, also while a Debug API UI-control request is served: the page holds
  // no Debug API authority (the host grants the few owner-only effects a request promises from the request itself,
  // `crates/vw-host/src/ui_grants.rs`). While an action is served, its commands, including those features send
  // through `core.client` directly (the terminal store), are recorded so its reply follows their answers.
  // A view of the client that differs only in `execute` (everything else is looked up on the client
  // itself at call time, so a later change to it is seen).
  const client = Object.create(rawClient) as CoreClient & { execute: CoreClient["execute"] };
  const dispatch: CoreClient["execute"] = (command, options) => {
    // What the view sends by itself (a terminal's focus report when a Debug API request moves focus) is not part
    // of the action.
    if (options?.origin === "window") return rawClient.execute(command, options);
    const action = serving();
    return action === null ? rawClient.execute(command) : track(action, rawClient.execute(command));
  };
  client.execute = async (command, options) => {
    const complete = observe(command);
    try {
      const result = await dispatch(command, options);
      complete?.({ status: "succeeded" });
      return result;
    } catch (thrown) {
      complete?.({ status: "failed", code: toCallError(thrown).error.code });
      throw thrown;
    }
  };
  const store = createStore<CoreState>(initialCore);
  const listeners = new Set<(event: CoreEvent) => void>();
  let unsubscribe: (() => void) | null = null;
  let reading: Promise<void> | null = null;

  const fold = (result: CommandResult): void => {
    store.set((state) => applyResult(state, result));
  };
  const fail = (thrown: unknown): void => {
    const error = toCallError(thrown).error;
    if (notInstalled(error)) store.set((state) => applyNotInstalled(state, error));
    else if (error.code === "serverUnavailable")
      store.set((state) => applyUnavailable(state, error));
    else notify({ kind: "error", bellKind: "failure", text: error.message });
  };

  /** Fetches `pages` pages from `cursor` (undefined = the first). */
  const fetchPages = async (pages: number, cursor?: string): Promise<ListingPass> => {
    const include = store.get().index.includeArchived;
    const rows: ListingPass["rows"][number][] = [];
    let next: string | null = cursor ?? null;
    let continueSessionId: string | null = null;
    let done = 0;
    do {
      const result = await client.execute({
        type: "listSessions",
        limit: PAGE_SIZE,
        ...(next === null ? {} : { cursor: next }),
        ...(include ? { includeArchived: true } : {}),
      });
      if (result.type !== "sessions") throw new Error("unexpected listing result");
      rows.push(...result.rows);
      if (done === 0) continueSessionId = result.continueSessionId;
      next = result.nextCursor;
      done += 1;
    } while (next !== null && done < pages);
    return {
      rows,
      nextCursor: next,
      ...(cursor === undefined ? { continueSessionId } : {}),
      pages: done,
      refresh: cursor === undefined,
    };
  };
  const list = async (pages: number, cursor?: string): Promise<void> => {
    store.set((state) => indexLoading(state, true));
    try {
      const pass = await fetchPages(pages, cursor);
      store.set((state) => applyListingPass(state, pass));
    } catch (thrown) {
      const error = toCallError(thrown).error;
      // Before Vibe is installed there is nothing to list yet; that is not an error (the list
      // loads when the server starts: the `running` event below refreshes).
      if (notInstalled(error)) store.set((state) => indexLoading(state, false));
      else store.set((state) => indexFailed(state, error.message));
      fail(thrown);
    }
  };
  let refreshedAt = 0;
  const read = async (): Promise<void> => {
    try {
      fold(await client.execute({ type: "readStatus" }));
    } catch (thrown) {
      fail(thrown);
      return;
    }
    // The session grants are kept in the core's memory; after a reload the window reads them again. A core
    // without them (an older test double) leaves the list empty.
    try {
      fold(await client.execute({ type: "askGrants", request: { op: "list" } }));
    } catch {
      // No grants to show: the chip stays hidden.
    }
    await list(Math.max(1, store.get().index.pages));
    refreshedAt = Date.now();
  };
  const refresh = (): Promise<void> => {
    reading ??= read().finally(() => {
      reading = null;
    });
    return reading;
  };

  // A session we created is "local" until Vibe lists it, which happens once a turn has run. A turn
  // that ends on a local-only session therefore schedules one refresh (debounced, no timer loop).
  let localTimer: ReturnType<typeof setTimeout> | null = null;
  const refreshSoon = (): void => {
    localTimer ??= setTimeout(() => {
      localTimer = null;
      void refresh();
    }, LOCAL_REFRESH_DELAY_MS);
  };
  const onEvent = (event: Parameters<typeof applyEvent>[1]): void => {
    store.set((state) => applyEvent(state, event));
    if (event.type === "change" && event.change.kind === "turnUpserted") {
      if (store.get().sessions[event.change.sessionId]?.localOnly === true) refreshSoon();
    }
    const toast = notificationFor(event, store.get());
    if (toast !== null) notify(toast);
    if (event.type === "lagged" || (event.type === "server" && event.status.state === "running")) {
      void refresh();
    }
    for (const listener of [...listeners]) listener(event);
  };

  return {
    store,
    client,
    trackAction<T>(work: Promise<T>): Promise<T> {
      const action = serving();
      return action === null ? work : trackAction(action, work);
    },
    async start() {
      try {
        unsubscribe = await client.subscribe(onEvent);
      } catch (thrown) {
        fail(thrown);
        return;
      }
      await refresh();
    },
    refresh,
    refreshIfStale() {
      return Date.now() - refreshedAt < FOCUS_REFRESH_MIN_MS ? Promise.resolve() : refresh();
    },
    async loadMore() {
      const { nextCursor, loading } = store.get().index;
      if (nextCursor === null || loading) return;
      await list(1, nextCursor);
    },
    async setIncludeArchived(include) {
      if (store.get().index.includeArchived === include) return;
      store.set((state) => setIncludeArchived(state, include));
      await refresh();
    },
    patchSession(id, fields) {
      store.set((state) => patchSessionFields(state, id, fields));
    },
    forget(id) {
      store.set((state) => removeSession(state, id));
    },
    setRuntime(id, runtime, accessMode) {
      store.set((state) => {
        const entry = state.sessions[id];
        return entry === undefined
          ? state
          : {
              ...state,
              sessions: {
                ...state.sessions,
                [id]: { ...entry, runtime, ...(accessMode === undefined ? {} : { accessMode }) },
              },
            };
      });
    },
    async run(command, options) {
      try {
        const result = await client.execute(command, options);
        fold(result);
        return result;
      } catch (thrown) {
        throw thrown instanceof CoreCallError ? thrown : toCallError(thrown);
      }
    },
    stop() {
      if (localTimer !== null) clearTimeout(localTimer);
      localTimer = null;
      unsubscribe?.();
      unsubscribe = null;
    },
    listen(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    onSessionsGone(listener) {
      let known = new Set(Object.keys(store.get().sessions));
      return store.subscribe(() => {
        const state = store.get();
        if (state.index.loading) return;
        const gone = [...known].filter((id) => !(id in state.sessions));
        known = new Set(Object.keys(state.sessions));
        if (gone.length > 0) listener(gone);
      });
    },
  };
}
