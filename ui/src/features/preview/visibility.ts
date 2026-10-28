/**
 * Reports the Browser pane's selected page and window visibility to the core when they change.
 * Depends on: shell state, browser tabs, typed preview command. Called by: register.tsx.
 */
import type { CoreController } from "../../state/coreStore";
import { panelOf, panelOwner, type ShellState } from "../../state/shellState";
import type { App } from "../shell/AppContext";
import { awayFromHome } from "../shell/home";
import type { Browser } from "./browser";

export type PreviewVisibilityReport = ReturnType<typeof previewVisibility>;
export type VisibilityReason =
  | "windowHidden"
  | "mainPage"
  | "anotherSession"
  | "browserPanelClosed"
  | "browserPaused"
  | "anotherPage";

/** First reason the requested App preview or web tab cannot be captured. */
export function pickPreviewVisibilityReason(
  report: PreviewVisibilityReport,
  target: { sessionId: string; pageKey: string; kind: "app" | "web"; paused: boolean },
): VisibilityReason | null {
  if (!report.windowVisible) return "windowHidden";
  if (report.selectedSessionId === null) return "mainPage";
  if (report.selectedSessionId !== target.sessionId) return "anotherSession";
  if (!report.browserPanelOpen) return "browserPanelClosed";
  if (target.paused) return "browserPaused";
  if (report.selectedPageKey !== target.pageKey) return "anotherPage";
  return null;
}

/** The facts the core needs to explain why a requested screenshot is unavailable. */
export function previewVisibility(
  shell: ShellState,
  browser: Browser,
  covered = false,
): {
  windowVisible: boolean;
  selectedSessionId: string | null;
  browserPanelOpen: boolean;
  selectedPageKey: string | null;
} {
  const sessionId = covered || shell.settings.open ? null : panelOwner(shell);
  const panel = panelOf(shell);
  const browserPanelOpen =
    panel.open && panel.active === "browser" && !covered && !shell.settings.open;
  const selectedPageKey =
    browserPanelOpen && sessionId !== null
      ? (browser.store.get().tabs[sessionId]?.find((tab) => tab.active)?.pageKey ?? sessionId)
      : null;
  return {
    windowVisible: document.visibilityState === "visible",
    selectedSessionId: sessionId,
    browserPanelOpen,
    selectedPageKey,
  };
}

/** Sends the latest facts once per change, serially so an old report cannot overtake a newer one. */
export function reportPreviewVisibility(
  core: CoreController,
  app: App,
  browser: Browser,
): () => void {
  let latest = previewVisibility(app.shell.store.get(), browser, awayFromHome(app));
  let sent = "";
  let sending = false;
  let active = true;
  let pageShown = true;
  const flush = async () => {
    if (sending) return;
    sending = true;
    try {
      while (active) {
        const report = latest;
        const key = JSON.stringify(report);
        if (key === sent) break;
        sent = key;
        try {
          await core.run({ type: "preview", request: { op: "visibility", report } });
        } catch {
          // A later report can recover; layout and browser commands retain their own errors.
        }
      }
    } finally {
      sending = false;
    }
  };
  const changed = () => {
    latest = {
      ...previewVisibility(app.shell.store.get(), browser, awayFromHome(app)),
      windowVisible: pageShown && document.visibilityState === "visible",
    };
    void flush();
  };
  const pageHide = () => {
    pageShown = false;
    changed();
  };
  const pageShow = () => {
    pageShown = true;
    changed();
  };
  const stopShell = app.shell.store.subscribe(changed);
  const stopBrowser = browser.store.subscribe(changed);
  const stopProjects = app.projects?.store.subscribe(changed);
  const stopConnectors = app.connectors?.store.subscribe(changed);
  const stopLibraries = app.libraries?.store.subscribe(changed);
  document.addEventListener("visibilitychange", changed);
  window.addEventListener("pageshow", pageShow);
  window.addEventListener("pagehide", pageHide);
  changed();
  return () => {
    active = false;
    stopShell();
    stopBrowser();
    stopProjects?.();
    stopConnectors?.();
    stopLibraries?.();
    document.removeEventListener("visibilitychange", changed);
    window.removeEventListener("pageshow", pageShow);
    window.removeEventListener("pagehide", pageHide);
  };
}
