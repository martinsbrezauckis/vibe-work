/** Read-only tab context for the person, with the same state and actions in the Debug API. */
import type { ChromeTab } from "../../protocol/core";
import type { ChromePicker } from "../../state/composerStore";
import type { App } from "../shell/AppContext";

type Source = "window" | "debug-api";
export async function openChromeTabs(app: App, sessionId: string, origin: Source): Promise<void> {
  const pending: ChromePicker = { sessionId, rows: [], loading: true, error: null };
  app.composer.setChromePicker(pending);
  try {
    const result = await app.core?.run(
      { type: "chromeControl", request: { op: "tabs" } },
      origin === "window" ? { origin } : undefined,
    );
    if (app.composer.store.get().chromePicker !== pending) return;
    if (result?.type !== "chromeTabs") throw new Error("Chrome tabs unavailable");
    app.composer.setChromePicker({ ...pending, loading: false, rows: result.tabs });
  } catch {
    if (app.composer.store.get().chromePicker === pending)
      app.composer.setChromePicker({
        ...pending,
        loading: false,
        error: "Chrome tabs unavailable",
      });
  }
}
export async function pickChromeTab(
  app: App,
  sessionId: string,
  tab: Pick<ChromeTab, "profileId" | "tabId">,
  origin: Source,
): Promise<void> {
  const current = app.composer.store.get().chromePicker;
  if (current?.sessionId !== sessionId || current.loading) return;
  const pending = { ...current, loading: true, error: null };
  app.composer.setChromePicker(pending);
  try {
    const result = await app.core?.run(
      { type: "chromeControl", request: { op: "tabContext", ...tab } },
      origin === "window" ? { origin } : undefined,
    );
    if (app.composer.store.get().chromePicker !== pending) return;
    if (result?.type !== "chromeSelection" || result.selection === null)
      throw new Error("Chrome tab unavailable");
    const picked = result.selection;
    const actions = origin === "window" ? app.composer.actions : app.composer.debugApi;
    actions?.insertContext(sessionId, {
      label: picked.title.slice(0, 60) || "Chrome tab",
      source: "browser",
      text: `${picked.url}\n${picked.text}`,
      modelText: picked.modelText,
    });
    app.composer.setChromePicker(null);
  } catch {
    if (app.composer.store.get().chromePicker === pending)
      app.composer.setChromePicker({ ...pending, loading: false, error: "Chrome tab unavailable" });
  }
}
