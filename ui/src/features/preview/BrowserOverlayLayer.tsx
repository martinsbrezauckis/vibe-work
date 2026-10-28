/**
 * General Browser chrome overlay host: parks the selected native page for chrome popovers.
 * Adapted from ShellX.
 * Depends on: browserOverlays, typed preview controller. Called by: PreviewTab.
 * Invariant: the page's logical bounds stay fixed; native restoration precedes removing the cover;
 * stale async results never cover a different page.
 */
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactElement,
} from "react";

import type { Rect } from "../../protocol/core";
import {
  browserOverlayCount,
  browserOverlayRevision,
  closeBrowserOverlays,
  focusBrowserOverlay,
  onBrowserOverlays,
  trackBrowserOverlayWork,
  watchBrowserOverlayLayout,
} from "./browserOverlays";
import type { Previews } from "./store";

interface Frame {
  bounds: Rect;
  /** The page's picture, shown in place of the parked page; null: a plain cover. */
  snapshot: string | null;
  sessionId: string;
  tabId: string | undefined;
  pageUrl: string;
}

/** Parks the native child for all chrome popovers together, including nested selectors. */
export function BrowserOverlayLayer({
  previews,
  sessionId,
  tabId,
  pageUrl,
}: {
  previews: Previews;
  sessionId: string;
  tabId?: string;
  pageUrl: string | null;
}): ReactElement | null {
  useSyncExternalStore(onBrowserOverlays, browserOverlayRevision);
  const open = browserOverlayCount() > 0;
  const [frame, setFrame] = useState<Frame | null>(null);
  const [state, setState] = useState<"preparing" | "open" | "restoring">("preparing");
  const queue = useRef(Promise.resolve());
  const previousPage = useRef({ sessionId, tabId, pageUrl });

  useLayoutEffect(() => {
    if (pageUrl !== null && (open || frame !== null)) {
      document.body.dataset.browserOverlay = state;
      if (state === "open" && open) focusBrowserOverlay();
    } else delete document.body.dataset.browserOverlay;
    return () => {
      delete document.body.dataset.browserOverlay;
    };
  }, [state, open, frame, pageUrl]);

  useEffect(() => watchBrowserOverlayLayout(), []);
  useEffect(() => {
    const previous = previousPage.current;
    previousPage.current = { sessionId, tabId, pageUrl };
    if (
      previous.sessionId !== sessionId ||
      previous.tabId !== tabId ||
      previous.pageUrl !== pageUrl
    )
      closeBrowserOverlays();
  }, [sessionId, tabId, pageUrl]);

  useEffect(() => {
    if (!open || pageUrl === null) return;
    const attempt = { current: true };
    const active = () => attempt.current;
    const task = queue.current
      .then(async () => {
        setState("preparing");
        const result = await previews.browserOverlay(sessionId, "prepare", tabId);
        if (!active()) return;
        if (result.bounds === null) {
          // The page is no longer the one in view (a tab switch overtook the menu): nothing to park, the menus close.
          closeBrowserOverlays();
          return;
        }
        setFrame({
          bounds: result.bounds,
          snapshot: result.snapshot ?? null,
          sessionId,
          tabId,
          pageUrl,
        });
        await previews.browserOverlay(sessionId, "show", tabId);
        if (active()) setState("open");
      })
      .catch((error: unknown) => {
        // The person switched tabs or closed the menu while this attempt waited: the core refuses parking a page
        // that is no longer selected, which is expected and not an error to show over the page now in view.
        if (active()) throw error;
      });
    queue.current = task.catch(() => {
      // Keep backend refusal/error visible through the controller; never pretend the live view was hidden.
      if (attempt.current) closeBrowserOverlays();
    });
    void previews.trackAction(task);
    trackBrowserOverlayWork(task);
    return () => {
      attempt.current = false;
      setState("restoring");
      const restore = queue.current.then(async () => {
        // Retry a transient native restoration error while retaining the page cover.
        try {
          await previews.browserOverlay(sessionId, "close", tabId);
        } catch {
          await previews.browserOverlay(sessionId, "close", tabId);
        }
        // A newly opened menu keeps the page covered until parking is ready.
        if (browserOverlayCount() === 0) setFrame(null);
      });
      queue.current = restore.catch(() => {
        // Retain the cover when native restoration failed; the controller reports the error.
      });
      void previews.trackAction(restore);
      trackBrowserOverlayWork(restore);
    };
  }, [open, previews, sessionId, tabId, pageUrl]);

  if (
    frame === null ||
    frame.sessionId !== sessionId ||
    frame.tabId !== tabId ||
    frame.pageUrl !== pageUrl
  )
    return null;
  const { bounds, snapshot } = frame;
  return (
    <div
      className="browser-overlay-page"
      data-state={state}
      style={{ left: bounds.x, top: bounds.y, width: bounds.width, height: bounds.height }}
      aria-hidden="true"
    >
      {snapshot !== null && <img className="browser-overlay-shot" src={snapshot} alt="" />}
    </div>
  );
}
