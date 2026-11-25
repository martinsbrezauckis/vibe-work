/**
 * Pure-SVG shape and directional block-arrow geometry for block diagrams.
 */
import { circle, el, line, path, polygon, rect, type Attrs } from "../../svg";
import { color } from "../../theme";
import type { ArrowDir, BlockShape } from "./parse";

const ellipse = (cx: number, cy: number, rx: number, ry: number, attrs: Attrs = {}): string =>
  el("ellipse", { cx, cy, rx, ry, ...attrs });

/** Extra horizontal and vertical padding a leaf shape needs around its text box. */
export function shapePadding(shape: BlockShape): { px: number; py: number } {
  switch (shape) {
    case "rhombus":
      return { px: 28, py: 18 };
    case "hexagon":
    case "parallelogram-l":
    case "parallelogram-r":
    case "trapezoid":
    case "trapezoid-alt":
      return { px: 24, py: 12 };
    case "cylinder":
      return { px: 16, py: 16 };
    case "circle":
    case "doublecircle":
      return { px: 18, py: 18 };
    case "subroutine":
    case "asymmetric":
    case "stadium":
      return { px: 20, py: 12 };
    default:
      return { px: 14, py: 10 };
  }
}

/** Draw one leaf block shape in box `(x, y, w, h)` with tint fill and border stroke. */
export function drawLeafShape(
  shape: BlockShape,
  x: number,
  y: number,
  w: number,
  h: number,
  tint: string,
): string {
  const cx = x + w / 2;
  const cy = y + h / 2;
  const fillAttrs: Attrs = { fill: tint, "fill-opacity": 0.14 };
  const strokeAttrs: Attrs = { fill: "none", stroke: color.nodeStroke, "stroke-width": 1.2 };
  const both = (pts: readonly [number, number][]): string =>
    polygon(pts, fillAttrs) + polygon(pts, strokeAttrs);

  switch (shape) {
    case "round":
      return (
        rect(x, y, w, h, { rx: 12, ...fillAttrs }) + rect(x, y, w, h, { rx: 12, ...strokeAttrs })
      );
    case "stadium":
      return (
        rect(x, y, w, h, { rx: h / 2, ...fillAttrs }) +
        rect(x, y, w, h, { rx: h / 2, ...strokeAttrs })
      );
    case "subroutine":
      return (
        rect(x, y, w, h, { rx: 3, ...fillAttrs }) +
        rect(x, y, w, h, { rx: 3, ...strokeAttrs }) +
        line(x + 8, y, x + 8, y + h, strokeAttrs) +
        line(x + w - 8, y, x + w - 8, y + h, strokeAttrs)
      );
    case "cylinder": {
      const ry = Math.min(8, h * 0.18);
      const rx = w / 2;
      const body = `M${String(x)},${String(y + ry)} V${String(y + h - ry)} A${String(rx)},${String(ry)} 0 0 0 ${String(x + w)},${String(y + h - ry)} V${String(y + ry)} A${String(rx)},${String(ry)} 0 0 1 ${String(x)},${String(y + ry)} Z`;
      return (
        path(body, fillAttrs) +
        ellipse(cx, y + ry, rx, ry, fillAttrs) +
        path(
          `M${String(x)},${String(y + ry)} V${String(y + h - ry)} A${String(rx)},${String(ry)} 0 0 0 ${String(x + w)},${String(y + h - ry)} V${String(y + ry)}`,
          strokeAttrs,
        ) +
        ellipse(cx, y + ry, rx, ry, strokeAttrs)
      );
    }
    case "circle": {
      const r = Math.min(w, h) / 2;
      return circle(cx, cy, r, fillAttrs) + circle(cx, cy, r, strokeAttrs);
    }
    case "doublecircle": {
      const r = Math.min(w, h) / 2;
      return (
        circle(cx, cy, r, fillAttrs) +
        circle(cx, cy, r, strokeAttrs) +
        circle(cx, cy, Math.max(4, r - 5), strokeAttrs)
      );
    }
    case "asymmetric": {
      const n = Math.min(14, w * 0.2);
      return both([
        [x, y],
        [x + w, y],
        [x + w, y + h],
        [x, y + h],
        [x + n, cy],
      ]);
    }
    case "rhombus":
      return both([
        [cx, y],
        [x + w, cy],
        [cx, y + h],
        [x, cy],
      ]);
    case "hexagon": {
      const cut = Math.min(14, w * 0.2);
      return both([
        [x + cut, y],
        [x + w - cut, y],
        [x + w, cy],
        [x + w - cut, y + h],
        [x + cut, y + h],
        [x, cy],
      ]);
    }
    case "parallelogram-r": {
      const slant = Math.min(14, w * 0.2);
      return both([
        [x + slant, y],
        [x + w, y],
        [x + w - slant, y + h],
        [x, y + h],
      ]);
    }
    case "parallelogram-l": {
      const slant = Math.min(14, w * 0.2);
      return both([
        [x, y],
        [x + w - slant, y],
        [x + w, y + h],
        [x + slant, y + h],
      ]);
    }
    case "trapezoid": {
      const inset = Math.min(14, w * 0.2);
      return both([
        [x + inset, y],
        [x + w - inset, y],
        [x + w, y + h],
        [x, y + h],
      ]);
    }
    case "trapezoid-alt": {
      const inset = Math.min(14, w * 0.2);
      return both([
        [x, y],
        [x + w, y],
        [x + w - inset, y + h],
        [x + inset, y + h],
      ]);
    }
    default:
      return (
        rect(x, y, w, h, { rx: 4, ...fillAttrs }) + rect(x, y, w, h, { rx: 4, ...strokeAttrs })
      );
  }
}

