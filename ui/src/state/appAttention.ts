/** Counts unviewed waiting chats for the native app/tray icons. Uses the same waiting state as the
 * session list; opening acknowledges the current prompts, while a later prompt marks the chat again. */
import { pendingHistoryAsks } from "../features/transcript/model/protectedChange";
import type { App } from "../features/shell/AppContext";
import { badgeOf } from "./sessionSummary";
import { promptHost, waitingPrompts, waitingSessions } from "../features/ask/waiting";
import { awayFromHome } from "../features/shell/home";
import { documentFocused } from "./notifyStore";

/** Subscribe for the shell's lifetime; notification settings do not control these indicators. */
export function startAppAttention(app: App): () => void {
  const core = app.core;
  const desktop = core?.client.desktop;
  if (core == null || desktop?.attention === undefined) return () => undefined;
  const seen = new Map<string, string>();
  let previous = -1;
  let writes = Promise.resolve();
  const update = () => {
    const state = core.store.get();
    const waiting = waitingSessions(state);
    const prompts = waitingPrompts(state);
    const shell = app.shell.store.get();
    const open = documentFocused() && !awayFromHome(app) ? shell.selected : null;
    const pending = new Set<string>();
    let count = 0;
    for (const entry of Object.values(state.sessions)) {
      if (badgeOf(entry, state.stuck, waiting) !== "input") continue;
      const parent = entry.session["parentSessionId"];
      const host = typeof parent === "string" ? state.sessions[parent] : undefined;
      if (entry.ownedByParent === true && host && badgeOf(host, state.stuck, waiting) === "input")
        continue;
      pending.add(entry.id);
      const signature = JSON.stringify([
        prompts
          .filter((ask) => promptHost(state, ask) === entry.id)
          .map((ask) => ask.id)
          .sort(),
        entry.openCallbackIds,
        entry.childCallbackIds ?? [],
        pendingHistoryAsks(entry.state?.["history"]),
      ]);
      if (open === entry.id) seen.set(entry.id, signature);
      if (seen.get(entry.id) !== signature) count += 1;
    }
    for (const id of seen.keys()) if (!pending.has(id)) seen.delete(id);
    if (previous === count) return;
    previous = count;
    // Preserve native update order even when an IPC takes longer than the next state change.
    writes = writes
      .then(() => desktop.attention?.(count))
      .catch(() => {
        previous = -1;
      });
  };
  const off = [core.store.subscribe(update), app.shell.store.subscribe(update)];
  for (const store of [
    app.projects?.store,
    app.connectors?.store,
    app.libraries?.store,
    app.keys?.store,
  ]) {
    if (store) off.push(store.subscribe(update));
  }
  window.addEventListener("focus", update);
  document.addEventListener("visibilitychange", update);
  update();
  return () => {
    for (const stop of off) stop();
    window.removeEventListener("focus", update);
    document.removeEventListener("visibilitychange", update);
  };
}
