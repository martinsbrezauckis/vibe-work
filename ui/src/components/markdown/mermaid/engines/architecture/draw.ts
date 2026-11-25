/**
 * Architecture diagram drawing: 2D port-constraint grid solver (`T|B|L|R`, `align row|column`),
 * nested group bounding containers, service cards with icons, junctions, and orthogonal ported
 * routing (including `{group}` boundary attachments and port spreading).
 */
import { roundedPath, type Point } from "../../layout/layered";
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, measureLines, wrapText } from "../../measure";
import { circle, el, markerDefs, markerRef, n, path, rect, svgDoc, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import { drawArchIcon } from "./icons";
import type { ArchEdge, ArchGroup, ArchitectureModel, Side } from "./parse";

const CARD_PAD_X = 14;
const CARD_MIN_W = 86;
const CARD_MAX_W = 190;
const ICON_SIZE = 26;
const JUNCTION_SIZE = 16;
const COL_GAP = 56;
const ROW_GAP = 56;
const GROUP_PAD = 20;
const GROUP_HDR = 30;
const MARGIN = 24;
const STUB = 18;

interface NodeSpec {
  id: string;
  kind: "service" | "junction";
  icon: string;
  label: string;
  lines: string[];
  parent?: string | undefined;
  width: number;
  height: number;
}

interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

const outVec = (side: Side): [number, number] => {
  switch (side) {
    case "L":
      return [-1, 0];
    case "R":
      return [1, 0];
    case "T":
      return [0, -1];
    case "B":
      return [0, 1];
  }
};

function idealOffset(fromSide: Side, toSide: Side): [number, number] {
  const [ax, ay] = outVec(fromSide);
  const [bx, by] = outVec(toSide);
  const dx = Math.sign(ax - bx);
  const dy = Math.sign(ay - by);
  if (dx === 0 && dy === 0) {
    return fromSide === "T" || fromSide === "B" ? [1, 0] : [0, 1];
  }
  return [dx, dy];
}

function solveGrid(
  model: ArchitectureModel,
  nodes: NodeSpec[],
  budget: StepBudget,
): Map<string, { gx: number; gy: number }> {
  const adj = new Map<string, { other: string; dx: number; dy: number }[]>();
  const rowEquiv = new Map<string, string[]>();
  const colEquiv = new Map<string, string[]>();
  for (const node of nodes) {
    adj.set(node.id, []);
    rowEquiv.set(node.id, []);
    colEquiv.set(node.id, []);
  }
  const addLink = (a: string, b: string, dx: number, dy: number): void => {
    adj.get(a)?.push({ other: b, dx, dy });
    adj.get(b)?.push({ other: a, dx: -dx, dy: -dy });
  };
  for (const edge of model.edges) {
    if (edge.from === edge.to) continue;
    const [dx, dy] = idealOffset(edge.fromSide, edge.toSide);
    addLink(edge.from, edge.to, dx, dy);
  }
  for (const align of model.aligns) {
    for (let i = 1; i < align.members.length; i++) {
      const a = align.members[i - 1] ?? "";
      const b = align.members[i] ?? "";
      if (align.axis === "row") {
        addLink(a, b, 1, 0);
        rowEquiv.get(a)?.push(b);
        rowEquiv.get(b)?.push(a);
      } else {
        addLink(a, b, 0, 1);
        colEquiv.get(a)?.push(b);
        colEquiv.get(b)?.push(a);
      }
    }
  }

  // BFS per connected component to assign starting coordinates, wrapping wide disconnected sets.
  const coords = new Map<string, { gx: number; gy: number }>();
  let compOffsetX = 0;
  let compOffsetY = 0;
  let rowMaxY = 0;
  for (const root of nodes) {
    if (coords.has(root.id)) continue;
    const queue = [root.id];
    const compIds: string[] = [];
    coords.set(root.id, { gx: 0, gy: 0 });
    for (let head = 0; head < queue.length; head++) {
      budget.tick();
      const u = queue[head] ?? "";
      compIds.push(u);
      const uPos = coords.get(u) ?? { gx: 0, gy: 0 };
      for (const link of adj.get(u) ?? []) {
        if (coords.has(link.other)) continue;
        coords.set(link.other, { gx: uPos.gx + link.dx, gy: uPos.gy + link.dy });
        queue.push(link.other);
      }
    }

    // Pull aligned peers to identical row/column coordinate and relax ordering constraints.
    for (let iter = 0; iter < compIds.length * 2 + 4; iter++) {
      let changed = false;
      for (const u of compIds) {
        budget.tick();
        const uPos = coords.get(u) ?? { gx: 0, gy: 0 };
        for (const v of rowEquiv.get(u) ?? []) {
          const vPos = coords.get(v);
          if (vPos && vPos.gy !== uPos.gy) {
            vPos.gy = uPos.gy;
            changed = true;
          }
        }
        for (const v of colEquiv.get(u) ?? []) {
          const vPos = coords.get(v);
          if (vPos && vPos.gx !== uPos.gx) {
            vPos.gx = uPos.gx;
            changed = true;
          }
        }
      }
      for (const align of model.aligns) {
        for (let i = 1; i < align.members.length; i++) {
          const a = coords.get(align.members[i - 1] ?? "");
          const b = coords.get(align.members[i] ?? "");
          if (!a || !b) continue;
          if (align.axis === "row" && b.gx <= a.gx) {
            b.gx = a.gx + 1;
            changed = true;
          } else if (align.axis === "column" && b.gy <= a.gy) {
            b.gy = a.gy + 1;
            changed = true;
          }
        }
      }
      if (!changed) break;
    }

    // Resolve any exact (gx, gy) cell collisions inside the component.
    const rowLocked = new Set<string>();
    const colLocked = new Set<string>();
    for (const align of model.aligns) {
      for (const m of align.members) {
        if (align.axis === "row") rowLocked.add(m);
        else colLocked.add(m);
      }
    }
    const occupied = new Set<string>();
    for (const id of compIds) {
      const p = coords.get(id) ?? { gx: 0, gy: 0 };
      let key = `${String(p.gx)},${String(p.gy)}`;
      while (occupied.has(key)) {
        budget.tick();
        if (rowLocked.has(id) && !colLocked.has(id)) p.gx += 1;
        else p.gy += 1;
        key = `${String(p.gx)},${String(p.gy)}`;
      }
      occupied.add(key);
    }

    const minX = Math.min(...compIds.map((id) => coords.get(id)?.gx ?? 0));
    const maxX = Math.max(...compIds.map((id) => coords.get(id)?.gx ?? 0));
    const minY = Math.min(...compIds.map((id) => coords.get(id)?.gy ?? 0));
    const maxY = Math.max(...compIds.map((id) => coords.get(id)?.gy ?? 0));
    const compW = maxX - minX + 1;
    const compH = maxY - minY + 1;
    if (compOffsetX > 0 && compOffsetX + compW > 12) {
      compOffsetX = 0;
      compOffsetY += rowMaxY + 1;
      rowMaxY = 0;
    }
    for (const id of compIds) {
      const p = coords.get(id);
      if (!p) continue;
      p.gx = p.gx - minX + compOffsetX;
      p.gy = p.gy - minY + compOffsetY;
    }
    compOffsetX += compW + 1;
    rowMaxY = Math.max(rowMaxY, compH);
  }

  return coords;
}

function buildGroupChildren(groups: readonly ArchGroup[]): Map<string | undefined, ArchGroup[]> {
  const children = new Map<string | undefined, ArchGroup[]>();
  for (const grp of groups) {
    const list = children.get(grp.parent) ?? [];
    list.push(grp);
    children.set(grp.parent, list);
  }
  return children;
}

function groupChainDepth(id: string | undefined, byId: Map<string, ArchGroup>): number {
  let depth = 0;
  let cur = id;
  while (cur !== undefined) {
    depth++;
    cur = byId.get(cur)?.parent;
  }
  return depth;
}

/** Draw a parsed architecture diagram as a complete passive SVG string. */
export function drawArchitecture(model: ArchitectureModel): string {
  const budget = new StepBudget();
  const groupById = new Map(model.groups.map((g) => [g.id, g]));
  const nodeSpecs: NodeSpec[] = [
    ...model.services.map((s): NodeSpec => {
      const lines = wrapText(
        clampLabel(s.label),
        CARD_MAX_W - CARD_PAD_X * 2,
        { size: FONT.small + 1 },
        2,
      );
      const labelSize = measureLines(lines, { size: FONT.small + 1 });
      const width = Math.max(CARD_MIN_W, labelSize.width + CARD_PAD_X * 2, ICON_SIZE + 28);
      const height = 12 + ICON_SIZE + 8 + labelSize.height + 10;
      return {
        id: s.id,
        kind: "service",
        icon: s.icon,
        label: s.label,
        lines,
        parent: s.parent,
        width,
        height,
      };
    }),
    ...model.junctions.map((j): NodeSpec => ({
      id: j.id,
      kind: "junction",
      icon: "",
      label: "",
      lines: [],
      parent: j.parent,
      width: JUNCTION_SIZE,
      height: JUNCTION_SIZE,
    })),
  ];

  // If the diagram has only empty groups and no nodes, draw the groups directly.
  if (nodeSpecs.length === 0) {
    let x = MARGIN;
    let body = "";
    let maxH = 0;
    model.groups.forEach((grp, idx) => {
      const w = Math.max(
        140,
        measureLines([clampLabel(grp.label)], { size: FONT.body, bold: true }).width + 52,
      );
      const h = 84;
      body += rect(x, MARGIN, w, h, {
        rx: 8,
        fill: color.groupFill,
        stroke: color.nodeStroke,
        "stroke-width": 1.2,
        "stroke-dasharray": "6 4",
      });
      body += drawArchIcon(grp.icon, x + 18, MARGIN + 16, 16, series(idx));
      body += textLines(
        x + 32,
        MARGIN + 16,
        [fitWidth(clampLabel(grp.label), w - 42, { size: FONT.body, bold: true })],
        {
          size: FONT.body,
          weight: "600",
          fill: color.textSec,
        },
      );
      x += w + GROUP_PAD;
      maxH = Math.max(maxH, MARGIN * 2 + h);
    });
    return svgDoc(x, maxH || 120, body);
  }

  const grid = solveGrid(model, nodeSpecs, budget);
  const xs = [...new Set([...grid.values()].map((c) => c.gx))].sort((a, b) => a - b);
  const ys = [...new Set([...grid.values()].map((c) => c.gy))].sort((a, b) => a - b);
  const colIdx = new Map(xs.map((x, i) => [x, i]));
  const rowIdx = new Map(ys.map((y, i) => [y, i]));

  const colW = new Array<number>(xs.length).fill(48);
  const rowH = new Array<number>(ys.length).fill(48);
  for (const spec of nodeSpecs) {
    const pos = grid.get(spec.id);
    if (!pos) continue;
    const c = colIdx.get(pos.gx) ?? 0;
    const r = rowIdx.get(pos.gy) ?? 0;
    colW[c] = Math.max(colW[c] ?? 0, spec.width);
    rowH[r] = Math.max(rowH[r] ?? 0, spec.height);
  }

  // Add spacing between columns/rows when group membership changes so borders and headers never crowd.
  const colCellParent = new Array<string | undefined>(xs.length).fill(undefined);
  const rowCellParent = new Array<string | undefined>(ys.length).fill(undefined);
  for (const spec of nodeSpecs) {
    const pos = grid.get(spec.id);
    if (!pos) continue;
    const c = colIdx.get(pos.gx) ?? 0;
    const r = rowIdx.get(pos.gy) ?? 0;
    if (spec.parent && !colCellParent[c]) colCellParent[c] = spec.parent;
    if (spec.parent && !rowCellParent[r]) rowCellParent[r] = spec.parent;
  }

  const colCenterX: number[] = [];
  let cursorX = MARGIN;
  for (let c = 0; c < xs.length; c++) {
    const w = colW[c] ?? 48;
    colCenterX.push(cursorX + w / 2);
    const nextChanged = c + 1 < xs.length && colCellParent[c] !== colCellParent[c + 1];
    cursorX += w + COL_GAP + (nextChanged ? GROUP_PAD * 1.5 : 0);
  }

  const rowCenterY: number[] = [];
  let cursorY = MARGIN;
  for (let r = 0; r < ys.length; r++) {
    const h = rowH[r] ?? 48;
    rowCenterY.push(cursorY + h / 2);
    const nextChanged = r + 1 < ys.length && rowCellParent[r] !== rowCellParent[r + 1];
    cursorY += h + ROW_GAP + (nextChanged ? GROUP_HDR + GROUP_PAD : 0);
  }

  const nodeBoxes = new Map<string, Box>();
  for (const spec of nodeSpecs) {
    const pos = grid.get(spec.id);
    if (!pos) continue;
    const cx = colCenterX[colIdx.get(pos.gx) ?? 0] ?? MARGIN;
    const cy = rowCenterY[rowIdx.get(pos.gy) ?? 0] ?? MARGIN;
    nodeBoxes.set(spec.id, {
      x: cx - spec.width / 2,
      y: cy - spec.height / 2,
      width: spec.width,
      height: spec.height,
    });
  }

  // Compute group bounding boxes recursively from innermost groups outward.
  const groupBoxes = new Map<string, Box>();
  const childrenByGroup = buildGroupChildren(model.groups);
  const orderedGroups = [...model.groups].sort(
    (a, b) => groupChainDepth(b.parent, groupById) - groupChainDepth(a.parent, groupById),
  );
  for (const grp of orderedGroups) {
    const memberBoxes: Box[] = [];
    for (const spec of nodeSpecs) {
      if (spec.parent === grp.id) {
        const b = nodeBoxes.get(spec.id);
        if (b) memberBoxes.push(b);
      }
    }
    for (const child of childrenByGroup.get(grp.id) ?? []) {
      const b = groupBoxes.get(child.id);
      if (b) memberBoxes.push(b);
    }
    if (memberBoxes.length === 0) continue;
    const x1 = Math.min(...memberBoxes.map((b) => b.x)) - GROUP_PAD;
    const y1 = Math.min(...memberBoxes.map((b) => b.y)) - GROUP_HDR - 8;
    const x2 = Math.max(...memberBoxes.map((b) => b.x + b.width)) + GROUP_PAD;
    const y2 = Math.max(...memberBoxes.map((b) => b.y + b.height)) + GROUP_PAD;
    const minTitleW =
      measureLines([clampLabel(grp.label)], { size: FONT.body, bold: true }).width + 52;
    const finalW = Math.max(x2 - x1, minTitleW);
    groupBoxes.set(grp.id, { x: x1, y: y1, width: finalW, height: y2 - y1 });
  }

  // Spread edge endpoints that attach to the same box and side.
  const portEntries = new Map<
    string,
    { edgeIdx: number; end: "from" | "to"; alignCoord: number }[]
  >();
  const registerPort = (
    boxKey: string,
    side: Side,
    edgeIdx: number,
    end: "from" | "to",
    alignCoord: number,
  ): void => {
    const k = `${boxKey}:${side}`;
    const list = portEntries.get(k) ?? [];
    list.push({ edgeIdx, end, alignCoord });
    portEntries.set(k, list);
  };
  const specMap = new Map(nodeSpecs.map((s) => [s.id, s]));
  const boxForEnd = (
    nodeId: string,
    useGroup: boolean,
  ): { key: string; box: Box; nodeBox: Box } => {
    const nodeBox = nodeBoxes.get(nodeId) ?? { x: 0, y: 0, width: 40, height: 40 };
    if (useGroup) {
      const parent = specMap.get(nodeId)?.parent;
      const gBox = parent ? groupBoxes.get(parent) : undefined;
      if (parent && gBox) return { key: `g:${parent}`, box: gBox, nodeBox };
    }
    return { key: `n:${nodeId}`, box: nodeBox, nodeBox };
  };

  model.edges.forEach((edge, i) => {
    const a = boxForEnd(edge.from, edge.fromGroup);
    const b = boxForEnd(edge.to, edge.toGroup);
    const aAlign =
      edge.fromSide === "T" || edge.fromSide === "B"
        ? a.nodeBox.x + a.nodeBox.width / 2
        : a.nodeBox.y + a.nodeBox.height / 2;
    const bAlign =
      edge.toSide === "T" || edge.toSide === "B"
        ? b.nodeBox.x + b.nodeBox.width / 2
        : b.nodeBox.y + b.nodeBox.height / 2;
    registerPort(a.key, edge.fromSide, i, "from", bAlign);
    registerPort(b.key, edge.toSide, i, "to", aAlign);
  });

  const portShift = new Map<string, number>();
  for (const list of portEntries.values()) {
    budget.tick(list.length);
    list.sort((u, v) => u.alignCoord - v.alignCoord || u.edgeIdx - v.edgeIdx);
    list.forEach((entry, idx) => {
      const offset = (idx - (list.length - 1) / 2) * 10;
      portShift.set(`${String(entry.edgeIdx)}:${entry.end}`, offset);
    });
  }

  const portPoint = (edge: ArchEdge, end: "from" | "to", edgeIdx: number): Point => {
    const id = end === "from" ? edge.from : edge.to;
    const useGroup = end === "from" ? edge.fromGroup : edge.toGroup;
    const side = end === "from" ? edge.fromSide : edge.toSide;
    const { box, nodeBox } = boxForEnd(id, useGroup);
    const shift = useGroup ? 0 : (portShift.get(`${String(edgeIdx)}:${end}`) ?? 0);
    const anchorX = useGroup ? nodeBox.x + nodeBox.width / 2 : box.x + box.width / 2 + shift;
    const anchorY = useGroup ? nodeBox.y + nodeBox.height / 2 : box.y + box.height / 2 + shift;
    switch (side) {
      case "L":
        return [box.x, Math.max(box.y + 8, Math.min(box.y + box.height - 8, anchorY))];
      case "R":
        return [box.x + box.width, Math.max(box.y + 8, Math.min(box.y + box.height - 8, anchorY))];
      case "T":
        return [Math.max(box.x + 8, Math.min(box.x + box.width - 8, anchorX)), box.y];
      case "B":
        return [Math.max(box.x + 8, Math.min(box.x + box.width - 8, anchorX)), box.y + box.height];
    }
  };

  let minX = 0;
  let minY = 0;
  let maxX = cursorX;
  let maxY = cursorY;
  const expand = (b: Box): void => {
    minX = Math.min(minX, b.x - MARGIN);
    minY = Math.min(minY, b.y - MARGIN);
    maxX = Math.max(maxX, b.x + b.width + MARGIN);
    maxY = Math.max(maxY, b.y + b.height + MARGIN);
  };
  for (const b of nodeBoxes.values()) expand(b);
  for (const b of groupBoxes.values()) expand(b);

  const back: string[] = [];
  const mid: string[] = [];
  const front: string[] = [];

  // Draw groups from outermost to innermost so nested groups sit neatly inside parents.
  const outerFirst = [...orderedGroups].reverse();
  outerFirst.forEach((grp, idx) => {
    const b = groupBoxes.get(grp.id);
    if (!b) return;
    back.push(
      rect(b.x, b.y, b.width, b.height, {
        rx: 8,
        fill: color.groupFill,
        stroke: color.innerStroke,
        "stroke-width": 1.2,
        "stroke-dasharray": "6 4",
      }),
    );
    back.push(drawArchIcon(grp.icon, b.x + 18, b.y + 16, 16, series(idx)));
    back.push(
      textLines(
        b.x + 32,
        b.y + 16,
        [fitWidth(clampLabel(grp.label), b.width - 42, { size: FONT.body, bold: true })],
        { size: FONT.body, weight: "600", fill: color.textSec },
      ),
    );
  });

  // Draw edges orthogonally from exact ports.
  model.edges.forEach((edge, i) => {
    const p0 = portPoint(edge, "from", i);
    const p3 = portPoint(edge, "to", i);
    const [ox0, oy0] = outVec(edge.fromSide);
    const [ox1, oy1] = outVec(edge.toSide);
    const p1: Point = [p0[0] + ox0 * STUB, p0[1] + oy0 * STUB];
    const p2: Point = [p3[0] + ox1 * STUB, p3[1] + oy1 * STUB];

    const pts: Point[] = [p0];
    const isHoriz0 = ox0 !== 0;
    const isHoriz1 = ox1 !== 0;
    if (isHoriz0 && isHoriz1) {
      if (Math.abs(p0[1] - p3[1]) < 1 && ox0 * (p3[0] - p0[0]) > 0 && ox1 * (p0[0] - p3[0]) > 0) {
        pts.push(p3);
      } else {
        const mx = (p1[0] + p2[0]) / 2;
        pts.push(p1, [mx, p1[1]], [mx, p2[1]], p2, p3);
      }
    } else if (!isHoriz0 && !isHoriz1) {
      if (Math.abs(p0[0] - p3[0]) < 1 && oy0 * (p3[1] - p0[1]) > 0 && oy1 * (p0[1] - p3[1]) > 0) {
        pts.push(p3);
      } else {
        const my = (p1[1] + p2[1]) / 2;
        pts.push(p1, [p1[0], my], [p2[0], my], p2, p3);
      }
    } else if (isHoriz0) {
      pts.push(p1, [p2[0], p1[1]], p2, p3);
    } else {
      pts.push(p1, [p1[0], p2[1]], p2, p3);
    }

    for (const [px, py] of pts) {
      minX = Math.min(minX, px - MARGIN);
      minY = Math.min(minY, py - MARGIN);
      maxX = Math.max(maxX, px + MARGIN);
      maxY = Math.max(maxY, py + MARGIN);
    }

    mid.push(
      path(roundedPath(pts, 7), {
        fill: "none",
        stroke: color.line,
        "stroke-width": 1.4,
        "marker-start": edge.fromArrow ? markerRef("arrow-filled") : undefined,
        "marker-end": edge.toArrow ? markerRef("arrow-filled") : undefined,
      }),
    );
  });

  // Draw services and junctions on top.
  nodeSpecs.forEach((spec, idx) => {
    const b = nodeBoxes.get(spec.id);
    if (!b) return;
    const cx = b.x + b.width / 2;
    const cy = b.y + b.height / 2;
    if (spec.kind === "junction") {
      front.push(
        circle(cx, cy, 5, {
          fill: color.arrow,
          stroke: color.canvas,
          "stroke-width": 1.5,
        }),
      );
      return;
    }
    front.push(
      rect(b.x, b.y, b.width, b.height, {
        rx: 6,
        fill: color.nodeFill,
        stroke: color.nodeStroke,
        "stroke-width": 1.2,
      }),
    );
    front.push(drawArchIcon(spec.icon, cx, b.y + 12 + ICON_SIZE / 2, ICON_SIZE, series(idx)));
    front.push(
      textLines(
        cx,
        b.y + 12 + ICON_SIZE + 12,
        spec.lines,
        { size: FONT.small + 1, weight: "500", anchor: "middle" },
        1.25,
      ),
    );
  });

  const defs = markerDefs(["arrow-filled"]);
  const body = el(
    "g",
    { transform: `translate(${n(-minX)},${n(-minY)})` },
    [...back, ...mid, ...front].join(""),
  );
  return svgDoc(maxX - minX, maxY - minY, body, defs);
}
