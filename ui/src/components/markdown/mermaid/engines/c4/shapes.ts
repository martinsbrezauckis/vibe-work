/**
 * Pure-SVG shape rendering for C4 elements (Person silhouette, Box, Cylinder DB, Queue pipe) and
 * C4 scopes (Boundaries and Deployment Nodes with alignment).
 */
import { clampLabel, fitWidth, measureLines, wrapText } from "../../measure";
import { circle, el, line, path, rect, text, textLines, type Attrs } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { C4Element, C4Scope } from "./parse";

const CARD_MIN_W = 176;
const CARD_MAX_W = 224;
const CARD_PAD_X = 14;
const TITLE_STYLE = { size: FONT.body, bold: true };
const SUB_STYLE = { size: FONT.small };
const DESCR_STYLE = { size: FONT.small + 1 };

const ellipse = (cx: number, cy: number, rx: number, ry: number, attrs: Attrs = {}): string =>
  el("ellipse", { cx, cy, rx, ry, ...attrs });

/** Pre-measured box metrics for one C4 leaf element. */
export interface ElementMetrics {
  width: number;
  height: number;
  stereotype: string;
  titleLines: string[];
  technLine: string;
  descrLines: string[];
}

function roleAccent(role: C4Element["role"], external: boolean): string {
  if (external) return color.textMuted;
  switch (role) {
    case "person":
      return series(0);
    case "system":
      return series(1);
    case "container":
      return series(4);
    case "component":
      return series(6);
  }
}

function stereotypeOf(elItem: C4Element): string {
  const prefix = elItem.external ? "external " : "";
  return `«${prefix}${elItem.role}»`;
}

/** Measure text wrapping and outer card dimensions for one C4 element. */
export function measureC4Element(elItem: C4Element): ElementMetrics {
  const innerW = CARD_MAX_W - CARD_PAD_X * 2;
  const stereotype = stereotypeOf(elItem);
  const titleLines = wrapText(clampLabel(elItem.label), innerW, TITLE_STYLE, 2);
  const technLine = elItem.techn
    ? `[${fitWidth(clampLabel(elItem.techn), innerW, SUB_STYLE)}]`
    : "";
  const descrLines = elItem.descr ? wrapText(clampLabel(elItem.descr), innerW, DESCR_STYLE, 4) : [];

  const contentW = Math.max(
    measureLines([stereotype], SUB_STYLE).width,
    measureLines(titleLines, TITLE_STYLE).width,
    technLine ? measureLines([technLine], SUB_STYLE).width : 0,
    descrLines.length ? measureLines(descrLines, DESCR_STYLE).width : 0,
  );
  const width = Math.min(CARD_MAX_W, Math.max(CARD_MIN_W, contentW + CARD_PAD_X * 2));

  const topPad = elItem.shape === "person" ? 34 : elItem.shape === "db" ? 20 : 14;
  const botPad = elItem.shape === "db" ? 16 : 12;
  const titleH = titleLines.length * 18;
  const stereoH = 15;
  const technH = technLine ? 16 : 0;
  const descrH = descrLines.length > 0 ? descrLines.length * 16 + 6 : 0;
  const height = topPad + titleH + stereoH + technH + descrH + botPad;

  return { width, height, stereotype, titleLines, technLine, descrLines };
}

