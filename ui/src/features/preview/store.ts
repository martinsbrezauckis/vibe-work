/**
 * The Browser tab's controller: per-session lists from the core (the preview, the servers it may open,
 * leased ports, busy-port explanations) and every action as a typed `preview` command. Components render this
 * store and call these actions only.
 * Depends on: state/coreStore (`run`, `listen`), state/createStore, protocol types.
 * Called by: register.tsx, PreviewTab and its parts, UI tests.
 * Invariants:
 * - Nothing here loads a URL the core did not accept: refusals come back as errors and are shown as text.
 * - A `previewChanged` event reloads a session the store already shows (a `null` session reloads them all).
 * - `layout` calls are serialized and only the newest one is sent (a resize sends many).
 */
import { toCallError } from "../../protocol/client/types";
import type {
  BusyPortView,
  BrowserOverlayAction,
  DeviceWidth,
  PreviewRequest,
  PreviewResult,
  PreviewTarget,
  PreviewView,
  Rect,
} from "../../protocol/core";
import type { CoreController } from "../../state/coreStore";
import { createStore } from "../../state/createStore";
import { pruneSessions } from "../../state/prune";

/** One session's list as last read. */
export interface PreviewList {
  preview: PreviewView | null;
  targets: PreviewTarget[];
  leasedPorts: number[];
  busy: BusyPortView[];
}

/** The tab's state. */
export interface PreviewUi {
  lists: Readonly<Record<string, PreviewList>>;
  busy: Readonly<Record<string, boolean>>;
  error: Readonly<Record<string, string | null>>;
}

const initial: PreviewUi = { lists: {}, busy: {}, error: {} };

function put<T>(map: Readonly<Record<string, T>>, key: string, value: T): Record<string, T> {
  return { ...map, [key]: value };
}

/** The selected preview panel; a null rectangle parks it under a modal without dropping panel ownership. */
export type Shown = { sessionId: string; tabId?: string; rect: Rect | null } | null;

function sameShown(a: Shown, b: Shown): boolean {
  if (a === null || b === null) return a === b;
  if (a.rect === null || b.rect === null)
    return a.sessionId === b.sessionId && a.tabId === b.tabId && a.rect === b.rect;
  return (
    a.sessionId === b.sessionId &&
    a.tabId === b.tabId &&
    a.rect.x === b.rect.x &&
    a.rect.y === b.rect.y &&
    a.rect.width === b.rect.width &&
    a.rect.height === b.rect.height
  );
}

