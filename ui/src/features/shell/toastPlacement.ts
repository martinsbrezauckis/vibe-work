/**
 * Where the toast stack goes. Notifications never cover input
 * fields. Native child views (preview, viewers, agent browser) paint above
 * the page, so a toast under one is not covered but invisible: they are a harder rule than inputs.
 * Pure geometry so it is testable without a layout engine: given the window, the stack's size, the
 * rectangles to keep clear and the native views, pick the first corner whose stack rectangle
 * touches none of them. If the corners fail, search clear vertical gaps before using the status bar.
 * Depends on: nothing. Called by: Toasts (feeds real rectangles), tests (synthetic ones).
 * A bottom corner can also be **lifted** just above the regions it would cover (the composer), which is
 * tried after the plain bottom-right and top-right corners and before the left ones: in a session view the composer
 * fills the bottom and the header's actions (Changes, Files...) the top right, so the stack sits right above the
 * composer instead of over a button. A lifted stack must be free of every region and below the title bar.
 * Invariants: the corner offsets mirror `overlays.css` (`.toasts[data-corner]`): 18 px from the
 * side edges, 40 px above the bottom (status bar, `BOTTOM`), 48 px below the top (title bar); a lift adds to the
 * bottom offset (the stack sets it inline).
 * A visible rail adds a minimum left edge; the painted stack and native reservation use that same edge.
 */

/** A rectangle in viewport coordinates. */
export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** Available window and the minimum left edge (the rail stays outside every candidate). */
interface Viewport {
  width: number;
  height: number;
  leftInset?: number;
}

/** The four corners, in the order they are tried. */
export type Corner = "bottom-right" | "top-right" | "bottom-left" | "top-left";

const ORDER: readonly Corner[] = ["bottom-right", "top-right", "bottom-left", "top-left"];
const SIDE = 18;
/** Distance of a bottom corner's stack from the window's bottom edge (above the status bar). */
export const BOTTOM = 40;
const TOP = 48;
/** Space left between a lifted stack and the region it sits above. */
const GAP = 8;

/** The rectangle the stack would occupy in `corner`, a bottom corner raised by `lift` px. */
export function stackRect(
  corner: Corner,
  viewport: Viewport,
  size: { width: number; height: number },
  lift = 0,
): Rect {
  const left = corner.endsWith("left")
    ? Math.max(SIDE, viewport.leftInset ?? SIDE)
    : viewport.width - SIDE - size.width;
  const top = corner.startsWith("top") ? TOP : viewport.height - BOTTOM - lift - size.height;
  return { left, top, right: left + size.width, bottom: top + size.height };
}

function overlap(a: Rect, b: Rect): number {
  const w = Math.min(a.right, b.right) - Math.max(a.left, b.left);
  const h = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
  return w > 0 && h > 0 ? w * h : 0;
}

function fits(rect: Rect, viewport: Viewport): boolean {
  return (
    rect.left >= Math.max(SIDE, viewport.leftInset ?? SIDE) &&
    rect.right <= viewport.width - SIDE &&
    rect.top >= TOP &&
    rect.bottom <= viewport.height - 26
  );
}

/** The chosen corner, lift, and whether no entirely clear candidate exists (use the footer). */
export interface Placement {
  corner: Corner;
  blocked: boolean;
  /** Extra px above the plain bottom offset (0: the plain corner). */
  lift: number;
}

/** The lift that puts a bottom corner's stack right above what it covers, when that spot is free of everything. */
function liftFor(
  corner: Corner,
  viewport: Viewport,
  size: { width: number; height: number },
  avoid: readonly Rect[],
  native: readonly Rect[],
): number | null {
  const plain = stackRect(corner, viewport, size);
  const hits = avoid.filter((a) => overlap(plain, a) > 0);
  if (hits.length === 0) return null;
  const lift = plain.bottom - (Math.min(...hits.map((a) => a.top)) - GAP);
  if (lift <= 0) return null;
  const raised = stackRect(corner, viewport, size, lift);
  if (!fits(raised, viewport)) return null;
  const free = [...avoid, ...native].every((a) => overlap(raised, a) === 0);
  return free ? lift : null;
}

/** Finds a clear gap along one side, nearest the work area's bottom. */
function gapFor(
  corner: "bottom-right" | "bottom-left",
  viewport: Viewport,
  size: { width: number; height: number },
  obstacles: readonly Rect[],
): number | null {
  const plain = stackRect(corner, viewport, size);
  const tops = obstacles.flatMap((rect) => [rect.top - size.height - GAP, rect.bottom + GAP]);
  tops.sort((a, b) => b - a);
  for (const top of tops) {
    const lift = plain.top - top;
    if (lift <= 0) continue;
    const candidate = stackRect(corner, viewport, size, lift);
    if (fits(candidate, viewport) && obstacles.every((rect) => overlap(candidate, rect) === 0))
      return lift;
  }
  return null;
}

/**
 * Picks the corner for the stack (see the module docs). With nothing to avoid it is always the bottom right.
 * `native` = rectangles of native views; `avoid` = interactive and protected regions.
 */
export function placeToastsOver(
  viewport: Viewport,
  size: { width: number; height: number },
  avoid: readonly Rect[],
  native: readonly Rect[],
): Placement {
  for (const corner of ORDER) {
    const rect = stackRect(corner, viewport, size);
    if (fits(rect, viewport) && [...avoid, ...native].every((a) => overlap(rect, a) === 0))
      return { corner, blocked: false, lift: 0 };
    // right after the two right corners, the bottom right raised above what it covers.
    if (corner === "top-right") {
      const lift = liftFor("bottom-right", viewport, size, avoid, native);
      if (lift !== null) return { corner: "bottom-right", blocked: false, lift };
    }
  }
  const left = liftFor("bottom-left", viewport, size, avoid, native);
  if (left !== null) return { corner: "bottom-left", blocked: false, lift: left };
  const obstacles = [...avoid, ...native];
  for (const corner of ["bottom-right", "bottom-left"] as const) {
    const lift = gapFor(corner, viewport, size, obstacles);
    if (lift !== null) return { corner, blocked: false, lift };
  }
  return { corner: "bottom-right", blocked: true, lift: 0 };
}

/** Picks a clear corner with no native views, or null when the stack must use the footer. */
export function placeToasts(
  viewport: Viewport,
  size: { width: number; height: number },
  avoid: readonly Rect[],
): Corner | null {
  const placement = placeToastsOver(viewport, size, avoid, []);
  return placement.blocked ? null : placement.corner;
}
