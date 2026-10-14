/**
 * Plugs the Browser tab into the app: creates the preview and agent-browser controllers, registers
 * the tab body over the placeholder and the `ui/browser` route, and opens the tab when the agent starts using the
 * selected session's browser.
 * Depends on: store.ts, browser.ts, capture.ts, browserRoutes.ts, PreviewTab, pages/point (`insertChip`), AppContext types.
 * Called by: main.tsx (after the placeholders), the UI test rig.
 * Invariants: without a core nothing is registered (the placeholder stays); the returned function undoes the
 * registrations, hides every preview and stops following events. Notes go to the composer as chips and pictures
 * only (data, never a command).
 */
import { insertChip } from "../pages/point";
import type { App } from "../shell/AppContext";
import { createBrowser } from "./browser";
import { createCapture } from "./capture";
import { registerBrowserRoute } from "./browserRoutes";
import { PreviewTab } from "./PreviewTab";
import { createPreviews } from "./store";
import { reportPreviewVisibility } from "./visibility";

/** Registers the Browser tab body and route on `app`; returns the function that removes them. */
export function registerPreviewTab(app: App): () => void {
  const core = app.core;
  if (core === null) return () => undefined;
  const previews = createPreviews(core);
  let pictures = 0;
  const browser = createBrowser(
    core,
    {
      chip: (sessionId, chip, origin) => insertChip(app, sessionId, chip, origin).ok,
      picture: (sessionId, name, data, bytes, origin) => {
        pictures += 1;
        app.composer.addImage(sessionId, {
          id: `browser-note-${String(pictures)}`,
          name,
          mimeType: "image/png",
          data,
          bytes,
          ...(origin === "debug-api" ? { by: "debug-api" as const } : {}),
        });
      },
    },
    (sessionId) => {
      // The user sees it: the agent started using this session's browser.
      if (app.shell.store.get().selected === sessionId) app.shell.actions.openTab("browser");
    },
  );
  const capture = createCapture(core);
  const stopVisibility = reportPreviewVisibility(core, app, browser);
  const removeBody = app.panelTabs.register("browser", () => (
    <PreviewTab previews={previews} browser={browser} capture={capture} />
  ));
  const stopPreviewRequests = core.listen((event) => {
    if (
      event.type === "previewShowRequested" &&
      app.shell.store.get().selected === event.sessionId
    ) {
      // can leave a web tab selected. An explicit preview width asks to show the
      // pinned App preview, so select it before the tool waits for a live view.
      void browser.switchTab(event.sessionId, event.sessionId);
      app.shell.actions.openTab("browser");
    }
  });
  const removeRoute = registerBrowserRoute(app, browser, previews, capture);
  return () => {
    stopPreviewRequests();
    stopVisibility();
    removeRoute();
    removeBody();
    previews.layout(null);
    previews.stop();
    browser.dispose();
  };
}
