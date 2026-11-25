/**
 * Node measurement, pure-SVG shape drawing, and exact boundary ray clipping for all 46 supported
 * flowchart shapes. Uses only svg.ts primitives and theme.ts colour tokens.
 */
import { measureLines, wrapText } from "../../measure";
import { circle, el, line, n, path, polygon, rect, textLines } from "../../svg";
import { FONT, color } from "../../theme";
import type { FlowDirection, FlowNode, FlowShape } from "./types";

const LINE_H = 17;
const LABEL_STYLE = { size: FONT.body, bold: true } as const;
const CAPTION_STYLE = { size: FONT.small, bold: false } as const;

/** Computed layout box and wrapped label rows for a flowchart node. */
export interface MeasuredNode {
  node: FlowNode;
  width: number;
  height: number;
  lines: string[];
  /** True when the node is drawn as a compact symbolic glyph with an optional caption below. */
  glyphWithCaption: boolean;
  glyphSize: number;
}

const GLYPH_SHAPES = new Set<FlowShape>([
  "sm-circ",
  "f-circ",
  "framed-circle",
  "cross-circ",
  "fork",
  "hourglass",
  "bolt",
]);

/** Measure a node's outer bounding box in user units for Dagre layout. */
export function measureNodeShape(node: FlowNode, direction: FlowDirection): MeasuredNode {
  const shape = node.shape;
  if (GLYPH_SHAPES.has(shape)) {
    const horizontal = direction === "LR" || direction === "RL";
    const glyphW = shape === "fork" ? (horizontal ? 10 : 76) : shape === "bolt" ? 36 : 32;
    const glyphH = shape === "fork" ? (horizontal ? 72 : 10) : shape === "bolt" ? 36 : 32;
    const lines = node.hasExplicitLabel ? wrapText(node.label, 140, CAPTION_STYLE, 2) : [];
    const cap = lines.length ? measureLines(lines, CAPTION_STYLE, 1.25) : { width: 0, height: 0 };
    const width = Math.max(glyphW + 8, cap.width + 12);
    const height = glyphH + (lines.length ? 6 + cap.height : 4);
    return {
      node,
      width,
      height,
      lines,
      glyphWithCaption: true,
      glyphSize: Math.max(glyphW, glyphH),
    };
  }

  const lines = wrapText(node.label, 200, LABEL_STYLE, 4);
  const measured = measureLines(lines, LABEL_STYLE, 1.28);
  const textW = Math.max(24, measured.width);
  const textH = Math.max(LINE_H, lines.length * LINE_H);

  let padX = 28;
  let padY = 18;
  switch (shape) {
    case "diamond":
      padX = Math.max(44, textW * 0.6 + 24);
      padY = Math.max(30, textH * 0.8 + 16);
      break;
    case "hex":
    case "bow-rect":
    case "curv-trap":
      padX = 44;
      padY = 20;
      break;
    case "subproc":
    case "lin-rect":
    case "lin-doc":
    case "win-pane":
      padX = 42;
      padY = shape === "win-pane" ? 26 : 20;
      break;
    case "lean-r":
    case "lean-l":
    case "trap-b":
    case "trap-t":
    case "stadium":
    case "odd":
    case "delay":
    case "braces":
      padX = 42;
      padY = 20;
      break;
    case "comment":
    case "brace-r":
      padX = 32;
      padY = 18;
      break;
    case "cyl":
    case "lin-cyl":
      padX = 30;
      padY = 30;
      break;
    case "das":
      padX = 42;
      padY = 22;
      break;
    case "doc":
    case "docs":
    case "flag":
    case "tag-doc":
      padX = 32;
      padY = 28;
      break;
    case "div-rect":
    case "sl-rect":
    case "notch-rect":
    case "notch-pent":
    case "tag-rect":
    case "processes":
    case "datastore":
      padX = 32;
      padY = 24;
      break;
    case "tri":
    case "flip-tri": {
      const w = Math.max(84, textW * 1.65 + 34);
      const h = Math.max(66, textH * 1.9 + 26);
      return { node, width: w, height: h, lines, glyphWithCaption: false, glyphSize: 0 };
    }
    case "circle":
    case "dbl-circ": {
      const d = Math.max(54, Math.hypot(textW + 22, textH + 18) + (shape === "dbl-circ" ? 10 : 0));
      return { node, width: d, height: d, lines, glyphWithCaption: false, glyphSize: 0 };
    }
    case "text":
      padX = 16;
      padY = 12;
      break;
    default:
      break;
  }

  return {
    node,
    width: Math.max(shape === "text" ? 36 : 64, textW + padX),
    height: Math.max(shape === "text" ? 28 : 38, textH + padY),
    lines,
    glyphWithCaption: false,
    glyphSize: 0,
  };
}

