/**
 * The open session's dev servers and long commands: the UI side of the core's `processes`
 * ops `session`, `output` and `stopSession`. The window shows them in one panel with the "Left running" list.
 * Depends on: coreStore, createStore, shellStore, toastStore, protocol types. Called by: createApp, the status bar's
 * Processes button and dialog, the `ui/processes` route.
 * Invariants:
 * - The list belongs to the selected session; switching sessions reads it again (an answer for another session
 * is dropped).
 * - Live output: while the dialog is open, every process whose output is shown is read again every
 * `OUTPUT_POLL_MS`, and the list every `LIST_POLL_MS`; while it is closed the list is read every
 * `IDLE_POLL_MS` (for the button's count). Nothing is read without a selected session.
 * - Stop runs only on a click (or the route's request, the same click); failures become one toast.
 */
import type { ProcessesRequest, ProcessesResult, SessionProcessView } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";

/** How often shown output is read while the dialog is open. */
const OUTPUT_POLL_MS = 1000;
/** How often the list is read while the dialog is open. */
const LIST_POLL_MS = 2500;
/** How often the list is read while the dialog is closed (the button's count). */
const IDLE_POLL_MS = 15_000;

/** The UI state. */
export interface SessionProcessesState {
  sessionId: string | null;
  items: readonly SessionProcessView[];
  /** Output shown per process id (`null`: asked, not read yet). */
  outputs: Readonly<Record<string, string | null>>;
  stopping: readonly string[];
}

/** "npm run dev" / "pid 4242", for a row and its Stop label. */
export function processTitle(item: SessionProcessView): string {
  if (item.command !== null && item.command.trim() !== "") return item.command;
  return item.pid === null ? item.id : `pid ${String(item.pid)}`;
}

/** Creates the controller. */
export function createSessionProcesses(deps: {
  core: CoreController;
  shell: Shell;
  toasts: Toasts;
}) {
  const { core, shell, toasts } = deps;
  const store = createStore<SessionProcessesState>({
    sessionId: null,
    items: [],
    outputs: {},
    stopping: [],
  });

  const run = async (request: ProcessesRequest): Promise<ProcessesResult> => {
    const result = await core.run({ type: "processes", request });
    if (result.type !== "processes") throw new Error(`unexpected answer ${result.type}`);
    return result.result;
  };

  /** Reads the selected session's list (and drops state of another session). */
  const refresh = async (): Promise<void> => {
    const sessionId = shell.store.get().selected;
    if (sessionId !== store.get().sessionId)
      store.set(() => ({ sessionId, items: [], outputs: {}, stopping: [] }));
    if (sessionId === null) return;
    try {
      const result = await run({ op: "session", sessionId });
      if (result.kind !== "session" || shell.store.get().selected !== sessionId) return;
      const live = new Set(result.items.map((i) => i.id));
      store.set((s) => ({
        ...s,
        items: result.items,
        outputs: Object.fromEntries(Object.entries(s.outputs).filter(([id]) => live.has(id))),
      }));
    } catch {
      // A failed read keeps the last list; the next poll reads again.
    }
  };

  /** Reads the output of every process whose output is shown. */
  const readOutputs = async (): Promise<void> => {
    const { sessionId, outputs } = store.get();
    if (sessionId === null) return;
    for (const id of Object.keys(outputs)) {
      try {
        const result = await run({ op: "output", sessionId, id });
        if (result.kind !== "output" || store.get().sessionId !== sessionId) continue;
        const text = result.available ? result.text : "No output can be read for this process.";
        store.set((s) => (id in s.outputs ? { ...s, outputs: { ...s.outputs, [id]: text } } : s));
      } catch {
        // Gone meanwhile: the next list read drops it.
      }
    }
  };

  return {
    store,
    refresh,
    readOutputs,

    /** Shows or hides the output of `id` (shown output is read at once and then live). */
    toggleOutput(id: string): void {
      store.set((s) => {
        if (id in s.outputs)
          return {
            ...s,
            outputs: Object.fromEntries(Object.entries(s.outputs).filter(([key]) => key !== id)),
          };
        return { ...s, outputs: { ...s.outputs, [id]: null } };
      });
      void readOutputs();
    },

    /** Stop for one process of the session. Returns true when it stopped. */
    async stop(id: string): Promise<boolean> {
      const sessionId = store.get().sessionId;
      if (sessionId === null || store.get().stopping.includes(id)) return false;
      store.set((s) => ({ ...s, stopping: [...s.stopping, id] }));
      try {
        await run({ op: "stopSession", sessionId, id });
        return true;
      } catch (thrown) {
        toasts.push({
          kind: "error",
          bellKind: "failure",
          text: toCallError(thrown).error.message,
        });
        return false;
      } finally {
        store.set((s) => ({ ...s, stopping: s.stopping.filter((x) => x !== id) }));
        await refresh();
      }
    },

    /**
     * Keeps the list (and shown output) current: `isOpen` says whether the dialog shows. Returns the function that
     * stops it.
     */
    start(isOpen: () => boolean): () => void {
      void refresh();
      let last = 0;
      const timer = setInterval(() => {
        const open = isOpen();
        const now = Date.now();
        if (open) void readOutputs();
        if (now - last >= (open ? LIST_POLL_MS : IDLE_POLL_MS)) {
          last = now;
          void refresh();
        }
      }, OUTPUT_POLL_MS);
      const unsubscribe = shell.store.subscribe(() => {
        if (shell.store.get().selected !== store.get().sessionId) void refresh();
      });
      return () => {
        clearInterval(timer);
        unsubscribe();
      };
    },
  };
}

/** The controller type. */
export type SessionProcesses = ReturnType<typeof createSessionProcesses>;
