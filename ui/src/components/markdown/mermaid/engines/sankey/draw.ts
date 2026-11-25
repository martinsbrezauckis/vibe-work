/**
 * Sankey diagram drawing: justified layered columns with bounded Gauss-Seidel vertical relaxation,
 * overlap resolution, flush closed cubic Bezier flow ribbons, colored node bars, and side labels.
 */
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, textWidth } from "../../measure";
import { g, n, path, rect, svgDoc, text } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { SankeyLink, SankeyModel, SankeyNode } from "./parse";

const NODE_W = 12;
const NODE_PAD = 12;
const PAD = 20;
const COL_GAP = 160;
const MAX_LABEL_W = 150;

interface Box {
  node: SankeyNode;
  x: number;
  y: number;
  h: number;
  inLinks: SankeyLink[];
  outLinks: SankeyLink[];
}

function resolveColumnCollisions(col: Box[], topBound: number, bottomBound: number): void {
  if (col.length === 0) return;
  col.sort((a, b) => a.y - b.y || a.node.index - b.node.index);
  let cursor = topBound;
  for (const box of col) {
    if (box.y < cursor) box.y = cursor;
    cursor = box.y + box.h + NODE_PAD;
  }
  const last = col[col.length - 1];
  if (last && last.y + last.h > bottomBound) {
    const overflow = last.y + last.h - bottomBound;
    last.y -= overflow;
    for (let i = col.length - 2; i >= 0; i--) {
      const cur = col[i];
      const next = col[i + 1];
      if (cur && next && cur.y + cur.h + NODE_PAD > next.y) {
        cur.y = next.y - NODE_PAD - cur.h;
      }
    }
    const first = col[0];
    if (first && first.y < topBound) {
      const shift = topBound - first.y;
      for (const box of col) box.y += shift;
    }
  }
}