function braceLeftPath(x: number, y: number, h: number): string {
  const q = h * 0.25;
  const cy = y + h / 2;
  return `M${n(x + 9)},${n(y)} C${n(x + 2)},${n(y)} ${n(x + 4)},${n(y + q)} ${n(x + 4)},${n(cy - 4)} L${n(x)},${n(cy)} L${n(x + 4)},${n(cy + 4)} C${n(x + 4)},${n(y + h - q)} ${n(x + 2)},${n(y + h)} ${n(x + 9)},${n(y + h)}`;
}

function braceRightPath(x: number, y: number, h: number): string {
  const q = h * 0.25;
  const cy = y + h / 2;
  return `M${n(x - 9)},${n(y)} C${n(x - 2)},${n(y)} ${n(x - 4)},${n(y + q)} ${n(x - 4)},${n(cy - 4)} L${n(x)},${n(cy)} L${n(x - 4)},${n(cy + 4)} C${n(x - 4)},${n(y + h - q)} ${n(x - 2)},${n(y + h)} ${n(x - 9)},${n(y + h)}`;
}

function docPath(x: number, y: number, w: number, h: number): string {
  const wave = 6;
  const bot = y + h - wave;
  return `M${n(x)},${n(y)} L${n(x + w)},${n(y)} L${n(x + w)},${n(bot)} Q${n(x + w * 0.75)},${n(bot - wave * 1.4)} ${n(x + w * 0.5)},${n(bot)} Q${n(x + w * 0.25)},${n(bot + wave * 1.4)} ${n(x)},${n(bot)} Z`;
}

function drawGlyphShape(m: MeasuredNode, cx: number, cy: number, direction: FlowDirection): string {
  const shape = m.node.shape;
  const capH = m.lines.length ? m.lines.length * (FONT.small * 1.25) + 6 : 0;
  const gcy = cy - capH / 2;
  const attrs = { fill: color.nodeFill, stroke: color.nodeStroke, "stroke-width": 1.3 };
  let svg = "";

  if (shape === "sm-circ") {
    svg += circle(cx, gcy, 11, { fill: color.text, stroke: color.nodeStroke, "stroke-width": 1 });
  } else if (shape === "f-circ") {
    svg += circle(cx, gcy, 13, { fill: color.text, stroke: color.nodeStroke, "stroke-width": 1.2 });
  } else if (shape === "framed-circle") {
    svg += circle(cx, gcy, 15, {
      fill: color.canvas,
      stroke: color.nodeStroke,
      "stroke-width": 1.5,
    });
    svg += circle(cx, gcy, 9, { fill: color.text });
  } else if (shape === "cross-circ") {
    const r = 14;
    const d = r * 0.7;
    svg += circle(cx, gcy, r, attrs);
    svg += line(cx - d, gcy - d, cx + d, gcy + d, {
      stroke: color.nodeStroke,
      "stroke-width": 1.3,
    });
    svg += line(cx + d, gcy - d, cx - d, gcy + d, {
      stroke: color.nodeStroke,
      "stroke-width": 1.3,
    });
  } else if (shape === "fork") {
    const horizontal = direction === "LR" || direction === "RL";
    const fw = horizontal ? 8 : 74;
    const fh = horizontal ? 70 : 8;
    svg += rect(cx - fw / 2, gcy - fh / 2, fw, fh, { rx: 2, fill: color.text });
  } else if (shape === "hourglass") {
    const hw = 14;
    const hh = 14;
    svg += polygon(
      [
        [cx - hw, gcy - hh],
        [cx + hw, gcy - hh],
        [cx - hw, gcy + hh],
        [cx + hw, gcy + hh],
      ],
      attrs,
    );
  } else if (shape === "bolt") {
    svg += polygon(
      [
        [cx + 3, gcy - 16],
        [cx - 10, gcy + 2],
        [cx, gcy + 2],
        [cx - 3, gcy + 16],
        [cx + 10, gcy - 2],
        [cx, gcy - 2],
      ],
      { fill: color.accent, "fill-opacity": 0.25, stroke: color.nodeStroke, "stroke-width": 1.3 },
    );
  }

  if (m.lines.length > 0) {
    const labelY = cy + (m.height - capH) / 2 + 2;
    svg += textLines(
      cx,
      labelY,
      m.lines,
      { size: FONT.small, anchor: "middle", fill: color.textSec },
      1.25,
    );
  }
  return svg;
}

