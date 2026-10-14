/**
 * The terminal controller: the tabs store plus the actions the panel and the
 * `ui/terminal` route use. Every action is a typed `terminal` command through the core client
 * (components render; they do not decide).
 * Depends on: state/coreStore (`CoreController`: `client`, `listen`), state/createStore, model.ts.
 * Called by: register.tsx (creates one per app), TerminalPanel, session.ts, terminalUiControl.
 * Invariants:
 * - The store follows the core: command results and `terminal` events (also from the Debug API)
 *   go through the same reducers, so the tabs match what runs.
 * - A failed action sets the session's error line; nothing throws into a component.
 * - Input from this window is sent with `origin = ui` by the Tauri client (no dialog); in
 *   browser dev mode the client is the Debug API, so the core asks for confirmation.
 * - While a Debug API `ui/*` request is served, input is marked as the Debug API's (core controller), except a
 *   focus report (`isFocusReport`), which the view sends by itself and which stays the window's.
 */
import { type ExecuteOptions, toCallError } from "../../protocol/client/types";
import type {
  CommandResult,
  TerminalRequest,
  TerminalResult,
  TerminalView,
} from "../../protocol/core";
import type { CoreController } from "../../state/coreStore";
import { createStore } from "../../state/createStore";
import {
  type TerminalTabs,
  applyList,
  applyTerminalEvent,
  initialTabs,
  setActive,
  setError,
  upsert,
} from "./model";

/** Initial size before the view has measured itself (it resizes right after). */
const DEFAULT_COLS = 80;
const DEFAULT_ROWS = 24;

function resultOf(result: CommandResult): TerminalResult {
  if (result.type !== "terminal") throw new Error(`unexpected ${result.type} result`);
  return result.result;
}

/** Creates the controller for `core`. */
export function createTerminals(core: CoreController) {
  const store = createStore<TerminalTabs>(initialTabs);
  const run = async (request: TerminalRequest, options?: ExecuteOptions): Promise<TerminalResult> =>
    resultOf(await core.client.execute({ type: "terminal", request }, options));
  const failed = (sessionId: string, thrown: unknown): null => {
    store.set((s) => setError(s, sessionId, toCallError(thrown).error.message));
    return null;
  };
  const ok = (sessionId: string): void => {
    store.set((s) => setError(s, sessionId, null));
  };
  const stopListening = core.listen((event) => {
    if (event.type === "terminal") store.set((s) => applyTerminalEvent(s, event.event));
    // after `lagged` the core's events for this were missed: read again what is shown.
    if (event.type === "lagged")
      for (const id of Object.keys(store.get().loaded)) void controller.load(id);
  });

  // Sessions that already got their automatic first terminal (once per app run: a user who closed
  // every tab is not given a new one behind their back).
  const autoOpened = new Set<string>();

  const controller = {
    store,
    /**
     * Called when the Terminal tab shows `sessionId`: reads its terminals and, the first time a
     * session shows an empty tab, opens one (like opening a terminal app).
     */
    async ensure(sessionId: string): Promise<void> {
      if (store.get().loaded[sessionId] !== true) await controller.load(sessionId);
      if (store.get().loaded[sessionId] !== true || autoOpened.has(sessionId)) return;
      autoOpened.add(sessionId);
      if ((store.get().bySession[sessionId] ?? []).length === 0) await controller.open(sessionId);
    },
    /** Reads the session's terminals from the core. */
    async load(sessionId: string): Promise<void> {
      try {
        const result = await run({ op: "list", sessionId });
        if (result.type === "terminals")
          store.set((s) => applyList(s, sessionId, result.terminals));
        ok(sessionId);
      } catch (thrown) {
        failed(sessionId, thrown);
      }
    },
    /** Opens a terminal in the session's folder and makes it the active tab. */
    async open(
      sessionId: string,
      cols = DEFAULT_COLS,
      rows = DEFAULT_ROWS,
    ): Promise<TerminalView | null> {
      try {
        const result = await run({ op: "open", sessionId, cols, rows });
        if (result.type !== "terminal") return null;
        const view = result.terminal;
        store.set((s) => setActive(upsert(s, view), sessionId, view.terminalId));
        ok(sessionId);
        return view;
      } catch (thrown) {
        return failed(sessionId, thrown);
      }
    },
    /** Makes a tab active. */
    select(sessionId: string, terminalId: string): void {
      store.set((s) => setActive(s, sessionId, terminalId));
    },
    /** Stops the terminal's processes and removes the tab. */
    async close(sessionId: string, terminalId: string): Promise<void> {
      try {
        await run({ op: "close", terminalId });
        ok(sessionId);
      } catch (thrown) {
        failed(sessionId, thrown);
      }
    },
    /** Stops the shell and starts a new one in the same tab. */
    async restart(sessionId: string, terminalId: string): Promise<void> {
      try {
        const result = await run({ op: "restart", terminalId });
        if (result.type === "terminal") store.set((s) => upsert(s, result.terminal));
        ok(sessionId);
      } catch (thrown) {
        failed(sessionId, thrown);
      }
    },
    /**
     * Sends input (the view's input queue keeps the order). Input for a terminal that is closed or
     * whose shell exited is dropped here: nothing could read it, and sending it would only produce
     * a refusal. It happens on Windows, where the pseudo console turns on focus reporting, so the
     * view sends a focus-out report while it is being removed after a close (seen on Windows 11).
     */
    async write(terminalId: string, data: string, binary: boolean): Promise<void> {
      const running = Object.values(store.get().bySession).some((list) =>
        list.some((t) => t.terminalId === terminalId && t.state.type === "running"),
      );
      if (!running) return;
      await run(
        binary ? { op: "write", terminalId, data, binary } : { op: "write", terminalId, data },
        // A focus report is the view's own answer to a focus change, never typed input: it goes out as the
        // window's even when a Debug API request moved the focus (previously treated as Debug API input).
        !binary && isFocusReport(data) ? { origin: "window" } : undefined,
      );
    },
    /** Resizes the PTY; failures are ignored (the next fit tries again). */
    async resize(terminalId: string, cols: number, rows: number): Promise<void> {
      try {
        await run({ op: "resize", terminalId, cols, rows });
      } catch {
        // A resize racing a close; nothing to show.
      }
    },
    /** Reports an error for the session (input failures from the view). */
    report(sessionId: string, thrown: unknown): void {
      failed(sessionId, thrown);
    },
    /** The core client (the view attaches its output stream through it). */
    client: core.client,
    /** Stops following core events. */
    stop: stopListening,
  };
  return controller;
}

/** The controller the app uses. */
export type Terminals = ReturnType<typeof createTerminals>;

/**
 * True for exactly one xterm focus report: `ESC [ I` (focus in) or `ESC [ O` (focus out), which xterm sends when the
 * program turned focus reporting on (ConPTY always does on Windows). Anything longer is input and keeps its origin.
 */
export function isFocusReport(data: string): boolean {
  return data === "\x1b[I" || data === "\x1b[O";
}