/** Draw a parsed Sankey diagram as a complete passive SVG string. */
export function drawSankey(model: SankeyModel): string {
  const budget = new StepBudget();
  const cols: Box[][] = Array.from({ length: model.maxLayer + 1 }, () => []);
  const boxes: Box[] = model.nodes.map((node) => ({
    node,
    x: 0,
    y: 0,
    h: 0,
    inLinks: [],
    outLinks: [],
  }));
  for (const link of model.links) {
    boxes[link.source]?.outLinks.push(link);
    boxes[link.target]?.inLinks.push(link);
  }
  for (const box of boxes) {
    cols[box.node.layer]?.push(box);
  }

  const maxNodesInCol = Math.max(1, ...cols.map((c) => c.length));
  const targetH = Math.max(420, Math.min(2200, maxNodesInCol * 26));
  let scale = Number.POSITIVE_INFINITY;
  for (const col of cols) {
    if (col.length === 0) continue;
    const totalFlow = col.reduce((s, b) => s + b.node.value, 0);
    const padSpace = Math.max(0, col.length - 1) * NODE_PAD;
    if (totalFlow > 0) {
      const candidate = Math.max(0.01, (targetH - padSpace) / totalFlow);
      if (candidate < scale) scale = candidate;
    }
  }
  if (!Number.isFinite(scale)) scale = 1;

  const labelStyle = { size: FONT.small };
  const labelText = (node: SankeyNode): string =>
    fitWidth(clampLabel(node.id), MAX_LABEL_W, labelStyle);

  const firstColLabels = (cols[0] ?? []).map((b) => textWidth(labelText(b.node), labelStyle));
  const lastColLabels = (cols[model.maxLayer] ?? []).map((b) =>
    textWidth(labelText(b.node), labelStyle),
  );
  const leftMargin = PAD + Math.max(40, ...firstColLabels) + 10;
  const rightMargin = PAD + Math.max(40, ...lastColLabels) + 10;

  // Initial vertical placement per column.
  for (let c = 0; c <= model.maxLayer; c++) {
    const col = cols[c] ?? [];
    const colX = leftMargin + c * (NODE_W + COL_GAP);
    let y = PAD;
    for (const box of col) {
      box.x = colX;
      box.h = Math.max(4, box.node.value * scale);
      box.y = y;
      y += box.h + NODE_PAD;
    }
  }

  // Bounded weighted barycenter sweeps (forward and backward) to reduce edge crossings.
  const bottomBound = PAD + targetH;
  for (let iter = 0; iter < 16; iter++) {
    const alpha = Math.pow(0.85, iter);
    for (let c = 1; c <= model.maxLayer; c++) {
      const col = cols[c] ?? [];
      for (const box of col) {
        budget.tick();
        let sum = 0;
        let weight = 0;
        for (const link of box.inLinks) {
          const src = boxes[link.source];
          if (!src) continue;
          sum += (src.y + src.h / 2) * link.value;
          weight += link.value;
        }
        if (weight > 0) {
          const targetCenter = sum / weight;
          box.y += (targetCenter - box.h / 2 - box.y) * alpha;
        }
      }
      resolveColumnCollisions(col, PAD, bottomBound);
    }
    for (let c = model.maxLayer - 1; c >= 0; c--) {
      const col = cols[c] ?? [];
      for (const box of col) {
        budget.tick();
        let sum = 0;
        let weight = 0;
        for (const link of box.outLinks) {
          const dst = boxes[link.target];
          if (!dst) continue;
          sum += (dst.y + dst.h / 2) * link.value;
          weight += link.value;
        }
        if (weight > 0) {
          const targetCenter = sum / weight;
          box.y += (targetCenter - box.h / 2 - box.y) * alpha;
        }
      }
      resolveColumnCollisions(col, PAD, bottomBound);
    }
  }

  // Normalize top margin and compute total SVG dimensions.
  const minY = Math.min(...boxes.map((b) => b.y));
  const shiftY = PAD - minY;
  for (const box of boxes) box.y += shiftY;
  const maxY = Math.max(...boxes.map((b) => b.y + b.h));
  const width = leftMargin + model.maxLayer * (NODE_W + COL_GAP) + NODE_W + rightMargin;
  const height = maxY + PAD;

  // Assign sorted attachment offsets along source right edges and target left edges.
  const linkCoords = new Map<SankeyLink, { sy0: number; sy1: number; ty0: number; ty1: number }>();
  for (const box of boxes) {
    box.outLinks.sort((a, b) => {
      const ba = boxes[a.target];
      const bb = boxes[b.target];
      return (ba?.y ?? 0) + (ba?.h ?? 0) / 2 - ((bb?.y ?? 0) + (bb?.h ?? 0) / 2);
    });
    let sy = box.y;
    const outDenom = Math.max(box.node.outFlow, box.node.value);
    for (const link of box.outLinks) {
      const span = Math.max(1.2, (link.value / outDenom) * box.h);
      const sy0 = Math.min(box.y + box.h - 1.2, sy);
      const sy1 = Math.min(box.y + box.h, sy0 + span);
      const cur = linkCoords.get(link) ?? { sy0: 0, sy1: 0, ty0: 0, ty1: 0 };
      cur.sy0 = sy0;
      cur.sy1 = Math.max(sy0 + 1, sy1);
      linkCoords.set(link, cur);
      sy += (link.value / outDenom) * box.h;
    }

    box.inLinks.sort((a, b) => {
      const ba = boxes[a.source];
      const bb = boxes[b.source];
      return (ba?.y ?? 0) + (ba?.h ?? 0) / 2 - ((bb?.y ?? 0) + (bb?.h ?? 0) / 2);
    });
    let ty = box.y;
    const inDenom = Math.max(box.node.inFlow, box.node.value);
    for (const link of box.inLinks) {
      const span = Math.max(1.2, (link.value / inDenom) * box.h);
      const ty0 = Math.min(box.y + box.h - 1.2, ty);
      const ty1 = Math.min(box.y + box.h, ty0 + span);
      const cur = linkCoords.get(link) ?? { sy0: 0, sy1: 0, ty0: 0, ty1: 0 };
      cur.ty0 = ty0;
      cur.ty1 = Math.max(ty0 + 1, ty1);
      linkCoords.set(link, cur);
      ty += (link.value / inDenom) * box.h;
    }
  }

  // Draw ribbons first, then nodes and labels on top.
  const ribbons = model.links
    .map((link) => {
      const src = boxes[link.source];
      const dst = boxes[link.target];
      const c = linkCoords.get(link);
      if (!src || !dst || !c) return "";
      const x0 = src.x + NODE_W;
      const x1 = dst.x;
      const mx = (x0 + x1) / 2;
      const d = `M${n(x0)},${n(c.sy0)} C${n(mx)},${n(c.sy0)} ${n(mx)},${n(c.ty0)} ${n(x1)},${n(c.ty0)} L${n(x1)},${n(c.ty1)} C${n(mx)},${n(c.ty1)} ${n(mx)},${n(c.sy1)} ${n(x0)},${n(c.sy1)} Z`;
      return path(d, {
        fill: series(link.source),
        "fill-opacity": 0.36,
      });
    })
    .join("");

  const nodesMarkup = boxes
    .map((box) =>
      rect(box.x, box.y, NODE_W, box.h, {
        rx: 2,
        fill: series(box.node.index),
      }),
    )
    .join("");

  const labelsMarkup = boxes
    .map((box) => {
      const lbl = labelText(box.node);
      const cy = box.y + box.h / 2;
      if (box.node.layer === 0) {
        return text(box.x - 6, cy, lbl, {
          size: FONT.small,
          anchor: "end",
          fill: color.text,
        });
      }
      if (box.node.layer === model.maxLayer) {
        return text(box.x + NODE_W + 6, cy, lbl, {
          size: FONT.small,
          anchor: "start",
          fill: color.text,
        });
      }
      const onRight = box.node.layer <= model.maxLayer / 2;
      return text(onRight ? box.x + NODE_W + 6 : box.x - 6, cy, lbl, {
        size: FONT.small,
        anchor: onRight ? "start" : "end",
        fill: color.text,
      });
    })
    .join("");

  return svgDoc(width, height, g({}, ribbons) + g({}, nodesMarkup) + g({}, labelsMarkup));
}
