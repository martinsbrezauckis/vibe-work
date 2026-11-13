/** Person-picked tab context and window handoff, with the same state and actions in the Debug API. */
import type { ChromeControlView, ChromeTab } from "../../protocol/core";
import type { ChromePicker } from "../../state/composerStore";
import { NEW_CHAT } from "../../state/newChatKey";
import type { App } from "../shell/AppContext";

type Source = "window" | "debug-api";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
/** One browser identity however it is spelled: a bare Chrome UUID and "chrome:<uuid>" in any case are the same. */
export function sameProfile(left: string, right: string): boolean {
  const chrome = (id: string) => {
    const raw = id.startsWith("chrome:") ? id.slice("chrome:".length) : id;
    return UUID.test(raw) ? raw.toLowerCase() : null;
  };
  if (left === right) return true;
  const a = chrome(left);
  return a !== null && a === chrome(right);
}

/**
 * The profile a chat's browser work uses (`chatProfiles`), or null while it has none yet. `owner` is the chat, or the
 * New chat draft's owner. A chat keeps its profile, so the tab list offers only that profile's tabs.
 */
export function chatProfile(
  view: ChromeControlView | null | undefined,
  owner: string | null,
): string | null {
  if (owner === null) return null;
  return view?.chatProfiles?.find((row) => row.sessionId === owner)?.profileId ?? null;
}
export async function openChromeTabs(
  app: App,
  sessionId: string,
  origin: Source,
  browser: "Chrome" | "Edge" | "Safari" | "Firefox" = "Chrome",
): Promise<void> {
  const pending: ChromePicker = {
    sessionId,
    rows: [],
    loading: true,
    error: null,
    ...(browser !== "Chrome" ? { browser } : {}),
  };
  app.composer.setChromePicker(pending);
  try {
    const result = await app.core?.run(
      { type: "chromeControl", request: { op: "tabs" } },
      origin === "window" ? { origin } : undefined,
    );
    if (app.composer.store.get().chromePicker !== pending) return;
    if (result?.type !== "chromeTabs")
      throw new Error(
        "Chrome did not answer. Check that Chrome is open and paired, then try again.",
      );
    app.composer.setChromePicker({
      ...pending,
      loading: false,
      rows: result.tabs.filter((tab) => {
        const view = app.core?.store.get().chromeControl;
        if (browser !== "Chrome") return tab.profileId.startsWith(`${browser.toLowerCase()}:`);
        if (tab.profileId.startsWith("safari:")) return false;
        if (view?.firefox !== undefined && tab.profileId.startsWith("firefox:")) return false;
        return view?.edge === undefined || !tab.profileId.startsWith("edge:");
      }),
    });
  } catch {
    if (app.composer.store.get().chromePicker === pending)
      app.composer.setChromePicker({
        ...pending,
        loading: false,
        error: `${browser} did not answer. Check that ${browser} is open and paired, then try again.`,
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
  const browser = current.browser ?? "Chrome";
  const pending = { ...current, loading: true, error: null };
  app.composer.setChromePicker(pending);
  try {
    if (sessionId === NEW_CHAT && app.shell.store.get().newChatOwner === null)
      await app.newChat?.start();
    const owner = sessionId === NEW_CHAT ? app.shell.store.get().newChatOwner : sessionId;
    if (owner === null) throw new Error("Chrome did not answer");
    const result = await app.core?.run(
      { type: "chromeControl", request: { op: "tabContext", ...tab, sessionId: owner } },
      origin === "window" ? { origin } : undefined,
    );
    if (app.composer.store.get().chromePicker !== pending) return;
    if (result?.type !== "chromeSelection" || result.selection === null)
      throw new Error(
        "Chrome did not answer. Check that Chrome is open and paired, then try again.",
      );
    const picked = result.selection;
    const actions = origin === "window" ? app.composer.actions : app.composer.debugApi;
    actions?.insertContext(sessionId, {
      label: picked.title.slice(0, 60) || `${browser} tab`,
      source: "browser",
      text: `${picked.url}\n${picked.text}`,
      modelText: picked.modelText,
    });
    app.composer.setChromePicker(null);
  } catch {
    if (app.composer.store.get().chromePicker === pending)
      app.composer.setChromePicker({
        ...pending,
        loading: false,
        error: `${browser} did not answer. Check that ${browser} is open and paired, then try again.`,
      });
  }
}
