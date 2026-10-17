/**
 * The `ui/browser` UI-control route: a Debug API client reads the Browser tab's agent strip,
 * action log and preview notes, and does what the tab's buttons do (Stop, Resume, note on an element or area, the
 * note text, remove, add to the message, check again, the address bar) and the Capture menu (`capture`
 * {kind}). Commands run while the request is served
 * carry the Debug API's origin (`state/coreStore.ts`); notes it creates or edits stay marked as its own (`by`), and
 * their chips and crops carry its mark whoever sends them.
 * Depends on: state/uiControl, browser.ts, store.ts, browserModel.ts, AppContext types. Called by: register.tsx. The
 * Rust side lists the route (`crates/vw-host/src/host.rs` BROWSER_ROUTES).
 * Invariants: bodies are validated (`invalidParams`); `sessionId` defaults to the selected session; crops are not
 * returned (their presence is).
 */
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { awayFromHome } from "../shell/home";
import type { Browser } from "./browser";
import { CAPTURE_KINDS, type Capture } from "./capture";
import { addressToUrl, isLocalPage } from "./browserModel";
import type { Previews } from "./store";
import { pickPreviewVisibilityReason, previewVisibility } from "./visibility";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function text(value: unknown, field: string): string {
  if (typeof value !== "string")
    throw new UiControlError("invalidParams", `${field} must be a string`);
  return value;
}

