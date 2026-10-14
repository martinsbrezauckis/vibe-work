/**
 * The `ui/terminal` route: lets an external agent read and drive the Terminal
 * tab the way a user does. Typing goes through the core's `terminal` command (`POST /commands` or
 * `/terminals/{id}/input`), which writes directly for the Debug API too.
 * Depends on: state/uiControl, store.ts, model.ts, selection.ts, AppContext types. Called by:
 * register.tsx. The Rust side lists the route (`crates/vw-host/src/terminal.rs`).
 * Invariants: bodies are validated (`invalidParams` otherwise); `selection` only reads.
 */
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { activeOf } from "./model";
import { shownTerminal } from "./selection";
import type { Terminals } from "./store";

function asObject(body: unknown): Record<string, unknown> {
  if (typeof body === "object" && body !== null && !Array.isArray(body))
    return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

/** Registers `ui/terminal`; returns the function that removes it. */
export function registerTerminalRoute(app: App, terminals: Terminals): () => void {
  const state = () => {
    const sessionId = app.shell.store.get().selected;
    const tabs = terminals.store.get();
    const shown = shownTerminal();
    return {
      sessionId,
      terminals:
        sessionId === null
          ? []
          : (tabs.bySession[sessionId] ?? []).map((t) => ({
              terminalId: t.terminalId,
              state: t.state.type,
              pid: t.pid,
              cwd: t.cwd,
            })),
      active: sessionId === null ? null : (activeOf(tabs, sessionId)?.terminalId ?? null),
      shown: shown === null ? null : { terminalId: shown.terminalId, ...shown.info() },
      focused: shown?.hasFocus() ?? false,
      error: sessionId === null ? null : (tabs.error[sessionId] ?? null),
    };
  };
  return app.uiControl.register("terminal", async (body) => {
    const { action, terminalId } = asObject(body);
    const sessionId = app.shell.store.get().selected;
    switch (action) {
      case "state":
        return state();
      case "selection":
        return { text: shownTerminal()?.selection() ?? "" };
      case "selectAll":
        shownTerminal()?.selectAll();
        return { text: shownTerminal()?.selection() ?? "" };
      case "focus":
        shownTerminal()?.focus();
        return state();
      case "open":
        if (sessionId === null) throw new UiControlError("invalidParams", "no session is shown");
        await terminals.open(sessionId);
        return state();
      case "select":
        if (sessionId === null || typeof terminalId !== "string")
          throw new UiControlError("invalidParams", "select needs a shown session and terminalId");
        terminals.select(sessionId, terminalId);
        return state();
      default:
        throw new UiControlError(
          "invalidParams",
          "action must be state, selection, selectAll, focus, open or select",
        );
    }
  });
}
