/**
 * Inward-rounded native child bounds in the main view's CSS/logical pixels. NativeViewport calls
 * this before reserving notice space; pure geometry tests cover clipping at the window/footer.
 * Device scale is handled by the native toolkit, never applied to these coordinates a second time.
 */
import type { Rect } from "../protocol/core";
import { MIN_SIDE } from "./nativeLayer";

/** Intersect the DOM rectangle with the window's content area above its status bar. */
export function nativeBounds(
  dom: Pick<DOMRect, "left" | "top" | "width" | "height">,
  viewport: { width: number; height: number },
  statusTop = viewport.height,
): Rect | null {
  const values = [
    dom.left,
    dom.top,
    dom.width,
    dom.height,
    viewport.width,
    viewport.height,
    statusTop,
  ];
  if (values.some((value) => !Number.isFinite(value)) || dom.width <= 0 || dom.height <= 0)
    return null;
  const x = Math.ceil(Math.max(0, dom.left));
  const y = Math.ceil(Math.max(0, dom.top));
  const right = Math.floor(Math.min(viewport.width, dom.left + dom.width));
  const bottom = Math.floor(Math.min(viewport.height, statusTop, dom.top + dom.height));
  const width = right - x;
  const height = bottom - y;
  return width >= MIN_SIDE && height >= MIN_SIDE ? { x, y, width, height } : null;
}
