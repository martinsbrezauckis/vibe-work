/**
 * Processes left running: the UI side of the core's `processes` command. The agent's dev
 * servers keep running when their session is deleted, because only the user decides to stop them. Right after a
 * delete the window asks once, in the native dialog the core shows ("Also stop its N running processes?"); what
 * keeps running is listed behind the status bar's "Left running" button, each with Stop.
 * Depends on: coreStore (`CoreController`), createStore, toastStore, protocol types. Called by: createApp (wiring),
 * sessionActions (`afterDelete` once a delete succeeded), useShellWiring (`start`), the status bar and its dialog,
 * the `ui/processes` route.
 * Invariants:
 * - Nothing is stopped from here without a click (Stop, or Continue in the native dialog); a Debug API request is
 *   that click, and its `afterDelete` asks nothing and stops nothing (the core's rule).
 * - The list is re-read after a delete, a stop, opening the dialog, a `lagged` event, a server restart, and every
 *   30 s while it is not empty (so a server that ended on its own leaves it).
 * - Failures become one toast; nothing throws to the caller.
 */
import type { LeftRunningView, ProcessesRequest, ProcessesResult } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Toasts } from "./toastStore";

/** How often a non-empty list is checked again for processes that ended by themselves. */
export const LEFT_RUNNING_POLL_MS = 30_000;

/** The UI state. */
export interface LeftRunningState {
  /** Still running, oldest first. */
  items: readonly LeftRunningView[];
  /** The list dialog is open. */
  open: boolean;
  /** Ids whose Stop runs. */
  stopping: readonly string[];
}

/** "python3 · port 5173" / "pid 4242". */
export function processLabel(item: LeftRunningView): string {
  const name = item.name ?? `pid ${String(item.pid)}`;
  if (item.ports.length === 0) return name;
  const ports = item.ports.map(String).join(", ");
  return `${name} · port${item.ports.length === 1 ? "" : "s"} ${ports}`;
}

/** Creates the controller. */
export function createLeftRunning(deps: { core: CoreController; toasts: Toasts }) {
  const { core, toasts } = deps;
  const store = createStore<LeftRunningState>({ items: [], open: false, stopping: [] });

  const run = async (request: ProcessesRequest): Promise<ProcessesResult> => {
    const result = await core.run({ type: "processes", request });
    if (result.type !== "processes") throw new Error(`unexpected answer ${result.type}`);
    return result.result;
  };
  const fail = (thrown: unknown): void => {
    toasts.push({ kind: "error", bellKind: "failure", text: toCallError(thrown).error.message });
  };

  const refresh = async (): Promise<void> => {
    try {
      const result = await run({ op: "leftRunning" });
      if (result.kind === "leftRunning") store.set((s) => ({ ...s, items: result.items }));
    } catch {
      // A failed read keeps the last list; the next trigger reads again.
    }
  };

  return {
    store,
    refresh,

    /**
     * After a delete succeeded: the core asks the native question when the session left processes running (and
     * stops them on Continue). Returns the answer (`null` when the read failed), so the `ui/sessions` route can
     * hand the list to a Debug API client.
     */
    async afterDelete(sessionId: string): Promise<ProcessesResult | null> {
      let result: ProcessesResult;
      try {
        result = await run({ op: "afterDelete", sessionId });
      } catch (thrown) {
        fail(thrown);
        return null;
      }
      if (result.kind === "afterDelete") {
        if (result.failed.length > 0)
          toasts.push({
            kind: "warn",
            bellKind: "failure",
            text: `Not stopped: ${result.failed.join(" · ")}`,
          });
      }
      await refresh();
      return result;
    },

    /** Stop for one listed process. Returns true when it stopped. */
    async stop(id: string): Promise<boolean> {
      if (store.get().stopping.includes(id)) return false;
      store.set((s) => ({ ...s, stopping: [...s.stopping, id] }));
      try {
        await run({ op: "stop", id });
        return true;
      } catch (thrown) {
        fail(thrown);
        return false;
      } finally {
        store.set((s) => ({ ...s, stopping: s.stopping.filter((x) => x !== id) }));
        await refresh();
      }
    },

    /** Opens the list (and reads it again). */
    open(): void {
      store.set((s) => ({ ...s, open: true }));
      void refresh();
    },

    close(): void {
      store.set((s) => ({ ...s, open: false }));
    },

    /**
     * Reads the list now (the startup sweep may have found processes an earlier run's deleted sessions left) and
     * keeps it current (module invariants). Returns the function that stops it.
     */
    start(): () => void {
      void refresh();
      const stopListening = core.listen((event) => {
        if (
          event.type === "lagged" ||
          (event.type === "server" && event.status.state === "running")
        )
          void refresh();
      });
      const timer = setInterval(() => {
        if (store.get().items.length > 0) void refresh();
      }, LEFT_RUNNING_POLL_MS);
      return () => {
        stopListening();
        clearInterval(timer);
      };
    },
  };
}

/** The controller type. */
export type LeftRunning = ReturnType<typeof createLeftRunning>;
