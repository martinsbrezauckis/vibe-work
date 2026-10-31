/** Selected Chrome context becomes a chip. Delivery never sends a turn or changes typed text. */
import type { ChromeControlView } from "../../protocol/core";
import { NEW_CHAT } from "../../state/newChatKey";
import type { App } from "../shell/AppContext";

export function startChromeSelections(app: App): () => void {
  const core = app.core;
  if (core === null) return () => {};
  const claiming = new Set<string>();
  const receive = (view: ChromeControlView): void => {
    for (const selectionId of view.pendingSelections) {
      if (claiming.has(selectionId)) continue;
      claiming.add(selectionId);
      void core
        .run(
          { type: "chromeControl", request: { op: "takeSelection", selectionId } },
          { origin: "window" },
        )
        .then((result) => {
          if (result.type !== "chromeSelection" || result.selection === null) return;
          const picked = result.selection;
          const sessionId = picked.sessionId ?? NEW_CHAT;
          app.composer.actions?.insertContext(sessionId, {
            label: `Selected text from ${picked.title.slice(0, 60) || picked.url.slice(0, 60)}`,
            source: "browser",
            text: `${picked.url}\n${picked.text}`,
            modelText: picked.modelText,
          });
          app.shell.actions.selectSession(picked.sessionId);
        })
        .catch(() => {})
        .finally(() => {
          claiming.delete(selectionId);
        });
    }
  };
  const stop = core.listen((event) => {
    if (event.type === "chromeControl") receive(event.view);
  });
  void core
    .run({ type: "chromeControl", request: { op: "read" } }, { origin: "window" })
    .then((result) => {
      if (result.type === "chromeControl") receive(result.view);
    })
    .catch(() => {});
  return stop;
}