/** Creates the controller for `core`. */
export function createPreviews(core: CoreController) {
  const store = createStore<PreviewUi>(initial);

  const run = async (request: PreviewRequest): Promise<PreviewResult> => {
    const result = await core.run({ type: "preview", request });
    if (result.type !== "preview") throw new Error(`unexpected ${result.type} result`);
    return result.result;
  };
  const setBusy = (sessionId: string, busy: boolean) => {
    store.set((s) => ({ ...s, busy: put(s.busy, sessionId, busy) }));
  };
  const setError = (sessionId: string, error: string | null) => {
    store.set((s) => ({ ...s, error: put(s.error, sessionId, error) }));
  };
  const setPreview = (sessionId: string, preview: PreviewView | null) => {
    store.set((s) => {
      const list = s.lists[sessionId];
      if (list === undefined) return s;
      return { ...s, lists: put(s.lists, sessionId, { ...list, preview }) };
    });
  };

  /** Runs `request` for `sessionId` with the busy flag; failures become the session's error text. */
  const act = async (sessionId: string, request: PreviewRequest): Promise<PreviewResult | null> => {
    setBusy(sessionId, true);
    try {
      const result = await run(request);
      setError(sessionId, null);
      if (result.type === "preview") setPreview(sessionId, result.preview);
      return result;
    } catch (thrown) {
      setError(sessionId, toCallError(thrown).error.message);
      return null;
    } finally {
      setBusy(sessionId, false);
    }
  };

  let shown: Shown = null;
  let sent: Shown | undefined;
  let sending = false;
  const flushLayout = async (): Promise<void> => {
    if (sending) return;
    sending = true;
    try {
      while (sent === undefined || !sameShown(sent, shown)) {
        const target = shown;
        sent = target;
        try {
          await run(
            target === null
              ? { op: "layout" }
              : {
                  op: "layout",
                  sessionId: target.sessionId,
                  ...(target.tabId === undefined ? {} : { tabId: target.tabId }),
                  ...(target.rect === null ? {} : { rect: target.rect }),
                },
          );
        } catch (thrown) {
          if (target !== null) setError(target.sessionId, toCallError(thrown).error.message);
        }
      }
    } finally {
      sending = false;
    }
  };

  const controller = {
    store,
    trackAction<T>(work: Promise<T>): Promise<T> {
      return core.trackAction(work);
    },
    /** Temporary chrome overlay visibility; its native page rectangle remains unchanged. */
    async browserOverlay(sessionId: string, action: BrowserOverlayAction, tabId?: string) {
      try {
        const result = await run({
          op: "browserOverlay",
          sessionId,
          ...(tabId === undefined ? {} : { tabId }),
          action,
        });
        if (result.type !== "overlay") throw new Error(`unexpected ${result.type} result`);
        return result;
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
        throw thrown;
      }
    },
    /** Reads a session's list (preview, servers, leased ports, busy ports). The last action's error stays shown. */
    async load(sessionId: string): Promise<void> {
      try {
        const result = await run({ op: "list", sessionId });
        if (result.type !== "previews") return;
        store.set((s) => ({
          ...s,
          lists: put(s.lists, sessionId, {
            preview: result.preview,
            targets: result.targets,
            leasedPorts: result.leasedPorts,
            busy: result.busy,
          }),
        }));
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      }
    },
    /** Opens a port (or URL) of the session in its preview. */
    async open(sessionId: string, target: { port: number } | { url: string }): Promise<void> {
      await act(sessionId, { op: "open", sessionId, ...target });
      await controller.load(sessionId);
    },
    back: (sessionId: string) => act(sessionId, { op: "back", sessionId }),
    forward: (sessionId: string) => act(sessionId, { op: "forward", sessionId }),
    reload: (sessionId: string) => act(sessionId, { op: "reload", sessionId }),
    setWidth: (sessionId: string, width: DeviceWidth) =>
      act(sessionId, { op: "setWidth", sessionId, width }),
    /** Closes the preview (its URL is forgotten). */
    async close(sessionId: string): Promise<void> {
      await act(sessionId, { op: "close", sessionId });
      await controller.load(sessionId);
    },
    /** Stops the holder of a busy port (one of our sessions' processes; the core checks again). */
    async stopOwner(sessionId: string, port: number): Promise<void> {
      await act(sessionId, { op: "stopOwner", sessionId, port });
      await controller.load(sessionId);
    },
    /** Tells the core where the preview is shown now (only the newest value is sent). */
    layout(next: Shown): void {
      shown = next;
      void flushLayout();
    },
    /** Stops following core events. */
    stop: () => {
      stopListening();
    },
  };

  // a deleted session's entries go with it.
  core.onSessionsGone((ids) => {
    store.set((s) => pruneSessions(s, ["lists", "busy", "error"], ids));
  });
  const stopListening = core.listen((event) => {
    // after `lagged` the core's events for this were missed: read again what is shown.
    if (event.type !== "previewChanged" && event.type !== "lagged") return;
    const lists = store.get().lists;
    const ids =
      event.type === "lagged" || event.sessionId === null ? Object.keys(lists) : [event.sessionId];
    for (const id of ids) if (lists[id] !== undefined) void controller.load(id);
  });

  return controller;
}

/** The controller type. */
export type Previews = ReturnType<typeof createPreviews>;
