/** Available browsers and per-chat picker choice. Composer and its Debug API share these actions and core state. */
import { browserView } from "../settings/chrome/browserView";
import type { ChromeControlView } from "../../protocol/core";
import { chromeUsable, edgeUsable, firefoxUsable, setupLine } from "../settings/chrome/chromeRows";
import type { App } from "../shell/AppContext";
import { openBuiltinTabs } from "./builtinTabs";
import { openChromeTabs } from "./chromeTabs";

export type ComposerBrowser = "Chrome" | "Edge" | "Safari" | "Firefox" | "Vibe Work";

/** Only connected browsers whose module is present can supply tab context. */
export function usableBrowsers(view: ChromeControlView | null | undefined): ComposerBrowser[] {
  const browsers: ComposerBrowser[] = [
    ...(chromeUsable(view) ? ["Chrome" as const] : []),
    ...(edgeUsable(view) ? ["Edge" as const] : []),
    ...(chromeUsable(browserView(view, "Safari")) ? ["Safari" as const] : []),
    ...(firefoxUsable(view) ? ["Firefox" as const] : []),
    ...(view?.browserPreferences?.builtinAvailable && view.browserPreferences.builtinEnabled
      ? ["Vibe Work" as const]
      : []),
  ];
  return browsers.filter((browser) => composerShown(view, browser));
}

/** Presentation never changes a browser's connection or profile authority. */
export function composerShown(
  view: ChromeControlView | null | undefined,
  browser: ComposerBrowser,
): boolean {
  return (
    view?.browserPreferences?.shown.includes(browser === "Vibe Work" ? "Builtin" : browser) ?? true
  );
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
  if (browser === "Vibe Work") await openBuiltinTabs(app, sessionId);
  else if (browser !== undefined) await openChromeTabs(app, sessionId, origin, browser);
}

/** The three readiness states at the tab entry point; labels never decide connection behavior. */
export function browserSignals(view: ChromeControlView | null | undefined): {
  browser: ComposerBrowser;
  state: "missing" | "waiting" | "ready";
  label: string;
}[] {
  if (view == null) return [];
  const external = (["Chrome", "Edge", "Safari", "Firefox"] as const).flatMap((browser) => {
    if (!composerShown(view, browser)) return [];
    const selected = browserView(view, browser);
    if (selected === null || selected.setup.state === "absent") return [];
    const setup = selected.setup;
    const missing =
      setup.browserIssue === "missing" ||
      setup.browserIssue === "differentBrowser" ||
      setup.addon?.state === "chromeMissing" ||
      setup.addon?.state === "otherBrowser";
    return [
      {
        browser,
        state: missing
          ? ("missing" as const)
          : chromeUsable(selected)
            ? ("ready" as const)
            : ("waiting" as const),
        label:
          browser === "Firefox"
            ? selected.enabled
              ? selected.setup.pairing
              : "Off"
            : setupLine(selected, browser),
      },
    ];
  });
  return [
    ...external,
    ...(view.browserPreferences?.builtinAvailable && composerShown(view, "Vibe Work")
      ? [
          {
            browser: "Vibe Work" as const,
            state: view.browserPreferences.builtinEnabled
              ? ("ready" as const)
              : ("waiting" as const),
            label: view.browserPreferences.builtinEnabled ? "Available" : "Off",
          },
        ]
      : []),
  ];
}