/** Render a measured flowchart node as passive SVG markup centred at `(cx, cy)`. */
export function drawNodeShape(
  m: MeasuredNode,
  cx: number,
  cy: number,
  direction: FlowDirection,
): string {
  if (m.glyphWithCaption) return drawGlyphShape(m, cx, cy, direction);

  const x = cx - m.width / 2;
  const y = cy - m.height / 2;
  const w = m.width;
  const h = m.height;
  const attrs = { fill: color.nodeFill, stroke: color.nodeStroke, "stroke-width": 1.3 };
  const inner = { stroke: color.nodeStroke, "stroke-width": 1.1 };
  let shapeSvg: string;
  let textOffsetX = 0;
  let textOffsetY = 0;

  switch (m.node.shape) {
    case "rounded":
      shapeSvg = rect(x, y, w, h, { rx: 10, ...attrs });
      break;
    case "stadium":
      shapeSvg = rect(x, y, w, h, { rx: h / 2, ...attrs });
      break;
    case "subproc":
      shapeSvg =
        rect(x, y, w, h, { rx: 2, ...attrs }) +
        line(x + 8, y, x + 8, y + h, inner) +
        line(x + w - 8, y, x + w - 8, y + h, inner);
      break;
    case "lin-rect":
      shapeSvg = rect(x, y, w, h, { rx: 2, ...attrs }) + line(x + 9, y, x + 9, y + h, inner);
      textOffsetX = 4;
      break;
    case "div-rect":
      shapeSvg = rect(x, y, w, h, { rx: 3, ...attrs }) + line(x, y + 10, x + w, y + 10, inner);
      textOffsetY = 4;
      break;
    case "win-pane":
      shapeSvg =
        rect(x, y, w, h, { rx: 2, ...attrs }) +
        line(x + 10, y, x + 10, y + h, inner) +
        line(x, y + 10, x + w, y + 10, inner);
      textOffsetX = 4;
      textOffsetY = 4;
      break;
    case "datastore":
      shapeSvg =
        rect(x, y, w, h, { ...attrs, stroke: "none" }) +
        line(x, y, x + w, y, attrs) +
        line(x, y + 4, x + w, y + 4, inner) +
        line(x, y + h - 4, x + w, y + h - 4, inner) +
        line(x, y + h, x + w, y + h, attrs);
      break;
    case "cyl":
    case "lin-cyl": {
      const ry = 7;
      shapeSvg =
        rect(x, y + ry, w, h - ry * 2, { ...attrs, stroke: "none" }) +
        line(x, y + ry, x, y + h - ry, attrs) +
        line(x + w, y + ry, x + w, y + h - ry, attrs) +
        path(`M${n(x)},${n(y + h - ry)} A${n(w / 2)},${n(ry)} 0 0 0 ${n(x + w)},${n(y + h - ry)}`, {
          fill: color.nodeFill,
          stroke: color.nodeStroke,
          "stroke-width": 1.3,
        }) +
        el("ellipse", { cx, cy: y + ry, rx: w / 2, ry, ...attrs });
      if (m.node.shape === "lin-cyl") {
        shapeSvg += path(
          `M${n(x)},${n(y + ry + 5)} A${n(w / 2)},${n(ry)} 0 0 0 ${n(x + w)},${n(y + ry + 5)}`,
          { fill: "none", ...inner },
        );
        textOffsetY = 5;
      } else {
        textOffsetY = 3;
      }
      break;
    }
    case "das": {
      const rx = 8;
      shapeSvg =
        rect(x + rx, y, w - rx * 2, h, { ...attrs, stroke: "none" }) +
        line(x + rx, y, x + w - rx, y, attrs) +
        line(x + rx, y + h, x + w - rx, y + h, attrs) +
        path(`M${n(x + rx)},${n(y)} A${n(rx)},${n(h / 2)} 0 0 0 ${n(x + rx)},${n(y + h)}`, {
          fill: color.nodeFill,
          stroke: color.nodeStroke,
          "stroke-width": 1.3,
        }) +
        el("ellipse", { cx: x + w - rx, cy, rx, ry: h / 2, ...attrs });
      textOffsetX = -3;
      break;
    }
    case "circle":
      shapeSvg = circle(cx, cy, w / 2, attrs);
      break;
    case "dbl-circ":
      shapeSvg =
        circle(cx, cy, w / 2, attrs) +
        circle(cx, cy, w / 2 - 5, { fill: "none", stroke: color.nodeStroke, "stroke-width": 1.2 });
      break;
    case "odd": {
      const notch = 12;
      shapeSvg = polygon(
        [
          [x, y],
          [x + w, y],
          [x + w, y + h],
          [x, y + h],
          [x + notch, cy],
        ],
        attrs,
      );
      textOffsetX = 4;
      break;
    }
    case "diamond":
      shapeSvg = polygon(
        [
          [cx, y],
          [x + w, cy],
          [cx, y + h],
          [x, cy],
        ],
        attrs,
      );
      break;
    case "hex": {
      const d = 12;
      shapeSvg = polygon(
        [
          [x + d, y],
          [x + w - d, y],
          [x + w, cy],
          [x + w - d, y + h],
          [x + d, y + h],
          [x, cy],
        ],
        attrs,
      );
      break;
    }
    case "lean-r": {
      const s = 12;
      shapeSvg = polygon(
        [
          [x + s, y],
          [x + w, y],
          [x + w - s, y + h],
          [x, y + h],
        ],
        attrs,
      );
      break;
    }
    case "lean-l": {
      const s = 12;
      shapeSvg = polygon(
        [
          [x, y],
          [x + w - s, y],
          [x + w, y + h],
          [x + s, y + h],
        ],
        attrs,
      );
      break;
    }
    case "trap-b": {
      const s = 12;
      shapeSvg = polygon(
        [
          [x + s, y],
          [x + w - s, y],
          [x + w, y + h],
          [x, y + h],
        ],
        attrs,
      );
      break;
    }
    case "trap-t": {
      const s = 12;
      shapeSvg = polygon(
        [
          [x, y],
          [x + w, y],
          [x + w - s, y + h],
          [x + s, y + h],
        ],
        attrs,
      );
      break;
    }
    case "notch-rect": {
      const c = 11;
      shapeSvg = polygon(
        [
          [x + c, y],
          [x + w, y],
          [x + w, y + h],
          [x, y + h],
          [x, y + c],
        ],
        attrs,
      );
      break;
    }
    case "notch-pent": {
      const c = 11;
      shapeSvg = polygon(
        [
          [x + c, y],
          [x + w - c, y],
          [x + w, y + c],
          [x + w, y + h],
          [x, y + h],
          [x, y + c],
        ],
        attrs,
      );
      textOffsetY = 2;
      break;
    }
    case "sl-rect": {
      const drop = 10;
      shapeSvg = polygon(
        [
          [x, y + drop],
          [x + w, y],
          [x + w, y + h],
          [x, y + h],
        ],
        attrs,
      );
      textOffsetY = 3;
      break;
    }
    case "tri":
      shapeSvg = polygon(
        [
          [cx, y],
          [x + w, y + h],
          [x, y + h],
        ],
        attrs,
      );
      textOffsetY = h * 0.14;
      break;
    case "flip-tri":
      shapeSvg = polygon(
        [
          [x, y],
          [x + w, y],
          [cx, y + h],
        ],
        attrs,
      );
      textOffsetY = -h * 0.14;
      break;
    case "doc":
      shapeSvg = path(docPath(x, y, w, h), attrs);
      textOffsetY = -2;
      break;
    case "lin-doc":
      shapeSvg = path(docPath(x, y, w, h), attrs) + line(x + 9, y, x + 9, y + h - 5, inner);
      textOffsetX = 4;
      textOffsetY = -2;
      break;
    case "docs": {
      const off = 5;
      shapeSvg =
        path(docPath(x + off, y - off, w - off, h - off), attrs) +
        path(docPath(x + off / 2, y - off / 2, w - off, h - off), attrs) +
        path(docPath(x, y + off / 2, w - off, h - off), attrs);
      textOffsetX = -2;
      break;
    }
    case "processes": {
      const off = 5;
      shapeSvg =
        rect(x + off, y - off, w - off, h - off, { rx: 2, ...attrs }) +
        rect(x + off / 2, y - off / 2, w - off, h - off, { rx: 2, ...attrs }) +
        rect(x, y + off / 2, w - off, h - off, { rx: 2, ...attrs });
      textOffsetX = -2;
      textOffsetY = 1;
      break;
    }
    case "tag-doc":
      shapeSvg =
        path(docPath(x, y, w, h), attrs) +
        polygon(
          [
            [x + w - 12, y + h - 6],
            [x + w, y + h - 18],
            [x + w, y + h - 6],
          ],
          { fill: color.groupHdr, stroke: color.nodeStroke, "stroke-width": 1 },
        );
      textOffsetY = -2;
      break;
    case "tag-rect":
      shapeSvg =
        rect(x, y, w, h, { rx: 2, ...attrs }) +
        polygon(
          [
            [x + w - 12, y + h],
            [x + w, y + h - 12],
            [x + w, y + h],
          ],
          { fill: color.groupHdr, stroke: color.nodeStroke, "stroke-width": 1 },
        );
      break;
    case "delay": {
      const r = Math.min(h / 2, w * 0.38);
      shapeSvg = path(
        `M${n(x)},${n(y)} L${n(x + w - r)},${n(y)} A${n(r)},${n(h / 2)} 0 0 1 ${n(x + w - r)},${n(y + h)} L${n(x)},${n(y + h)} Z`,
        attrs,
      );
      break;
    }
    case "curv-trap": {
      const tip = 14;
      const r = Math.min(h / 2, 16);
      shapeSvg = path(
        `M${n(x + tip)},${n(y)} L${n(x + w - r)},${n(y)} A${n(r)},${n(h / 2)} 0 0 1 ${n(x + w - r)},${n(y + h)} L${n(x + tip)},${n(y + h)} L${n(x)},${n(cy)} Z`,
        attrs,
      );
      textOffsetX = 3;
      break;
    }
    case "flag": {
      const wave = 5;
      shapeSvg = path(
        `M${n(x)},${n(y + wave)} Q${n(x + w * 0.25)},${n(y - wave)} ${n(cx)},${n(y + wave)} Q${n(x + w * 0.75)},${n(y + wave * 2.2)} ${n(x + w)},${n(y + wave)} L${n(x + w)},${n(y + h - wave)} Q${n(x + w * 0.75)},${n(y + h)} ${n(cx)},${n(y + h - wave)} Q${n(x + w * 0.25)},${n(y + h - wave * 2.2)} ${n(x)},${n(y + h - wave)} Z`,
        attrs,
      );
      break;
    }
    case "bow-rect": {
      const curve = 10;
      shapeSvg = path(
        `M${n(x + curve)},${n(y)} L${n(x + w)},${n(y)} Q${n(x + w - curve * 1.3)},${n(cy)} ${n(x + w)},${n(y + h)} L${n(x + curve)},${n(y + h)} Q${n(x - curve * 0.4)},${n(cy)} ${n(x + curve)},${n(y)} Z`,
        attrs,
      );
      textOffsetX = 2;
      break;
    }
    case "comment":
      shapeSvg = path(braceLeftPath(x + 2, y, h), {
        fill: "none",
        stroke: color.nodeStroke,
        "stroke-width": 1.4,
      });
      textOffsetX = 4;
      break;
    case "brace-r":
      shapeSvg = path(braceRightPath(x + w - 2, y, h), {
        fill: "none",
        stroke: color.nodeStroke,
        "stroke-width": 1.4,
      });
      textOffsetX = -4;
      break;
    case "braces":
      shapeSvg =
        path(braceLeftPath(x + 2, y, h), {
          fill: "none",
          stroke: color.nodeStroke,
          "stroke-width": 1.4,
        }) +
        path(braceRightPath(x + w - 2, y, h), {
          fill: "none",
          stroke: color.nodeStroke,
          "stroke-width": 1.4,
        });
      break;
    case "text":
      shapeSvg = "";
      break;
    case "rect":
    default:
      shapeSvg = rect(x, y, w, h, { rx: 4, ...attrs });
      break;
  }

  const startY = cy + textOffsetY - ((m.lines.length - 1) * LINE_H) / 2;
  return (
    shapeSvg +
    textLines(
      cx + textOffsetX,
      startY,
      m.lines,
      { size: FONT.body, weight: "600", anchor: "middle" },
      LINE_H / FONT.body,
    )
  );
}

