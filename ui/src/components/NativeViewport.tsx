/**
 * A placeholder box whose position and size are sent to the core, which places a native child webview (the
 * preview, page viewer, File tab viewer, or agent browser) exactly over
 * it. The webview is native and paints above the page, so the box reports "hidden" (`null`) whenever it should not
 * cover anything: unmounted (tab or panel closed, session switched), not live, or a modal dialog open (palette,
 * settings, confirmations). Only the browser's Save prompt opts into showing the page around its reserved dialog bounds.
 * `null` hides the child; the core owns whether its engine stays parked or is released.
 * While live the box carries `data-toast-avoid="native"`, so the toast stack picks a corner
 * the view does not cover, or consumes grid space in a tray. The reported rectangle leaves out fixed notice
 * reservations in `nativeLayer` and is clamped to the DOM box above the status bar.
 * Depends on: React, protocol/core (`Rect`), nativeLayer. Called by: features/preview/Viewport, features/pages/PagesTab,
 * features/files/FileTab.
 * Invariants:
 * - Coordinates are the main webview's CSS pixels, which are the window's logical pixels (the main webview fills
 *   the window at zoom 1). Rounding is inward; device scale belongs to the native toolkit.
 * - Every change is coalesced to one report per animation frame; unmount always reports `null`.
 * - `NATIVE_LAYOUT_EVENT` fires when the box appears, moves, resizes or goes (not for a clip alone), so the toasts
 *   place themselves again without a feedback loop.
 */
import { type ReactElement, type ReactNode, useEffect, useRef } from "react";

import type { Rect } from "../protocol/core";
import { nativeBounds } from "./nativeBounds";
import { NATIVE_LAYOUT_EVENT, clipAround, onReservations, reserved } from "./nativeLayer";

/** A modal hides native views unless the Save prompt has finished reserving its measured bounds. */
function modalHidesNative(): boolean {
  return Array.from(document.querySelectorAll<HTMLElement>('[aria-modal="true"]')).some(
    (modal) =>
      modal.dataset.nativeOverlay !== "preview-save" || modal.dataset.nativeReady !== "true",
  );
}

/** The box. A null rectangle with `present` parks a mounted panel; cleanup clears its presence. */
export function NativeViewport({
  report,
  live,
  label,
  className,
  syncKey,
  children,
}: {
  report: (rect: Rect | null, present?: boolean) => void;
  live: boolean;
  label: string;
  className: string;
  /** Browser chrome revision: remeasure the same slot when a native overlay changes. */
  syncKey?: number;
  children?: ReactNode;
}): ReactElement {
  const box = useRef<HTMLDivElement>(null);
  const measureCurrent = useRef<() => void>(() => undefined);
  useEffect(() => {
    const element = box.current;
    if (element === null || !live) {
      report(null, true);
      return () => {
        report(null);
      };
    }
    let frame = 0;
    // The box as last laid out (not clipped), to tell the toasts only about real layout changes.
    let laidOut = "";
    const measure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const r = element.getBoundingClientRect();
        const footer = document.querySelector(".statusbar")?.getBoundingClientRect();
        // clamp to the DOM box above the status bar; a hidden view parks (`report(null, true)`).
        const rect = nativeBounds(
          r,
          { width: window.innerWidth, height: window.innerHeight },
          footer !== undefined && footer.height > 0 ? footer.top : undefined,
        );
        const visible =
          rect !== null && rect.width >= 16 && rect.height >= 16 && !modalHidesNative();
        const area = visible ? clipAround(rect, reserved()) : null;
        if (area === null) report(null, true);
        else report(area);
        const key = JSON.stringify(rect);
        if (key !== laidOut) {
          laidOut = key;
          window.dispatchEvent(new Event(NATIVE_LAYOUT_EVENT));
        }
      });
    };
    measureCurrent.current = measure;
    const resize = new ResizeObserver(measure);
    resize.observe(element);
    // A notification tray or status-bar change can move the content boundary without a window resize.
    const footer = document.querySelector(".statusbar");
    if (footer !== null) resize.observe(footer);
    const dialogs = new MutationObserver(measure);
    dialogs.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["aria-modal", "role", "data-native-ready"],
    });
    window.addEventListener("resize", measure);
    document.addEventListener("scroll", measure, true);
    // The panel grows and shrinks with a CSS transition: re-measure when it ends.
    document.addEventListener("transitionend", measure);
    // A notice reserved or released its place above native views.
    const stopReservations = onReservations(measure);
    measure();
    return () => {
      measureCurrent.current = () => undefined;
      cancelAnimationFrame(frame);
      resize.disconnect();
      dialogs.disconnect();
      stopReservations();
      window.removeEventListener("resize", measure);
      document.removeEventListener("scroll", measure, true);
      document.removeEventListener("transitionend", measure);
      report(null);
      window.dispatchEvent(new Event(NATIVE_LAYOUT_EVENT));
    };
  }, [report, live]);
  useEffect(() => {
    measureCurrent.current();
  }, [syncKey]);
  return (
    <div
      ref={box}
      className={className}
      data-live={live}
      data-toast-avoid={live ? "native" : undefined}
      aria-label={label}
      role="region"
    >
      {children}
    </div>
  );
}