/** Draw a directional block arrow polygon inside box `(x, y, w, h)`. */
export function drawBlockArrow(
  dirs: readonly ArrowDir[],
  x: number,
  y: number,
  w: number,
  h: number,
  tint: string,
): string {
  const cx = x + w / 2;
  const cy = y + h / 2;
  const set = new Set(dirs);
  const goL = set.has("left") || set.has("x");
  const goR = set.has("right") || set.has("x");
  const goU = set.has("up") || set.has("y");
  const goD = set.has("down") || set.has("y");

  const hw = Math.min(14, w * 0.25);
  const hh = Math.min(12, h * 0.28);
  const sy = h * 0.24;
  const sx = Math.min(16, w * 0.22);

  let pts: [number, number][];
  if (!goU && !goD) {
    const leftX = goL ? x + hw : x;
    const rightX = goR ? x + w - hw : x + w;
    pts = [
      [leftX, cy - sy],
      [rightX, cy - sy],
      ...(goR
        ? ([
            [rightX, y + 4],
            [x + w, cy],
            [rightX, y + h - 4],
          ] as [number, number][])
        : []),
      [rightX, cy + sy],
      [leftX, cy + sy],
      ...(goL
        ? ([
            [leftX, y + h - 4],
            [x, cy],
            [leftX, y + 4],
          ] as [number, number][])
        : []),
    ];
  } else if (!goL && !goR) {
    const topY = goU ? y + hh : y;
    const botY = goD ? y + h - hh : y + h;
    pts = [
      ...(goU
        ? ([
            [cx, y],
            [cx + sx + 6, topY],
          ] as [number, number][])
        : [[cx + sx, topY] as [number, number]]),
      [cx + sx, topY],
      [cx + sx, botY],
      ...(goD
        ? ([
            [cx + sx + 6, botY],
            [cx, y + h],
            [cx - sx - 6, botY],
          ] as [number, number][])
        : []),
      [cx - sx, botY],
      [cx - sx, topY],
      ...(goU ? ([[cx - sx - 6, topY]] as [number, number][]) : []),
    ];
  } else {
    // Combined horizontal + vertical (e.g. `(x, down)`): T/cross arrow polygon.
    const armY = cy - Math.min(10, h * 0.16);
    const armB = cy + Math.min(10, h * 0.16);
    const lx = goL ? x + hw : cx - sx;
    const rx = goR ? x + w - hw : cx + sx;
    pts = [
      ...(goU
        ? ([
            [cx, y],
            [cx + sx + 4, y + hh],
          ] as [number, number][])
        : []),
      [cx + sx, goU ? y + hh : armY],
      [rx, armY],
      ...(goR
        ? ([
            [rx, armY - 6],
            [x + w, cy],
            [rx, armB + 6],
          ] as [number, number][])
        : []),
      [rx, armB],
      [cx + sx, armB],
      ...(goD
        ? ([
            [cx + sx, y + h - hh],
            [cx + sx + 4, y + h - hh],
            [cx, y + h],
            [cx - sx - 4, y + h - hh],
            [cx - sx, y + h - hh],
          ] as [number, number][])
        : [[cx - sx, armB] as [number, number]]),
      [cx - sx, armB],
      [lx, armB],
      ...(goL
        ? ([
            [lx, armB + 6],
            [x, cy],
            [lx, armY - 6],
          ] as [number, number][])
        : []),
      [lx, armY],
      [cx - sx, armY],
      ...(goU
        ? ([
            [cx - sx, y + hh],
            [cx - sx - 4, y + hh],
          ] as [number, number][])
        : []),
    ];
  }

  return (
    polygon(pts, { fill: tint, "fill-opacity": 0.18 }) +
    polygon(pts, {
      fill: "none",
      stroke: color.nodeStroke,
      "stroke-width": 1.2,
      "stroke-linejoin": "round",
    })
  );
}
