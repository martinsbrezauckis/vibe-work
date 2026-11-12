/** Built-in tabs use the same person-share queue as pane sharing. Called by the composer's browser picker. */
import type { App } from "../shell/AppContext";
import type { ChromePicker } from "../../state/composerStore";
import { NEW_CHAT } from "../../state/newChatKey";

export async function openBuiltinTabs(app: App, sessionId: string): Promise<void> {
  const pending: ChromePicker = {
    sessionId,
    browser: "Vibe Work",
    rows: [],
    loading: true,
    error: null,
  };
  app.composer.setChromePicker(pending);
  const owner = sessionId === NEW_CHAT ? app.shell.store.get().newChatOwner : sessionId;
  try {
    const result =
      owner === null
        ? null
        : await app.core?.run({ type: "browser", request: { op: "tabs", sessionId: owner } });
    if (app.composer.store.get().chromePicker !== pending) return;
    if (result != null && (result.type !== "browser" || result.result.type !== "tabs"))
      throw new Error("tabs unavailable");
    app.composer.setChromePicker({
      ...pending,
      loading: false,
      builtinRows:
        result?.type === "browser" && result.result.type === "tabs"
          ? result.result.tabs.filter((tab) => tab.preview?.url)
          : [],
    });
  } catch {
    if (app.composer.store.get().chromePicker === pending)
      app.composer.setChromePicker({
        ...pending,
        loading: false,
        error: "Could not read browser tabs. Try again.",
      });
  }
}
export async function pickBuiltinTab(app: App, sessionId: string, pageKey: string): Promise<void> {
  const current = app.composer.store.get().chromePicker;
  if (current?.sessionId !== sessionId || current.loading) return;
  const pending = { ...current, loading: true, error: null };
  app.composer.setChromePicker(pending);
  try {
    await app.core?.run(
      { type: "browser", request: { op: "share", pageKey } },
      { origin: "window" },
    );
    if (app.composer.store.get().chromePicker === pending) app.composer.setChromePicker(null);
  } catch {
    if (app.composer.store.get().chromePicker === pending)
      app.composer.setChromePicker({
        ...pending,
        loading: false,
        error: "The page changed or could not be shared. Choose it again.",
      });
  }
}
