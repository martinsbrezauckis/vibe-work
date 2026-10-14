/**
 * The window's native child views and the HTML notices that must stay visible above them.
 *
 * The preview, the Project and File viewers and the agent browser are native webviews placed over a
 * `NativeViewport` box; the OS paints them above the page, so an HTML toast or the update notice under one cannot be
 * seen. Two rules keep notices visible:
 * 1. A live native box carries `data-toast-avoid="native"`: toasts pick a clear gap or show one compact notice
 *    in the existing status bar. Opening the full notification dialog reserves its bounds.
 * 2. Fixed notices such as the update notice reserve their rectangle here and every native view clips itself
 *    around it (`clipAround`): the view gets a little smaller while the notice shows, and nothing is hidden.
 *
 * Depends on: nothing (DOM for `nativeRects`). Called by: NativeViewport (subscribes, clips), Toasts and UpdateNotice
 * (nativeRects / dialog reservation; UpdateNotice reserves), tests.
 * Invariants: `clipAround` is pure; a reservation that does not touch a box leaves it unchanged; a box clipped below
 * 16 x 16 px is reported hidden (`null`), never as a sliver.
 */

/** A rectangle in the main webview's CSS pixels. */
export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The smallest native view worth showing (the same limit NativeViewport uses). */
export const MIN_SIDE = 16;
/** Space kept between a notice and the clipped view. */
export const CLIP_GAP = 8;

const reservations = new Map<string, Box>();
const listeners = new Set<() => void>();

/**
 * Sets (or with `null` removes) the rectangle `key` must keep visible above native views; tells every native view
 * to measure again when it changed.
 */
export function reserve(key: string, box: Box | null): void {
  const before = reservations.get(key);
  if (box === null) {
    if (before === undefined) return;
    reservations.delete(key);
  } else {
    if (
      before !== undefined &&
      before.x === box.x &&
      before.y === box.y &&
      before.width === box.width &&
      before.height === box.height
    )
      return;
    reservations.set(key, box);
  }
  for (const listener of listeners) listener();
}

/** The current reservations. */
export function reserved(): Box[] {
  return Array.from(reservations.values());
}

/** Calls `listener` whenever a reservation changes; returns the function that stops it. */
export function onReservations(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Event NativeViewport fires on `window` when a native view appears, moves or goes (the toasts place again). */
export const NATIVE_LAYOUT_EVENT = "vw-native-layout";

/** The rectangles of the live native boxes, read from the DOM. */
export function nativeRects(): DOMRect[] {
  return Array.from(document.querySelectorAll('[data-toast-avoid="native"]')).map((el) =>
    el.getBoundingClientRect(),
  );
}

function intersects(a: Box, b: Box): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/**
 * `box` minus every reserved rectangle that touches it: for each one, the largest of the four cuts (keep above,
 * below, left of or right of it, `CLIP_GAP` away). `null` when less than `MIN_SIDE` remains either way.
 */
export function clipAround(box: Box, around: readonly Box[]): Box | null {
  let current: Box = box;
  for (const r of around) {
    if (!intersects(current, r)) continue;
    const right = current.x + current.width;
    const bottom = current.y + current.height;
    const cuts: Box[] = [
      // Above the reservation.
      { ...current, height: r.y - CLIP_GAP - current.y },
      // Below it.
      { ...current, y: r.y + r.height + CLIP_GAP, height: bottom - (r.y + r.height + CLIP_GAP) },
      // Left of it.
      { ...current, width: r.x - CLIP_GAP - current.x },
      // Right of it.
      { ...current, x: r.x + r.width + CLIP_GAP, width: right - (r.x + r.width + CLIP_GAP) },
    ];
    const best = cuts
      .filter((c) => c.width >= MIN_SIDE && c.height >= MIN_SIDE)
      .reduce<Box | null>(
        (a, c) => (a === null || c.width * c.height > a.width * a.height ? c : a),
        null,
      );
    if (best === null) return null;
    current = best;
  }
  return current;
}