/** Draw one C4 leaf element inside box `(x, y, width, height)`. */
export function drawC4Element(
  elItem: C4Element,
  metrics: ElementMetrics,
  x: number,
  y: number,
): string {
  const { width: w, height: h } = metrics;
  const cx = x + w / 2;
  const accent = roleAccent(elItem.role, elItem.external);
  const fillAttrs: Attrs = {
    fill: elItem.external ? color.groupFill : accent,
    "fill-opacity": elItem.external ? 0.9 : 0.14,
  };
  const strokeAttrs: Attrs = {
    fill: "none",
    stroke: elItem.external ? color.textMuted : color.nodeStroke,
    "stroke-width": 1.3,
    "stroke-dasharray": elItem.external ? "5 3" : undefined,
  };

  let shell = "";
  let textStartY: number;

  if (elItem.shape === "person") {
    const headR = 13;
    const headCy = y + headR + 2;
    const bodyTop = y + 26;
    const bodyH = h - 26;
    shell += rect(x, bodyTop, w, bodyH, { rx: 12, ...fillAttrs });
    shell += rect(x, bodyTop, w, bodyH, { rx: 12, ...strokeAttrs });
    shell += circle(cx, headCy, headR, { fill: color.canvas });
    shell += circle(cx, headCy, headR, fillAttrs);
    shell += circle(cx, headCy, headR, strokeAttrs);
    textStartY = bodyTop + 16;
  } else if (elItem.shape === "db") {
    const ry = 9;
    const rx = w / 2;
    const bodyD = `M${String(x)},${String(y + ry)} V${String(y + h - ry)} A${String(rx)},${String(ry)} 0 0 0 ${String(x + w)},${String(y + h - ry)} V${String(y + ry)} A${String(rx)},${String(ry)} 0 0 1 ${String(x)},${String(y + ry)} Z`;
    shell += path(bodyD, fillAttrs);
    shell += ellipse(cx, y + ry, rx, ry, fillAttrs);
    shell += path(
      `M${String(x)},${String(y + ry)} V${String(y + h - ry)} A${String(rx)},${String(ry)} 0 0 0 ${String(x + w)},${String(y + h - ry)} V${String(y + ry)}`,
      strokeAttrs,
    );
    shell += ellipse(cx, y + ry, rx, ry, strokeAttrs);
    textStartY = y + ry * 2 + 10;
  } else if (elItem.shape === "queue") {
    const rx = 10;
    const ry = h / 2;
    const bodyD = `M${String(x + rx)},${String(y)} H${String(x + w - rx)} A${String(rx)},${String(ry)} 0 0 1 ${String(x + w - rx)},${String(y + h)} H${String(x + rx)} A${String(rx)},${String(ry)} 0 0 1 ${String(x + rx)},${String(y)} Z`;
    shell += path(bodyD, fillAttrs);
    shell += path(bodyD, strokeAttrs);
    shell += path(
      `M${String(x + w - rx)},${String(y)} A${String(rx)},${String(ry)} 0 0 0 ${String(x + w - rx)},${String(y + h)}`,
      strokeAttrs,
    );
    textStartY = y + 18;
  } else {
    shell += rect(x, y, w, h, { rx: 6, ...fillAttrs });
    shell += rect(x, y, w, h, { rx: 6, ...strokeAttrs });
    textStartY = y + 18;
  }

  let cursorY = textStartY;
  let content = textLines(
    cx,
    cursorY,
    metrics.titleLines,
    { size: FONT.body, weight: "600", anchor: "middle" },
    18 / FONT.body,
  );
  cursorY += metrics.titleLines.length * 18;

  if (metrics.technLine) {
    content += text(cx, cursorY, metrics.technLine, {
      size: FONT.small,
      anchor: "middle",
      fill: color.textSec,
    });
    cursorY += 15;
  }

  content += text(cx, cursorY, metrics.stereotype, {
    size: FONT.small,
    anchor: "middle",
    fill: color.textMuted,
  });
  cursorY += 16;

  if (metrics.descrLines.length > 0) {
    content += textLines(
      cx,
      cursorY + 3,
      metrics.descrLines,
      { size: FONT.small + 1, anchor: "middle", fill: color.textSec },
      16 / (FONT.small + 1),
    );
  }

  return shell + content;
}

/** Draw a C4 boundary or deployment node frame around `(x, y, width, height)`. */
export function drawC4ScopeFrame(
  scope: C4Scope,
  x: number,
  y: number,
  width: number,
  height: number,
): string {
  const isNode = scope.scopeType === "node";
  const label = fitWidth(
    clampLabel(scope.label.replace(/<br\s*\/?>/gi, " ")),
    width - 24,
    TITLE_STYLE,
  );
  const typeTag = scope.typeLabel
    ? `[${fitWidth(clampLabel(scope.typeLabel.replace(/<br\s*\/?>/gi, " ")), width - 24, SUB_STYLE)}]`
    : "";

  if (isNode) {
    const hdrH = scope.descr ? 48 : 34;
    const anchor = scope.align === "left" ? "start" : scope.align === "right" ? "end" : "middle";
    const tx =
      scope.align === "left" ? x + 12 : scope.align === "right" ? x + width - 12 : x + width / 2;
    let out = rect(x, y, width, height, {
      rx: 6,
      fill: color.groupFill,
      stroke: color.nodeStroke,
      "stroke-width": 1.3,
    });
    out += rect(x, y, width, hdrH, { rx: 6, fill: color.groupHdr });
    out += rect(x, y + hdrH - 6, width, 6, { fill: color.groupHdr });
    out += line(x, y + hdrH, x + width, y + hdrH, { stroke: color.innerStroke, "stroke-width": 1 });
    out += text(tx, y + 13, label, { size: FONT.body, weight: "600", anchor });
    if (typeTag) {
      out += text(tx, y + 27, typeTag, { size: FONT.small, anchor, fill: color.textSec });
    }
    if (scope.descr) {
      out += text(tx, y + 40, fitWidth(clampLabel(scope.descr), width - 24, SUB_STYLE), {
        size: FONT.small,
        anchor,
        fill: color.textMuted,
      });
    }
    return out;
  }

  // Dashed boundary frame with title and [type] footer/header tag.
  let out = rect(x, y, width, height, {
    rx: 8,
    fill: color.groupFill,
    stroke: color.innerStroke,
    "stroke-width": 1.3,
    "stroke-dasharray": "6 4",
  });
  out += text(x + 12, y + 15, label, {
    size: FONT.body,
    weight: "600",
    fill: color.textSec,
  });
  if (typeTag) {
    out += text(x + 12, y + 29, typeTag, {
      size: FONT.small,
      fill: color.textMuted,
    });
  }
  return out;
}
