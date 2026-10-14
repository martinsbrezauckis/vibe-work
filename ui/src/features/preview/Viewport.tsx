/**
 * The page area of the Browser tab: a `NativeViewport` whose rectangle places the session's native
 * preview webview. Queued and paused engines show status text. The measuring rules
 * (coalescing, hidden under modal dialogs, `null` on unmount) live in
 * components/NativeViewport, shared with the Pages tab's viewer.
 * Adapted from ShellX.
 * Depends on: store.ts (`layout`), components/NativeViewport. Called by: PreviewTab.
 */
import { type ReactElement, type ReactNode, useCallback, useSyncExternalStore } from "react";

import { NativeViewport } from "../../components/NativeViewport";
import type { Rect } from "../../protocol/core";
import {
  browserEngineBoundsAreParked,
  browserEngineSyncBoundsForRect,
} from "./browserEngineLayout";
import { browserOverlayRevision, onBrowserOverlays } from "./browserOverlays";
import type { Previews } from "./store";

/** The box. `sessionId` is the shown session; `live` is false while nothing is open in it. */
export function Viewport({
  previews,
  sessionId,
  tabId,
  live,
  waiting = false,
  paused = false,
  stopped = null,
  snapshot,
  onReload,
  children,
}: {
  previews: Previews;
  sessionId: string;
  tabId?: string;
  live: boolean;
  waiting?: boolean;
  paused?: boolean;
  stopped?: string | null;
  snapshot?: string | null;
  onReload?: () => void;
  /** The server list and empty guidance are page content, never toolbar content. */
  children?: ReactNode;
}): ReactElement {
  const overlayRevision = useSyncExternalStore(onBrowserOverlays, browserOverlayRevision);
  const report = useCallback(
    (rect: Rect | null, present = rect !== null) => {
      if (!present) {
        previews.layout(null);
        return;
      }
      // The core's Layout API represents the parked
      // native slot with null; the native adapter parks the child offscreen.
      const sync = browserEngineSyncBoundsForRect(
        rect === null
          ? { left: 0, top: 0, width: 0, height: 0 }
          : { left: rect.x, top: rect.y, width: rect.width, height: rect.height },
      );
      previews.layout({
        sessionId,
        ...(tabId ? { tabId } : {}),
        rect: browserEngineBoundsAreParked(sync) ? null : sync,
      });
    },
    [previews, sessionId, tabId],
  );
  return (
    <NativeViewport
      report={report}
      live={live}
      label={tabId ? "Selected browser page" : "Preview of the app"}
      className="preview-viewport"
      syncKey={overlayRevision}
    >
      {stopped !== null ? (
        <div className="empty" role="status">
          <p>Server stopped</p>
          <p className="preview-url">{stopped}</p>
          {onReload && (
            <button type="button" className="preview-btn" onClick={onReload}>
              Reload
            </button>
          )}
        </div>
      ) : waiting ? (
        <div className="empty" role="status">
          Waiting for the browser
        </div>
      ) : paused ? (
        <div className="browser-paused" role="status">
          {snapshot && (
            <img
              className="browser-paused-shot"
              src={snapshot}
              alt="Last view before this tab paused"
            />
          )}
          <div className="browser-paused-message">
            <span>Browser paused</span>
            {onReload && (
              <button type="button" className="preview-btn" onClick={onReload}>
                Reload
              </button>
            )}
          </div>
        </div>
      ) : (
        !live && (
          <>
            {children}
            <div className="empty">
              Type an address above, or open one of this session's servers, to see the page here.
            </div>
          </>
        )
      )}
    </NativeViewport>
  );
}
