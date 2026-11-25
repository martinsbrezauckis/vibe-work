/**
 * Block diagram edge anchoring. Every connector leaves the source on its border and ends on the target's border
 * (node or group), on the sides that face each other, so a line never runs through the source label and an
 * arrowhead always sits on the border it points at.
 */
import type { Point } from "../../layout/layered";

/** A node or group rectangle in diagram coordinates; edges are anchored on its border. */
export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
  /** True for a composite (group) box: its connectors anchor inside the overlap with the other box. */
  group?: boolean;
}

/** Header band of a group: a connector between a group and its own child avoids the top side. */
const TOP_PENALTY = 24;

function contains(outer: Box, inner: Box): boolean {
  return (
    outer.x <= inner.x &&
    outer.y <= inner.y &&
    outer.x + outer.width >= inner.x + inner.width &&
    outer.y + outer.height >= inner.y + inner.height
  );
}

/** Source and target overlap on both axes (a group and something inside it): nearest sides, one straight segment. */
function routeNested(a: Box, b: Box): Point[] {
  const aInner = contains(b, a) && !contains(a, b);
  const inner = aInner ? a : b;
  const outer = aInner ? b : a;
  const icx = inner.x + inner.width / 2;
  const icy = inner.y + inner.height / 2;
  const sides = [
    { d: inner.x - outer.x, near: [inner.x, icy], far: [outer.x, icy] },
    {
      d: outer.x + outer.width - (inner.x + inner.width),
      near: [inner.x + inner.width, icy],
      far: [outer.x + outer.width, icy],
    },
    { d: inner.y - outer.y + TOP_PENALTY, near: [icx, inner.y], far: [icx, outer.y] },
    {
      d: outer.y + outer.height - (inner.y + inner.height),
      near: [icx, inner.y + inner.height],
      far: [icx, outer.y + outer.height],
    },
  ] as const;
  const best = sides.reduce((m, s) => (s.d < m.d ? s : m));
  const near: Point = [best.near[0], best.near[1]];
  const far: Point = [best.far[0], best.far[1]];
  return aInner ? [near, far] : [far, near];
}

/** Orthogonal route from the border of `a` to the border of `b`; `idx` spreads parallel connectors. */
export function routeEdge(a: Box, b: Box, idx: number): Point[] {
  const aRight = a.x + a.width;
  const aBottom = a.y + a.height;
  const bRight = b.x + b.width;
  const bBottom = b.y + b.height;
  const toRight = b.x - aRight;
  const toLeft = a.x - bRight;
  const toDown = b.y - aBottom;
  const toUp = a.y - bBottom;
  const hGap = Math.max(toRight, toLeft);
  const vGap = Math.max(toDown, toUp);
  if (hGap < 0 && vGap < 0) return routeNested(a, b);

  const acx = a.x + a.width / 2;
  const acy = a.y + a.height / 2;
  const bcx = b.x + b.width / 2;
  const bcy = b.y + b.height / 2;
  // Dominant axis by centre distance, but only an axis whose sides really face each other can carry the line.
  let horizontal = Math.abs(bcx - acx) >= Math.abs(bcy - acy);
  if (horizontal && hGap < 0) horizontal = false;
  else if (!horizontal && vGap < 0) horizontal = true;
  const grouped = a.group === true || b.group === true;

  if (horizontal) {
    const goRight = toRight >= 0;
    const lo = Math.max(a.y, b.y);
    const hi = Math.min(aBottom, bBottom);
    const straight = grouped && hi > lo;
    const y0 = straight ? (lo + hi) / 2 : acy;
    const y1 = straight ? (lo + hi) / 2 : bcy;
    const p0: Point = [goRight ? aRight : a.x, y0];
    const p1: Point = [goRight ? b.x : bRight, y1];
    const mx = (p0[0] + p1[0]) / 2 + ((idx % 3) - 1) * Math.min(6, hGap / 3);
    return Math.abs(y0 - y1) < 2 ? [p0, p1] : [p0, [mx, y0], [mx, y1], p1];
  }
  const goDown = toDown >= 0;
  const lo = Math.max(a.x, b.x);
  const hi = Math.min(aRight, bRight);
  const straight = grouped && hi > lo;
  const x0 = straight ? (lo + hi) / 2 : acx;
  const x1 = straight ? (lo + hi) / 2 : bcx;
  const p0: Point = [x0, goDown ? aBottom : a.y];
  const p1: Point = [x1, goDown ? b.y : bBottom];
  const my = (p0[1] + p1[1]) / 2 + ((idx % 3) - 1) * Math.min(6, vGap / 3);
  return Math.abs(x0 - x1) < 2 ? [p0, p1] : [p0, [x0, my], [x1, my], p1];
}