function intersectPoly(
  cx: number,
  cy: number,
  tx: number,
  ty: number,
  poly: readonly [number, number][],
): [number, number] {
  const dx = tx - cx;
  const dy = ty - cy;
  let bestT = Number.POSITIVE_INFINITY;
  for (let i = 0; i < poly.length; i++) {
    const a = poly[i] ?? [cx, cy];
    const b = poly[(i + 1) % poly.length] ?? [cx, cy];
    const sx = b[0] - a[0];
    const sy = b[1] - a[1];
    const denom = dx * sy - dy * sx;
    if (Math.abs(denom) < 1e-9) continue;
    const t = ((a[0] - cx) * sy - (a[1] - cy) * sx) / denom;
    const u = ((a[0] - cx) * dy - (a[1] - cy) * dx) / denom;
    if (t > 0 && u >= -1e-6 && u <= 1 + 1e-6 && t < bestT) bestT = t;
  }
  if (!Number.isFinite(bestT)) return [cx, cy];
  return [cx + dx * bestT, cy + dy * bestT];
}

/** Clip the ray from `(cx, cy)` toward `(tx, ty)` against the node's drawn outline. */
export function clipShapePoint(
  m: MeasuredNode,
  cx: number,
  cy: number,
  tx: number,
  ty: number,
): [number, number] {
  const dx = tx - cx;
  const dy = ty - cy;
  const dist = Math.hypot(dx, dy);
  if (dist < 1e-6) return [cx, cy];

  const x = cx - m.width / 2;
  const y = cy - m.height / 2;
  const w = m.width;
  const h = m.height;
  const s = m.node.shape;

  if (s === "circle" || s === "dbl-circ") {
    const r = w / 2;
    return [cx + (dx / dist) * r, cy + (dy / dist) * r];
  }
  if (s === "diamond") {
    return intersectPoly(cx, cy, tx, ty, [
      [cx, y],
      [x + w, cy],
      [cx, y + h],
      [x, cy],
    ]);
  }
  if (s === "hex") {
    const d = 12;
    return intersectPoly(cx, cy, tx, ty, [
      [x + d, y],
      [x + w - d, y],
      [x + w, cy],
      [x + w - d, y + h],
      [x + d, y + h],
      [x, cy],
    ]);
  }
  if (s === "tri") {
    return intersectPoly(cx, cy, tx, ty, [
      [cx, y],
      [x + w, y + h],
      [x, y + h],
    ]);
  }
  if (s === "flip-tri") {
    return intersectPoly(cx, cy, tx, ty, [
      [x, y],
      [x + w, y],
      [cx, y + h],
    ]);
  }
  if (s === "lean-r" || s === "lean-l" || s === "trap-b" || s === "trap-t") {
    const k = 12;
    const poly: [number, number][] =
      s === "lean-r"
        ? [
            [x + k, y],
            [x + w, y],
            [x + w - k, y + h],
            [x, y + h],
          ]
        : s === "lean-l"
          ? [
              [x, y],
              [x + w - k, y],
              [x + w, y + h],
              [x + k, y + h],
            ]
          : s === "trap-b"
            ? [
                [x + k, y],
                [x + w - k, y],
                [x + w, y + h],
                [x, y + h],
              ]
            : [
                [x, y],
                [x + w, y],
                [x + w - k, y + h],
                [x + k, y + h],
              ];
    return intersectPoly(cx, cy, tx, ty, poly);
  }
  const sx = Math.abs(dx) > 1e-6 ? w / 2 / Math.abs(dx) : Number.POSITIVE_INFINITY;
  const sy = Math.abs(dy) > 1e-6 ? h / 2 / Math.abs(dy) : Number.POSITIVE_INFINITY;
  const t = Math.min(sx, sy);
  return [cx + dx * t, cy + dy * t];
}
