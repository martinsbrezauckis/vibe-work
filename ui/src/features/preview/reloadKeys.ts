/** Main-window reload bindings share the Browser toolbar actions. Called by preview registration; native page keys stay in that page. */
import type { App } from "../shell/AppContext";
import { awayFromHome } from "../shell/home";
import type { Browser } from "./browser";
import type { Previews } from "./store";
import { previewVisibility } from "./visibility";

/** Reserves window reload keys, reloading only the open chat's visible page. */
export function registerReloadKeys(
  app: App,
  previews: Previews | null,
  browser: Browser | null,
): () => void {
  const reload = () => {
    if (previews === null || browser === null) return;
    const shell = app.shell.store.get();
    const report = previewVisibility(shell, browser, awayFromHome(app));
    const sessionId = shell.selected;
    if (
      !report.windowVisible ||
      !report.browserPanelOpen ||
      sessionId === null ||
      report.selectedSessionId !== sessionId ||
      shell.overlay !== null
    )
      return;
    const tab = browser.store.get().tabs[sessionId]?.find((t) => t.active);
    const preview =
      tab?.kind === "web" ? tab.preview : previews.store.get().lists[sessionId]?.preview;
    if (
      !preview?.url ||
      preview.state === "suspended" ||
      preview.state === "stopped" ||
      preview.waitingForBrowser ||
      previews.store.get().busy[sessionId] === true
    )
      return;
    void (tab?.kind === "web"
      ? browser.webAction(sessionId, tab.pageKey, "reload")
      : previews.reload(sessionId));
  };
  const removers = ["F5", "Mod+R", "Mod+Shift+R", "Mod+F5"].map((keys, index) =>
    app.shortcuts.register({
      id: `browser.reload-${String(index)}`,
      keys,
      description: "Reload",
      allowInInput: true,
      allowInTerminal: true,
      terminalGetsKey: true,
      allowWhileComposing: true,
      stopPropagation: true,
      run: index < 2 ? reload : () => {},
    }),
  );
  return () => {
    for (const remove of removers) remove();
  };
}
