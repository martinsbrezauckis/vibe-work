/**
 * Swimlane diagram drawing: lane-banded DAG layout (LR/RL horizontal lanes or TB/BT vertical
 * lanes) with global longest-path ranking, nested inner subgraph boxes, flowchart shapes, and
 * rounded orthogonal connectors across lane dividers.
 */
import { StepBudget } from "../../limits";
import { roundedPath, type Point } from "../../layout/layered";
import { fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import {
  circle,
  el,
  line,
  markerDefs,
  markerRef,
  path,
  polygon,
  rect,
  svgDoc,
  text,
  textLines,
} from "../../svg";
import { FONT, color, series } from "../../theme";
import type { SwimlaneModel, SwimlaneNode } from "./parse";

const PAD = 20;
const LANE_GAP = 10;
const RANK_GAP = 54;
const CELL_GAP = 20;
const LANE_HEAD_LR = 148;
const LANE_HEAD_TB = 38;
const LINE_H = 17;

interface MeasuredNode {
  node: SwimlaneNode;
  lines: string[];
  width: number;
  height: number;
  rank: number;
}

function measureNode(node: SwimlaneNode): Omit<MeasuredNode, "rank"> {
  const style = { size: FONT.body, bold: true };
  const lines = wrapText(node.label, 180, style, 3);
  const measured = measureLines(lines, style, 1.3);
  const padX =
    node.shape === "diamond"
      ? 48
      : node.shape === "hexagon" || node.shape === "subroutine" || node.shape === "stadium"
        ? 38
        : 28;
  const padY = node.shape === "diamond" ? 32 : node.shape === "cylinder" ? 26 : 18;
  let width = Math.max(82, measured.width + padX);
  let height = Math.max(40, lines.length * LINE_H + padY);
  if (node.shape === "circle") {
    const d = Math.max(48, Math.max(width, height));
    width = d;
    height = d;
  }
  return { node, lines, width, height };
}

function drawShape(m: MeasuredNode, cx: number, cy: number): string {
  const x = cx - m.width / 2;
  const y = cy - m.height / 2;
  const w = m.width;
  const h = m.height;
  const attrs = { fill: color.nodeFill, stroke: color.nodeStroke, "stroke-width": 1.3 };
  let shapeSvg: string;

  switch (m.node.shape) {
    case "round":
      shapeSvg = rect(x, y, w, h, { rx: 10, ...attrs });
      break;
    case "stadium":
      shapeSvg = rect(x, y, w, h, { rx: h / 2, ...attrs });
      break;
    case "circle":
      shapeSvg = circle(cx, cy, w / 2, attrs);
      break;
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
    case "hexagon": {
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
    case "subroutine":
      shapeSvg =
        rect(x, y, w, h, { rx: 4, ...attrs }) +
        line(x + 9, y, x + 9, y + h, { stroke: color.nodeStroke, "stroke-width": 1.1 }) +
        line(x + w - 9, y, x + w - 9, y + h, { stroke: color.nodeStroke, "stroke-width": 1.1 });
      break;
    case "cylinder": {
      const ry = 6;
      shapeSvg =
        rect(x, y + ry, w, h - ry * 2, { ...attrs, stroke: "none" }) +
        line(x, y + ry, x, y + h - ry, { stroke: color.nodeStroke, "stroke-width": 1.3 }) +
        line(x + w, y + ry, x + w, y + h - ry, { stroke: color.nodeStroke, "stroke-width": 1.3 }) +
        el("ellipse", { cx, cy: y + ry, rx: w / 2, ry, ...attrs }) +
        el("ellipse", {
          cx,
          cy: y + h - ry,
          rx: w / 2,
          ry,
          fill: "none",
          stroke: color.nodeStroke,
          "stroke-width": 1.3,
        });
      break;
    }
    default:
      shapeSvg = rect(x, y, w, h, { rx: 4, ...attrs });
      break;
  }

  return (
    shapeSvg +
    textLines(
      cx,
      cy - ((m.lines.length - 1) * LINE_H) / 2,
      m.lines,
      { size: FONT.body, weight: "600", anchor: "middle" },
      LINE_H / FONT.body,
    )
  );
}

function assignRanks(model: SwimlaneModel, budget: StepBudget): Map<string, number> {
  const nodes = model.nodes;
  const idx = new Map(nodes.map((n, i) => [n.id, i]));
  const out: number[][] = nodes.map(() => []);
  const pairs: { from: number; to: number }[] = [];
  for (const e of model.edges) {
    const a = idx.get(e.from);
    const b = idx.get(e.to);
    if (a !== undefined && b !== undefined && a !== b) {
      out[a]?.push(pairs.length);
      pairs.push({ from: a, to: b });
    }
  }

  const state = new Array<number>(nodes.length).fill(0);
  const reversed = new Set<number>();
  for (let root = 0; root < nodes.length; root++) {
    if (state[root] !== 0) continue;
    const dfs: { node: number; next: number }[] = [{ node: root, next: 0 }];
    state[root] = 1;
    while (dfs.length > 0) {
      budget.tick();
      const top = dfs[dfs.length - 1];
      if (!top) break;
      const k = out[top.node]?.[top.next++];
      if (k === undefined) {
        state[top.node] = 2;
        dfs.pop();
        continue;
      }
      const to = pairs[k]?.to ?? 0;
      if (state[to] === 1) reversed.add(k);
      else if (state[to] === 0) {
        state[to] = 1;
        dfs.push({ node: to, next: 0 });
      }
    }
  }

  const dag = pairs.map((e, k) => (reversed.has(k) ? { from: e.to, to: e.from } : e));
  const indeg = new Array<number>(nodes.length).fill(0);
  const succ: number[][] = nodes.map(() => []);
  for (const e of dag) {
    indeg[e.to] = (indeg[e.to] ?? 0) + 1;
    succ[e.from]?.push(e.to);
  }
  const rank = new Array<number>(nodes.length).fill(0);
  const queue = nodes.map((_, i) => i).filter((i) => indeg[i] === 0);
  for (let head = 0; head < queue.length; head++) {
    budget.tick();
    const u = queue[head] ?? 0;
    for (const v of succ[u] ?? []) {
      rank[v] = Math.max(rank[v] ?? 0, (rank[u] ?? 0) + 1);
      indeg[v] = (indeg[v] ?? 1) - 1;
      if (indeg[v] === 0) queue.push(v);
    }
  }

  const outMap = new Map<string, number>();
  nodes.forEach((n, i) => outMap.set(n.id, rank[i] ?? 0));
  return outMap;
}

/** Draw a parsed swimlane diagram as a complete SVG string. */
export function drawSwimlane(model: SwimlaneModel): string {
  const budget = new StepBudget();
  const ranks = assignRanks(model, budget);
  const measuredMap = new Map<string, MeasuredNode>();
  let maxRank = 0;
  for (const node of model.nodes) {
    budget.tick();
    const r = ranks.get(node.id) ?? 0;
    maxRank = Math.max(maxRank, r);
    measuredMap.set(node.id, { ...measureNode(node), rank: r });
  }

  const horizontal = model.direction === "LR" || model.direction === "RL";
  const reverse = model.direction === "RL" || model.direction === "BT";
  const rankCount = maxRank + 1;

  const laneRankGroups = new Map<string, MeasuredNode[][]>();
  for (const lane of model.lanes) {
    const cols: MeasuredNode[][] = Array.from({ length: rankCount }, () => []);
    for (const id of lane.nodeIds) {
      const m = measuredMap.get(id);
      if (m) cols[m.rank]?.push(m);
    }
    laneRankGroups.set(lane.id, cols);
  }

  // Flow-axis rank sizes and cross-axis lane sizes.
  const rankFlowSize = new Array<number>(rankCount).fill(82);
  const laneCrossSize: number[] = model.lanes.map(() => (horizontal ? 88 : 156));
  const labelStyle = { size: FONT.small + 1 };

  for (let r = 0; r < rankCount; r++) {
    let maxF = horizontal ? 90 : 48;
    model.lanes.forEach((lane, li) => {
      const cell = laneRankGroups.get(lane.id)?.[r] ?? [];
      let crossTotal = 0;
      for (const m of cell) {
        maxF = Math.max(maxF, horizontal ? m.width : m.height);
        crossTotal += (horizontal ? m.height : m.width) + CELL_GAP;
      }
      if (cell.length > 0) crossTotal -= CELL_GAP;
      const boxPad = lane.nestedBoxes.length > 0 ? 48 : 34;
      laneCrossSize[li] = Math.max(laneCrossSize[li] ?? 88, crossTotal + boxPad);
    });
    rankFlowSize[r] = maxF;
  }

  const extraRankGap = new Array<number>(rankCount).fill(0);
  for (const e of model.edges) {
    if (!e.label) continue;
    const a = ranks.get(e.from) ?? 0;
    const b = ranks.get(e.to) ?? 0;
    const minR = Math.min(a, b);
    const need = horizontal ? textWidth(e.label, labelStyle) + 18 : 22;
    extraRankGap[minR] = Math.max(extraRankGap[minR] ?? 0, need);
  }

  const rankCenters = new Array<number>(rankCount).fill(0);
  let flowCursor = PAD + (horizontal ? LANE_HEAD_LR + 24 : LANE_HEAD_TB + 24);
  const orderRanks = Array.from({ length: rankCount }, (_, i) => (reverse ? rankCount - 1 - i : i));
  for (const r of orderRanks) {
    const size = rankFlowSize[r] ?? 80;
    rankCenters[r] = flowCursor + size / 2;
    flowCursor += size + RANK_GAP + (extraRankGap[r] ?? 0);
  }
  const flowEnd = flowCursor - RANK_GAP + PAD;

  const laneStarts: number[] = [];
  let crossCursor = PAD;
  model.lanes.forEach((_, li) => {
    laneStarts.push(crossCursor);
    crossCursor += (laneCrossSize[li] ?? 100) + LANE_GAP;
  });
  const crossEnd = crossCursor - LANE_GAP + PAD;

  const totalW = horizontal ? flowEnd : crossEnd;
  const totalH = horizontal ? crossEnd : flowEnd;

  const back: string[] = [];
  const mid: string[] = [];
  const front: string[] = [];
  const placed = new Map<string, { cx: number; cy: number; w: number; h: number }>();

  // Draw swimlane bands and place nodes.
  model.lanes.forEach((lane, li) => {
    budget.tick();
    const start = laneStarts[li] ?? PAD;
    const cross = laneCrossSize[li] ?? 100;
    const lx = horizontal ? PAD : start;
    const ly = horizontal ? start : PAD;
    const lw = horizontal ? totalW - PAD * 2 : cross;
    const lh = horizontal ? cross : totalH - PAD * 2;

    back.push(
      rect(lx, ly, lw, lh, {
        rx: 8,
        fill: series(li),
        "fill-opacity": 0.05,
        stroke: color.innerStroke,
        "stroke-width": 1.1,
      }),
    );
    if (horizontal) {
      back.push(
        rect(lx, ly, LANE_HEAD_LR, lh, {
          rx: 8,
          fill: color.groupHdr,
          "fill-opacity": 0.85,
          stroke: "none",
        }),
      );
      back.push(
        line(lx + LANE_HEAD_LR, ly, lx + LANE_HEAD_LR, ly + lh, {
          stroke: color.innerStroke,
          "stroke-width": 1,
        }),
      );
      const lLines = wrapText(lane.label, LANE_HEAD_LR - 20, { size: FONT.body, bold: true }, 3);
      front.push(
        textLines(
          lx + LANE_HEAD_LR / 2,
          ly + lh / 2 - ((lLines.length - 1) * LINE_H) / 2,
          lLines,
          { size: FONT.body, weight: "600", anchor: "middle" },
          LINE_H / FONT.body,
        ),
      );
    } else {
      back.push(
        rect(lx, ly, lw, LANE_HEAD_TB, {
          rx: 8,
          fill: color.groupHdr,
          "fill-opacity": 0.85,
          stroke: "none",
        }),
      );
      back.push(
        line(lx, ly + LANE_HEAD_TB, lx + lw, ly + LANE_HEAD_TB, {
          stroke: color.innerStroke,
          "stroke-width": 1,
        }),
      );
      front.push(
        text(
          lx + lw / 2,
          ly + LANE_HEAD_TB / 2,
          fitWidth(lane.label, lw - 16, { size: FONT.body, bold: true }),
          {
            size: FONT.body,
            weight: "600",
            anchor: "middle",
          },
        ),
      );
    }

    const groups = laneRankGroups.get(lane.id) ?? [];
    groups.forEach((cell, r) => {
      if (cell.length === 0) return;
      const flowCenter = rankCenters[r] ?? 0;
      const totalCross =
        cell.reduce((s, m) => s + (horizontal ? m.height : m.width), 0) +
        (cell.length - 1) * CELL_GAP;
      let cCursor = start + cross / 2 - totalCross / 2;
      for (const m of cell) {
        const sizeC = horizontal ? m.height : m.width;
        const cMid = cCursor + sizeC / 2;
        const cx = horizontal ? flowCenter : cMid;
        const cy = horizontal ? cMid : flowCenter;
        placed.set(m.node.id, { cx, cy, w: m.width, h: m.height });
        cCursor += sizeC + CELL_GAP;
      }
    });

    // Nested boxes inside the lane.
    for (const box of lane.nestedBoxes) {
      const members = box.nodeIds
        .map((id) => placed.get(id))
        .filter((p): p is { cx: number; cy: number; w: number; h: number } => p !== undefined);
      if (members.length === 0) continue;
      const bx1 = Math.min(...members.map((p) => p.cx - p.w / 2)) - 12;
      const bx2 = Math.max(...members.map((p) => p.cx + p.w / 2)) + 12;
      const by1 = Math.min(...members.map((p) => p.cy - p.h / 2)) - 26;
      const by2 = Math.max(...members.map((p) => p.cy + p.h / 2)) + 12;
      back.push(
        rect(bx1, by1, bx2 - bx1, by2 - by1, {
          rx: 6,
          fill: color.groupFill,
          "fill-opacity": 0.7,
          stroke: color.innerStroke,
          "stroke-width": 1,
          "stroke-dasharray": "5 3",
        }),
      );
      back.push(
        text(bx1 + 10, by1 + 13, fitWidth(box.label, bx2 - bx1 - 20, labelStyle), {
          size: FONT.small + 1,
          weight: "600",
          fill: color.textSec,
        }),
      );
    }
  });

  // Draw edges.
  model.edges.forEach((edge, i) => {
    budget.tick();
    const a = placed.get(edge.from);
    const b = placed.get(edge.to);
    if (!a || !b) return;
    let pts: Point[];
    let lx: number;
    let ly: number;
    if (edge.from === edge.to) {
      const x = a.cx + a.w / 2;
      pts = [
        [x, a.cy - 10],
        [x + 24, a.cy - 10],
        [x + 24, a.cy + 10],
        [x, a.cy + 10],
      ];
      lx = x + 30;
      ly = a.cy;
    } else if (horizontal) {
      const forward = b.cx >= a.cx;
      const sx = forward ? a.cx + a.w / 2 : a.cx - a.w / 2;
      const ex = forward ? b.cx - b.w / 2 : b.cx + b.w / 2;
      const offset = ((i % 3) - 1) * 6;
      const sy = a.cy + offset;
      const ey = b.cy + offset;
      if (Math.abs(a.cx - b.cx) < 1) {
        pts = [
          [a.cx, b.cy > a.cy ? a.cy + a.h / 2 : a.cy - a.h / 2],
          [b.cx, b.cy > a.cy ? b.cy - b.h / 2 : b.cy + b.h / 2],
        ];
        lx = a.cx;
        ly = (a.cy + b.cy) / 2;
      } else {
        const mx = (sx + ex) / 2;
        pts = [
          [sx, sy],
          [mx, sy],
          [mx, ey],
          [ex, ey],
        ];
        lx = mx;
        ly = (sy + ey) / 2;
      }
    } else {
      const forward = b.cy >= a.cy;
      const sy = forward ? a.cy + a.h / 2 : a.cy - a.h / 2;
      const ey = forward ? b.cy - b.h / 2 : b.cy + b.h / 2;
      const offset = ((i % 3) - 1) * 6;
      const sx = a.cx + offset;
      const ex = b.cx + offset;
      if (Math.abs(a.cy - b.cy) < 1) {
        pts = [
          [b.cx > a.cx ? a.cx + a.w / 2 : a.cx - a.w / 2, a.cy],
          [b.cx > a.cx ? b.cx - b.w / 2 : b.cx + b.w / 2, b.cy],
        ];
        lx = (a.cx + b.cx) / 2;
        ly = a.cy;
      } else {
        const my = (sy + ey) / 2;
        pts = [
          [sx, sy],
          [sx, my],
          [ex, my],
          [ex, ey],
        ];
        lx = (sx + ex) / 2;
        ly = my;
      }
    }

    mid.push(
      path(roundedPath(pts, 7), {
        fill: "none",
        stroke: color.line,
        "stroke-width": edge.style === "thick" ? 2.4 : 1.4,
        "stroke-dasharray": edge.style === "dotted" ? "5 4" : undefined,
        "marker-start": edge.arrowStart ? markerRef("arrow-filled") : undefined,
        "marker-end": edge.arrowEnd ? markerRef("arrow-filled") : undefined,
      }),
    );

    if (edge.label) {
      const lbl = fitWidth(edge.label, 140, labelStyle);
      const lw = textWidth(lbl, labelStyle) + 12;
      front.push(
        rect(lx - lw / 2, ly - 9, lw, 18, {
          rx: 4,
          fill: color.canvas,
          "fill-opacity": 0.94,
          stroke: color.innerStroke,
          "stroke-width": 0.8,
        }),
      );
      front.push(
        text(lx, ly, lbl, {
          size: FONT.small + 1,
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }
  });

  // Draw node shapes above edges.
  for (const node of model.nodes) {
    const m = measuredMap.get(node.id);
    const p = placed.get(node.id);
    if (m && p) front.push(drawShape(m, p.cx, p.cy));
  }

  return svgDoc(totalW, totalH, [...back, ...mid, ...front].join(""), markerDefs(["arrow-filled"]));
}
