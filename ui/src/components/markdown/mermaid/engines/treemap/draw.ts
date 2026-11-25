/**
 * Treemap diagram drawing. Tiles hierarchical sections and leaves using the squarified treemap
 * algorithm inside fixed outer bounds, using branch palette tokens with nested fill-opacity and
 * deterministic comma-formatted numeric values.
 */
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth } from "../../measure";
import { g, rect, svgDoc, text } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { TreemapModel, TreemapNode } from "./parse";

const CANVAS_W = 860;
const CANVAS_H = 540;
const PAD = 16;
const TILE_GAP = 4;
const HDR_H = 22;

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface SizedItem {
  node: TreemapNode;
  area: number;
}

function formatNumber(value: number): string {
  const rounded = Math.round(value * 1_000_000) / 1_000_000;
  const parts = String(rounded).split(".");
  const intPart = (parts[0] ?? "0").replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return parts[1] !== undefined ? `${intPart}.${parts[1]}` : intPart;
}

function worstAspect(areas: number[], side: number): number {
  if (areas.length === 0 || side <= 0) return Number.POSITIVE_INFINITY;
  let sum = 0;
  let minA = Number.POSITIVE_INFINITY;
  let maxA = 0;
  for (const a of areas) {
    sum += a;
    minA = Math.min(minA, a);
    maxA = Math.max(maxA, a);
  }
  if (sum <= 0 || minA <= 0) return Number.POSITIVE_INFINITY;
  const s2 = side * side;
  return Math.max((s2 * maxA) / (sum * sum), (sum * sum) / (s2 * minA));
}

function layoutStrip(
  row: SizedItem[],
  box: Box,
  horizontalSplit: boolean,
  out: { node: TreemapNode; box: Box }[],
): Box {
  const rowSum = row.reduce((s, item) => s + item.area, 0);
  if (horizontalSplit) {
    const stripW = box.h > 0 ? rowSum / box.h : 0;
    let cy = box.y;
    for (const item of row) {
      const ih = stripW > 0 ? item.area / stripW : 0;
      out.push({ node: item.node, box: { x: box.x, y: cy, w: stripW, h: ih } });
      cy += ih;
    }
    return { x: box.x + stripW, y: box.y, w: Math.max(0, box.w - stripW), h: box.h };
  }
  const stripH = box.w > 0 ? rowSum / box.w : 0;
  let cx = box.x;
  for (const item of row) {
    const iw = stripH > 0 ? item.area / stripH : 0;
    out.push({ node: item.node, box: { x: cx, y: box.y, w: iw, h: stripH } });
    cx += iw;
  }
  return { x: box.x, y: box.y + stripH, w: box.w, h: Math.max(0, box.h - stripH) };
}

function squarify(
  nodes: TreemapNode[],
  container: Box,
  budget: StepBudget,
): { node: TreemapNode; box: Box }[] {
  const positive = nodes
    .map((node, index) => ({ node, index }))
    .filter((item) => item.node.value > 0)
    .sort((a, b) => b.node.value - a.node.value || a.index - b.index);
  if (positive.length === 0 || container.w <= 0 || container.h <= 0) return [];

  const totalVal = positive.reduce((s, item) => s + item.node.value, 0);
  const totalArea = container.w * container.h;
  const items: SizedItem[] = positive.map(({ node }) => ({
    node,
    area: (node.value / totalVal) * totalArea,
  }));

  const placed: { node: TreemapNode; box: Box }[] = [];
  let rem = { ...container };
  let row: SizedItem[] = [];
  let rowAreas: number[] = [];

  for (const item of items) {
    budget.tick();
    const shortSide = Math.min(rem.w, rem.h);
    if (shortSide <= 0) break;
    if (
      row.length === 0 ||
      worstAspect([...rowAreas, item.area], shortSide) <= worstAspect(rowAreas, shortSide)
    ) {
      row.push(item);
      rowAreas.push(item.area);
    } else {
      rem = layoutStrip(row, rem, rem.w >= rem.h, placed);
      row = [item];
      rowAreas = [item.area];
    }
  }
  if (row.length > 0 && rem.w > 0 && rem.h > 0) {
    layoutStrip(row, rem, rem.w >= rem.h, placed);
  }
  return placed;
}

function drawTile(node: TreemapNode, rawBox: Box, budget: StepBudget): string {
  budget.tick();
  const x = rawBox.x + TILE_GAP / 2;
  const y = rawBox.y + TILE_GAP / 2;
  const w = Math.max(0, rawBox.w - TILE_GAP);
  const h = Math.max(0, rawBox.h - TILE_GAP);
  if (w < 2 || h < 2) return "";

  const accent = series(node.branch);
  const labelText = clampLabel(node.label);

  if (node.isLeaf || node.children.length === 0) {
    let out = rect(x, y, w, h, {
      rx: 4,
      fill: accent,
      "fill-opacity": 0.28,
      stroke: accent,
      "stroke-width": 1.2,
    });
    const valStr = formatNumber(node.value);
    if (w >= 42 && h >= 44) {
      const fitted = fitWidth(labelText, w - 12, { size: FONT.body, bold: true });
      out += text(x + w / 2, y + h / 2 - 8, fitted, {
        size: FONT.body,
        weight: "600",
        anchor: "middle",
        fill: color.text,
      });
      out += text(x + w / 2, y + h / 2 + 10, fitWidth(valStr, w - 12, { size: FONT.small }), {
        size: FONT.small,
        anchor: "middle",
        fill: color.textSec,
      });
    } else if (w >= 54 && h >= 22) {
      const combined = fitWidth(`${labelText}: ${valStr}`, w - 10, { size: FONT.small });
      out += text(x + w / 2, y + h / 2, combined, {
        size: FONT.small,
        anchor: "middle",
        fill: color.text,
      });
    } else if (w >= 28 && h >= 18) {
      out += text(x + w / 2, y + h / 2, fitWidth(labelText, w - 8, { size: FONT.small }), {
        size: FONT.small,
        anchor: "middle",
        fill: color.text,
      });
    }
    return out;
  }

  let out = rect(x, y, w, h, {
    rx: 6,
    fill: accent,
    "fill-opacity": 0.11,
    stroke: accent,
    "stroke-width": 1.4,
  });
  const showHeader = w >= 52 && h >= 38;
  if (showHeader) {
    out += text(x + 8, y + 12, fitWidth(labelText, w - 16, { size: FONT.small + 1, bold: true }), {
      size: FONT.small + 1,
      weight: "600",
      fill: color.text,
    });
  }
  const innerBox: Box = showHeader
    ? { x: x + 4, y: y + HDR_H, w: Math.max(0, w - 8), h: Math.max(0, h - HDR_H - 4) }
    : { x: x + 2, y: y + 2, w: Math.max(0, w - 4), h: Math.max(0, h - 4) };
  if (innerBox.w >= 6 && innerBox.h >= 6) {
    const children = squarify(node.children, innerBox, budget);
    for (const child of children) {
      out += drawTile(child.node, child.box, budget);
    }
  }
  return out;
}

/** Draw a parsed treemap diagram as a complete passive SVG string. */
export function drawTreemap(model: TreemapModel): string {
  const budget = new StepBudget();
  const rootBox: Box = {
    x: PAD,
    y: PAD,
    w: CANVAS_W - PAD * 2,
    h: CANVAS_H - PAD * 2,
  };
  const topPlaced = squarify(model.roots, rootBox, budget);
  const tiles = topPlaced.map((item) => drawTile(item.node, item.box, budget)).join("");
  return svgDoc(CANVAS_W, CANVAS_H, g({}, tiles));
}
