/**
 * Plugs the terminal into the app: creates the terminal controller, registers the
 * Terminal tab body over the placeholder and the `ui/terminal` route, and sends terminal links to
 * the link dialog (`app.transcript.askLink`; "Open in browser" there calls `open_link`).
 * Depends on: store.ts, TerminalPanel, terminalUiControl, AppContext types. Called by: main.tsx
 * (after the placeholders), the UI test rig.
 * Invariants: without a core there is nothing to register (the placeholder stays); the returned
 * function undoes every registration and stops following core events. The New chat screen's terminals are counted
 * for its status line and follow the session its first message starts (`app.newChat`).
 */
import type { App } from "../shell/AppContext";
import { TerminalPanel } from "./TerminalPanel";
import { setLinkOpener } from "./links";
import { createTerminals } from "./store";
import { registerTerminalRoute } from "./terminalUiControl";

/** Registers the terminal tab and route on `app`; returns the function that removes them. */
export function registerTerminalTab(app: App): () => void {
  const core = app.core;
  if (core === null) return () => undefined;
  const terminals = createTerminals(core);
  const removeBody = app.panelTabs.register("terminal", () => (
    <TerminalPanel terminals={terminals} />
  ));
  const removeRoute = registerTerminalRoute(app, terminals);
  // The New chat screen's terminals: counted for its status line and moved to the session its first message starts.
  const draft = app.newChat;
  const stopHandOver = draft?.onHandOver((from, to) => {
    terminals.handOver(from, to);
  });
  const count = () => {
    const owner = app.shell.store.get().newChatOwner;
    draft?.report("terminals", owner === null ? 0 : terminals.count(owner));
  };
  const stopCount = [terminals.store.subscribe(count), app.shell.store.subscribe(count)];
  const removeOpener = setLinkOpener((url) => {
    app.transcript.askLink(url);
  });
  return () => {
    stopHandOver?.();
    for (const stop of stopCount) stop();
    removeOpener();
    removeRoute();
    removeBody();
    terminals.stop();
  };
}
