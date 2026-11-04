/** Available browsers and per-chat picker choice. Composer and its Debug API share these actions and core state. */
import type { ChromeControlView } from "../../protocol/core";
import { chromeUsable, edgeUsable, firefoxUsable } from "../settings/chrome/chromeRows";
import type { App } from "../shell/AppContext";
import { openChromeTabs } from "./chromeTabs";

export type ComposerBrowser = "Chrome" | "Edge" | "Firefox";

/** Only connected browsers whose module is present can supply tab context. */
export function usableBrowsers(view: ChromeControlView | null | undefined): ComposerBrowser[] {
  return [
    ...(chromeUsable(view) ? ["Chrome" as const] : []),
    ...(edgeUsable(view) ? ["Edge" as const] : []),
    ...(firefoxUsable(view) ? ["Firefox" as const] : []),
  ];
}

/** The chat's browser session, then its last picker choice, then Chrome or the first usable browser. */
export async function openBrowserTabs(
  app: App,
  sessionId: string,
  origin: "window" | "debug-api",
  requested?: ComposerBrowser,
): Promise<void> {
  const view = app.core?.store.get().chromeControl;
  const browsers = usableBrowsers(view);
  const last = app.composer.store.get().tabBrowser[sessionId];
  const sessionBrowser = view?.work.find((row) => row.sessionId === sessionId)?.browser;
  const browser = [requested, sessionBrowser, last, ...browsers].find(
    (candidate): candidate is ComposerBrowser =>
      candidate !== undefined && browsers.some((browser) => browser === candidate),
  );
  if (browser !== undefined) await openChromeTabs(app, sessionId, origin, browser);
}