/** Registers `ui/browser`; returns the function that removes it. */
export function registerBrowserRoute(
  app: App,
  browser: Browser,
  previews: Previews,
  capture: Capture,
): () => void {
  const state = (sessionId: string, targetPageKey = sessionId) => {
    const ui = browser.store.get();
    const tabs = ui.tabs[sessionId] ?? [];
    const selected = tabs.find((tab) => tab.active);
    const appPreview = previews.store.get().lists[sessionId]?.preview ?? null;
    const selectedPreview = selected?.kind === "web" ? selected.preview : appPreview;
    const url = selectedPreview?.url ?? null;
    const targetTab = tabs.find((tab) => tab.pageKey === targetPageKey);
    if (targetPageKey !== sessionId && targetTab === undefined)
      throw new UiControlError("invalidParams", "tabId is not a page of this session");
    const targetPreview = targetTab?.kind === "web" ? targetTab.preview : appPreview;
    const visibilityReason = pickPreviewVisibilityReason(
      previewVisibility(app.shell.store.get(), browser, awayFromHome(app)),
      {
        sessionId,
        pageKey: targetPageKey,
        kind: targetTab?.kind === "web" ? "web" : "app",
        paused: targetPreview?.state === "suspended",
      },
    );
    return {
      sessionId,
      url,
      previewState: selectedPreview?.state ?? null,
      server: selectedPreview?.server ?? null,
      loading: selectedPreview?.loading ?? false,
      visibilityReason,
      tabs: tabs.map((tab) => {
        const preview = tab.kind === "web" ? tab.preview : appPreview;
        return {
          tabId: tab.pageKey,
          kind: tab.kind,
          ownerKind: tab.ownerKind,
          active: tab.active,
          agentCurrent: tab.agentCurrent,
          title: preview?.title ?? null,
          url: preview?.url ?? null,
          state: preview?.state ?? null,
          server: preview?.server ?? null,
          loading: preview?.loading ?? false,
        };
      }),
      selectedTabId: selected?.pageKey ?? sessionId,
      notesOffered: selected?.kind !== "web" && isLocalPage(url),
      agent: ui.states[sessionId] ?? null,
      picking: ui.picking[sessionId] ?? null,
      error: ui.error[sessionId] ?? null,
      capture: {
        running: capture.store.get().busy[sessionId] ?? null,
        last: capture.store.get().last[sessionId] ?? null,
        error: capture.store.get().error[sessionId] ?? null,
      },
      notes: (ui.notes[sessionId] ?? []).map((n) => ({
        id: n.id,
        mode: n.annotation.mode,
        role: n.annotation.role,
        name: n.annotation.name,
        route: n.annotation.route,
        source: n.annotation.source,
        note: n.note,
        status: n.status,
        by: n.by,
        crop: n.annotation.cropPngBase64 !== null,
      })),
    };
  };
  return app.uiControl.register("browser", async (body) => {
    const args = asObject(body);
    const sessionId =
      typeof args["sessionId"] === "string" ? args["sessionId"] : app.shell.store.get().selected;
    if (sessionId === null) throw new UiControlError("invalidParams", "no session is selected");
    switch (args["action"] ?? "state") {
      case "state":
        await Promise.all([
          browser.load(sessionId),
          browser.loadTabs(sessionId),
          previews.load(sessionId),
        ]);
        return state(sessionId, typeof args["tabId"] === "string" ? args["tabId"] : sessionId);
      case "tabs":
        await browser.loadTabs(sessionId);
        return state(sessionId);
      case "openTab":
        await browser.openTab(sessionId, typeof args["url"] === "string" ? args["url"] : undefined);
        return state(sessionId);
      case "switchTab":
        await browser.switchTab(sessionId, text(args["tabId"], "tabId"));
        return state(sessionId);
      case "closeTab":
        await browser.closeTab(sessionId, text(args["tabId"], "tabId"));
        return state(sessionId);
      case "stop":
        await browser.stopAgent(sessionId);
        return state(sessionId);
      case "resume":
        await browser.resumeAgent(sessionId);
        return state(sessionId);
      case "annotate": {
        const mode = args["mode"] === "box" ? "box" : "element";
        // No person points for a Debug API request: it names the target and the answer is immediate.
        const target = args["target"];
        if (typeof target !== "object" || target === null || Array.isArray(target))
          throw new UiControlError(
            "invalidParams",
            mode === "box"
              ? "target is required: target.rect (x, y, width, height)"
              : "target is required: target.ref, target.selector, or target.x and target.y",
          );
        const added = await browser.annotate(sessionId, mode, "debug-api", target);
        if (added === null)
          throw new UiControlError(
            "invalidParams",
            browser.store.get().error[sessionId] ?? "nothing was annotated",
          );
        return { added: added.id, ...state(sessionId) };
      }
      case "cancel":
        await browser.cancelPick(sessionId);
        return state(sessionId);
      case "note":
        browser.setNote(sessionId, text(args["id"], "id"), text(args["text"], "text"), "debug-api");
        return state(sessionId);
      case "remove":
        browser.remove(sessionId, text(args["id"], "id"));
        return state(sessionId);
      case "send":
        return { sent: browser.send(sessionId, "debug-api"), ...state(sessionId) };
      case "recheck":
        await browser.recheck(sessionId, args["reload"] !== false);
        return state(sessionId);
      case "capture": {
        // what the Capture menu does (the same `browser` `capture` command; the menu's kinds).
        const kind = text(args["kind"] ?? "design", "kind");
        if (!CAPTURE_KINDS.some((k) => k.id === kind))
          throw new UiControlError(
            "invalidParams",
            `kind must be one of ${CAPTURE_KINDS.map((k) => k.id).join(", ")}`,
          );
        const selectedTabId =
          browser.store.get().tabs[sessionId]?.find((tab) => tab.active)?.pageKey ?? sessionId;
        const done = await capture.capture(sessionId, kind, {}, selectedTabId);
        return { captured: done, ...state(sessionId) };
      }
      case "address":
        {
          const url = addressToUrl(text(args["url"], "url"));
          const tab = browser.store.get().tabs[sessionId]?.find((item) => item.active);
          if (tab?.kind === "web") await browser.webAction(sessionId, tab.pageKey, "navigate", url);
          else if (isLocalPage(url)) await previews.open(sessionId, { url });
          else await browser.openTab(sessionId, url);
        }
        return state(sessionId);
      default:
        throw new UiControlError("invalidParams", "unknown action");
    }
  });
}
