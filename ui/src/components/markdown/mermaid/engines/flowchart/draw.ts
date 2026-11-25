/**
 * Flowchart drawing using @dagrejs/dagre for compound graph layout and pure-SVG builders for
 * nodes, subgraphs (with isolated subgraph orientation and collapsed folding), rounded paths,
 * arrowheads/circle/cross markers, and edge label pills.
 */
import * as dagreNs from "@dagrejs/dagre";

function resolveDagre(): typeof dagreNs {
  const bag = dagreNs as { layout?: unknown; graphlib?: unknown; default?: typeof dagreNs };
  if (typeof bag.layout === "function" && bag.graphlib !== undefined) return dagreNs;
  if (bag.default !== undefined) return bag.default;
  return dagreNs;
}

const dagre = resolveDagre();
import { roundedPath, type Point } from "../../layout/layered";
import { StepBudget } from "../../limits";
import { fitWidth, textWidth } from "../../measure";
import { circle, el, line, markerDefs, markerRef, n, path, rect, svgDoc, text } from "../../svg";
import { FONT, color, series } from "../../theme";
import { clipShapePoint, drawNodeShape, measureNodeShape, type MeasuredNode } from "./shapes";
import type {
  FlowDirection,
  FlowEdge,
  FlowEndpointMark,
  FlowSubgraph,
  FlowchartModel,
} from "./types";

const PAD = 20;
const SG_PAD_X = 16;
const SG_PAD_BOT = 16;
const SG_HEADER_H = 30;
const EDGE_LABEL_STYLE = { size: FONT.small } as const;

interface PlacedBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface LaidGraph {
  width: number;
  height: number;
  back: string[];
  mid: string[];
  front: string[];
}

function unitVec(a: Point, b: Point): [number, number] {
  const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
  return len > 1e-6 ? [(b[0] - a[0]) / len, (b[1] - a[1]) / len] : [1, 0];
}

function pointInBox(p: Point, box: PlacedBox, inset = 1): boolean {
  return (
    Math.abs(p[0] - box.x) < box.width / 2 - inset &&
    Math.abs(p[1] - box.y) < box.height / 2 - inset
  );
}

function clipRect(box: PlacedBox, target: Point): Point {
  const dx = target[0] - box.x;
  const dy = target[1] - box.y;
  if (Math.abs(dx) < 1e-6 && Math.abs(dy) < 1e-6) return [box.x, box.y];
  const sx = Math.abs(dx) > 1e-6 ? box.width / 2 / Math.abs(dx) : Number.POSITIVE_INFINITY;
  const sy = Math.abs(dy) > 1e-6 ? box.height / 2 / Math.abs(dy) : Number.POSITIVE_INFINITY;
  const t = Math.min(sx, sy);
  return [box.x + dx * t, box.y + dy * t];
}

function polylineMidpoint(pts: readonly Point[]): Point {
  if (pts.length === 0) return [0, 0];
  if (pts.length === 1) return pts[0] ?? [0, 0];
  let total = 0;
  for (let i = 1; i < pts.length; i++) {
    const a = pts[i - 1] as Point;
    const b = pts[i] as Point;
    total += Math.hypot(b[0] - a[0], b[1] - a[1]);
  }
  let target = total / 2;
  for (let i = 1; i < pts.length; i++) {
    const a = pts[i - 1] as Point;
    const b = pts[i] as Point;
    const seg = Math.hypot(b[0] - a[0], b[1] - a[1]);
    if (seg >= target || i === pts.length - 1) {
      const t = seg > 1e-6 ? target / seg : 0.5;
      return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    }
    target -= seg;
  }
  return pts[0] ?? [0, 0];
}

function markToAttr(mark: FlowEndpointMark): string | undefined {
  if (mark === "arrow") return markerRef("arrow-filled");
  if (mark === "cross") return markerRef("arrow-cross");
  return undefined;
}

function drawSubgraphCard(
  back: string[],
  sx: number,
  sy: number,
  width: number,
  height: number,
  title: string,
  tint: string,
): void {
  back.push(
    rect(sx, sy, width, height, {
      rx: 8,
      fill: color.groupFill,
      stroke: color.innerStroke,
      "stroke-width": 1.2,
    }),
  );
  if (!title) return;
  back.push(rect(sx, sy, width, 24, { rx: 8, fill: color.groupHdr, stroke: "none" }));
  back.push(rect(sx, sy + 18, width, 6, { fill: color.groupHdr, stroke: "none" }));
  back.push(
    line(sx, sy + 24, sx + width, sy + 24, {
      stroke: tint,
      "stroke-opacity": 0.45,
      "stroke-width": 1,
    }),
  );
  back.push(
    text(sx + 12, sy + 12, fitWidth(title, width - 24, { size: FONT.body, bold: true }), {
      size: FONT.body,
      weight: "600",
      fill: color.textSec,
    }),
  );
}

/** Draw a parsed flowchart model as a complete passive SVG string. */
export function drawFlowchart(model: FlowchartModel): string {
  const budget = new StepBudget();
  const sgById = new Map(model.subgraphs.map((s) => [s.id, s]));
  const nodeById = new Map(model.nodes.map((nd) => [nd.id, nd]));
  const measured = new Map<string, MeasuredNode>();
  for (const nd of model.nodes) {
    budget.tick();
    measured.set(nd.id, measureNodeShape(nd, model.direction));
  }

  // Collapse ancestry lookup (outermost collapsed subgraph wins).
  const collapsedAncestor = (id: string): string => {
    let cur: string | undefined = nodeById.get(id)?.subgraphId ?? id;
    if (sgById.has(id)) cur = id;
    let topmost: string | undefined;
    let guard = 0;
    while (cur !== undefined && guard < 32) {
      guard++;
      budget.tick();
      const sg = sgById.get(cur);
      if (!sg) break;
      if (sg.collapsed) topmost = sg.id;
      cur = sg.parentId;
    }
    return topmost ?? id;
  };

  const countSubgraphItems = (sgId: string): number => {
    const sg = sgById.get(sgId);
    if (!sg) return 0;
    let total = sg.nodeIds.length;
    for (const childId of sg.childSubgraphIds) {
      total += 1 + countSubgraphItems(childId);
    }
    return total;
  };

  const visibleEdges = model.edges
    .map((e) => ({
      ...e,
      from: collapsedAncestor(e.from),
      to: collapsedAncestor(e.to),
    }))
    .filter((e, idx) => {
      const orig = model.edges[idx];
      if (!orig) return false;
      return !(e.from === e.to && orig.from !== orig.to);
    });

  // Collect all node + descendant subgraph IDs strictly inside `sgId` (excluding `sgId` itself).
  const internalSetCache = new Map<string, Set<string>>();
  const internalIds = (sgId: string): Set<string> => {
    const cached = internalSetCache.get(sgId);
    if (cached) return cached;
    const set = new Set<string>();
    const sg = sgById.get(sgId);
    if (sg) {
      for (const nid of sg.nodeIds) set.add(nid);
      for (const cid of sg.childSubgraphIds) {
        set.add(cid);
        for (const sub of internalIds(cid)) set.add(sub);
      }
    }
    internalSetCache.set(sgId, set);
    return set;
  };

  const isIsolatedCustomDir = (sg: FlowSubgraph, parentDir: FlowDirection): boolean => {
    if (sg.collapsed || !sg.direction || sg.direction === parentDir) return false;
    const inside = internalIds(sg.id);
    if (inside.size === 0) return false;
    for (const e of visibleEdges) {
      const aIn = inside.has(e.from);
      const bIn = inside.has(e.to);
      if (aIn !== bIn) return false;
    }
    return true;
  };

  const layoutRegion = (
    scopeSgId: string | undefined,
    dir: FlowDirection,
    depth: number,
  ): LaidGraph => {
    budget.tick();
    const g = new dagre.graphlib.Graph({ directed: true, multigraph: true, compound: true });
    g.setGraph({
      rankdir: dir,
      nodesep: 42,
      ranksep: 60,
      edgesep: 22,
      marginx: PAD,
      marginy: PAD,
    });
    g.setDefaultEdgeLabel(() => ({}));

    const childSubgraphs = model.subgraphs.filter((s) => s.parentId === scopeSgId);
    const childNodes = model.nodes.filter((nd) => nd.subgraphId === scopeSgId);
    const preLaidSubgraphs = new Map<string, { laid: LaidGraph; sg: FlowSubgraph }>();
    const regionCompoundSubgraphs: FlowSubgraph[] = [];
    const collapsedSubgraphs: FlowSubgraph[] = [];
    const regionNodes = [...childNodes];

    const addCompoundSubgraph = (sg: FlowSubgraph): void => {
      if (sg.collapsed) {
        collapsedSubgraphs.push(sg);
        const title = sg.title || sg.id;
        const sub = `${String(countSubgraphItems(sg.id))} items (collapsed)`;
        const w = Math.max(
          130,
          Math.max(
            textWidth(title, { size: FONT.body, bold: true }),
            textWidth(sub, { size: FONT.small }),
          ) + 32,
        );
        g.setNode(sg.id, { width: w, height: 54 });
        if (sg.parentId !== undefined && sg.parentId !== scopeSgId) {
          g.setParent(sg.id, sg.parentId);
        }
        return;
      }

      if (isIsolatedCustomDir(sg, dir)) {
        const subLaid = layoutRegion(sg.id, sg.direction ?? dir, depth + 1);
        const titleW = sg.title ? textWidth(sg.title, { size: FONT.body, bold: true }) + 28 : 0;
        const hdr = sg.title ? SG_HEADER_H : 12;
        const w = Math.max(subLaid.width + SG_PAD_X * 2, titleW, 110);
        const h = subLaid.height + hdr + SG_PAD_BOT;
        preLaidSubgraphs.set(sg.id, { laid: subLaid, sg });
        g.setNode(sg.id, { width: w, height: h });
        if (sg.parentId !== undefined && sg.parentId !== scopeSgId) {
          g.setParent(sg.id, sg.parentId);
        }
        return;
      }

      const empty = sg.nodeIds.length === 0 && sg.childSubgraphIds.length === 0;
      if (empty) {
        const titleW = sg.title ? textWidth(sg.title, { size: FONT.body, bold: true }) + 28 : 0;
        g.setNode(sg.id, { width: Math.max(120, titleW), height: 64 });
      } else {
        g.setNode(sg.id, {});
      }
      if (sg.parentId !== undefined && sg.parentId !== scopeSgId) {
        g.setParent(sg.id, sg.parentId);
      }
      regionCompoundSubgraphs.push(sg);

      for (const nid of sg.nodeIds) {
        const nd = nodeById.get(nid);
        if (nd) regionNodes.push(nd);
      }
      for (const cid of sg.childSubgraphIds) {
        const child = sgById.get(cid);
        if (child) addCompoundSubgraph(child);
      }
    };

    for (const sg of childSubgraphs) {
      addCompoundSubgraph(sg);
    }

    // Re-measure nodes in isolated subgraphs using the subgraph's own orientation (for fork bars).
    const regionMeasured = new Map<string, MeasuredNode>();
    for (const nd of regionNodes) {
      const m = dir === model.direction ? measured.get(nd.id) : measureNodeShape(nd, dir);
      if (m) {
        regionMeasured.set(nd.id, m);
        g.setNode(nd.id, { width: m.width, height: m.height });
        if (nd.subgraphId !== undefined && nd.subgraphId !== scopeSgId) {
          g.setParent(nd.id, nd.subgraphId);
        }
      }
    }

    const memberIds = new Set<string>([
      ...regionNodes.map((nd) => nd.id),
      ...collapsedSubgraphs.map((s) => s.id),
      ...preLaidSubgraphs.keys(),
      ...regionCompoundSubgraphs.map((s) => s.id),
    ]);

    // Dagre crashes when edges attach directly to compound parents (nodes with children in `g`).
    // Proxy non-empty compound endpoints via descendant leaves during layout, then clip back to the box.
    const compoundParents = new Set(
      regionCompoundSubgraphs.filter((sg) => g.children(sg.id).length > 0).map((sg) => sg.id),
    );
    const collectLeaves = (sgId: string): string[] => {
      const out: string[] = [];
      const visit = (id: string): void => {
        const ch = g.children(id);
        if (ch.length === 0) {
          out.push(id);
          return;
        }
        for (const c of ch) visit(c);
      };
      visit(sgId);
      return out;
    };
    const pickProxy = (sgId: string, role: "entry" | "exit"): string | undefined => {
      const leaves = collectLeaves(sgId);
      if (leaves.length === 0) return undefined;
      const leafSet = new Set(leaves);
      const hasIn = new Set<string>();
      const hasOut = new Set<string>();
      for (const e of visibleEdges) {
        if (leafSet.has(e.from) && leafSet.has(e.to) && e.from !== e.to) {
          hasOut.add(e.from);
          hasIn.add(e.to);
        }
      }
      if (role === "entry") return leaves.find((id) => !hasIn.has(id)) ?? leaves[0];
      for (let i = leaves.length - 1; i >= 0; i--) {
        const id = leaves[i];
        if (id && !hasOut.has(id)) return id;
      }
      return leaves[leaves.length - 1];
    };

    const regionEdges: {
      edge: FlowEdge;
      key: string;
      layoutFrom: string;
      layoutTo: string;
      syntheticLoop: boolean;
    }[] = [];

    visibleEdges.forEach((edge, idx) => {
      if (!memberIds.has(edge.from) || !memberIds.has(edge.to)) return;
      const key = `e_${String(idx)}`;
      const fromCompound = compoundParents.has(edge.from);
      const toCompound = compoundParents.has(edge.to);
      if (edge.from === edge.to && fromCompound) {
        regionEdges.push({
          edge,
          key,
          layoutFrom: edge.from,
          layoutTo: edge.to,
          syntheticLoop: true,
        });
        return;
      }
      const layoutFrom = fromCompound ? (pickProxy(edge.from, "exit") ?? edge.from) : edge.from;
      const layoutTo = toCompound ? (pickProxy(edge.to, "entry") ?? edge.to) : edge.to;
      if ((fromCompound || toCompound) && layoutFrom === layoutTo) {
        regionEdges.push({ edge, key, layoutFrom, layoutTo, syntheticLoop: true });
        return;
      }
      const labelW = edge.label
        ? textWidth(fitWidth(edge.label, 180, EDGE_LABEL_STYLE), EDGE_LABEL_STYLE) + 14
        : 0;
      g.setEdge(
        layoutFrom,
        layoutTo,
        {
          minlen: edge.minlen,
          width: labelW,
          height: edge.label ? 20 : 0,
          labelpos: "c",
        },
        key,
      );
      regionEdges.push({ edge, key, layoutFrom, layoutTo, syntheticLoop: false });
    });

    dagre.layout(g);

    const nodeBoxes = new Map<string, PlacedBox>();
    for (const nd of regionNodes) {
      const pos = g.node(nd.id) as
        { x?: number; y?: number; width?: number; height?: number } | undefined;
      if (pos && typeof pos.x === "number" && typeof pos.y === "number") {
        const m = regionMeasured.get(nd.id);
        nodeBoxes.set(nd.id, {
          x: pos.x,
          y: pos.y,
          width: m?.width ?? pos.width ?? 60,
          height: m?.height ?? pos.height ?? 36,
        });
      }
    }
    for (const sg of collapsedSubgraphs) {
      const pos = g.node(sg.id) as
        { x?: number; y?: number; width?: number; height?: number } | undefined;
      if (pos && typeof pos.x === "number" && typeof pos.y === "number") {
        nodeBoxes.set(sg.id, {
          x: pos.x,
          y: pos.y,
          width: pos.width ?? 130,
          height: pos.height ?? 54,
        });
      }
    }
    for (const [id, entry] of preLaidSubgraphs) {
      const pos = g.node(id) as
        { x?: number; y?: number; width?: number; height?: number } | undefined;
      if (pos && typeof pos.x === "number" && typeof pos.y === "number") {
        nodeBoxes.set(id, {
          x: pos.x,
          y: pos.y,
          width: pos.width ?? entry.laid.width + SG_PAD_X * 2,
          height: pos.height ?? entry.laid.height + SG_HEADER_H + SG_PAD_BOT,
        });
      }
    }

    // Compute compound subgraph bounding boxes bottom-up so nested headers never collide.
    const sgBoxes = new Map<string, PlacedBox>();
    const computeSgBox = (sg: FlowSubgraph): PlacedBox | undefined => {
      const existing = sgBoxes.get(sg.id);
      if (existing) return existing;
      if (sg.nodeIds.length === 0 && sg.childSubgraphIds.length === 0) {
        const pos = g.node(sg.id) as
          { x?: number; y?: number; width?: number; height?: number } | undefined;
        if (!pos || typeof pos.x !== "number" || typeof pos.y !== "number") return undefined;
        const box = { x: pos.x, y: pos.y, width: pos.width ?? 120, height: pos.height ?? 64 };
        sgBoxes.set(sg.id, box);
        nodeBoxes.set(sg.id, box);
        return box;
      }

      let x1 = Number.POSITIVE_INFINITY;
      let y1 = Number.POSITIVE_INFINITY;
      let x2 = Number.NEGATIVE_INFINITY;
      let y2 = Number.NEGATIVE_INFINITY;

      for (const nid of sg.nodeIds) {
        const b = nodeBoxes.get(nid);
        if (!b) continue;
        x1 = Math.min(x1, b.x - b.width / 2);
        y1 = Math.min(y1, b.y - b.height / 2);
        x2 = Math.max(x2, b.x + b.width / 2);
        y2 = Math.max(y2, b.y + b.height / 2);
      }
      for (const cid of sg.childSubgraphIds) {
        const child = sgById.get(cid);
        if (!child) continue;
        const cb =
          child.collapsed || preLaidSubgraphs.has(cid) ? nodeBoxes.get(cid) : computeSgBox(child);
        if (!cb) continue;
        x1 = Math.min(x1, cb.x - cb.width / 2);
        y1 = Math.min(y1, cb.y - cb.height / 2);
        x2 = Math.max(x2, cb.x + cb.width / 2);
        y2 = Math.max(y2, cb.y + cb.height / 2);
      }
      if (!Number.isFinite(x1)) return undefined;

      const hdr = sg.title ? SG_HEADER_H : 14;
      const minW = sg.title ? textWidth(sg.title, { size: FONT.body, bold: true }) + 28 : 0;
      const rawW = x2 - x1 + SG_PAD_X * 2;
      const width = Math.max(rawW, minW);
      const extraX = (width - rawW) / 2;
      const left = x1 - SG_PAD_X - extraX;
      const top = y1 - hdr;
      const height = y2 + SG_PAD_BOT - top;
      const box = { x: left + width / 2, y: top + height / 2, width, height };
      sgBoxes.set(sg.id, box);
      nodeBoxes.set(sg.id, box);
      return box;
    };

    for (const sg of regionCompoundSubgraphs) computeSgBox(sg);

    const back: string[] = [];
    const mid: string[] = [];
    const front: string[] = [];

    // 1. Subgraph containers (in declaration order so parents sit behind children).
    regionCompoundSubgraphs.forEach((sg, idx) => {
      const b = sgBoxes.get(sg.id);
      if (!b) return;
      drawSubgraphCard(
        back,
        b.x - b.width / 2,
        b.y - b.height / 2,
        b.width,
        b.height,
        sg.title,
        series(depth + idx),
      );
    });

    // 2. Pre-laid-out isolated subgraphs with independent orientation.
    let isoIdx = 0;
    for (const [id, entry] of preLaidSubgraphs) {
      const b = nodeBoxes.get(id);
      if (!b) continue;
      const sx = b.x - b.width / 2;
      const sy = b.y - b.height / 2;
      const tint = series(depth + regionCompoundSubgraphs.length + isoIdx);
      isoIdx++;
      const hdr = entry.sg.title ? SG_HEADER_H : 12;
      drawSubgraphCard(back, sx, sy, b.width, b.height, entry.sg.title, tint);
      const ox = sx + (b.width - entry.laid.width) / 2;
      const oy = sy + hdr;
      back.push(el("g", { transform: `translate(${n(ox)},${n(oy)})` }, entry.laid.back.join("")));
      mid.push(el("g", { transform: `translate(${n(ox)},${n(oy)})` }, entry.laid.mid.join("")));
      front.push(el("g", { transform: `translate(${n(ox)},${n(oy)})` }, entry.laid.front.join("")));
    }

    // 3. Edges (invisible edges still shaped the layout above, and are skipped here).
    for (const { edge, key, layoutFrom, layoutTo, syntheticLoop } of regionEdges) {
      if (edge.style === "invisible") continue;
      budget.tick();
      const laidEdge = syntheticLoop
        ? undefined
        : (g.edge(layoutFrom, layoutTo, key) as
            { points?: { x: number; y: number }[]; x?: number; y?: number } | undefined);

      const fromBox = nodeBoxes.get(edge.from);
      const toBox = nodeBoxes.get(edge.to);
      let pts: Point[];

      if ((edge.from === edge.to || syntheticLoop) && fromBox) {
        const rx = fromBox.x + fromBox.width / 2;
        pts = [
          [rx, fromBox.y - 10],
          [rx + 28, fromBox.y - 10],
          [rx + 28, fromBox.y + 10],
          [rx, fromBox.y + 10],
        ];
      } else {
        if (!laidEdge?.points || laidEdge.points.length === 0) continue;
        pts = laidEdge.points.map((p): Point => [p.x, p.y]);
        const fromIsCompound = edge.from !== layoutFrom;
        const toIsCompound = edge.to !== layoutTo;

        if (fromIsCompound && fromBox) {
          pts = pts.filter((p) => !pointInBox(p, fromBox));
          pts.unshift([fromBox.x, fromBox.y]);
        }
        if (toIsCompound && toBox) {
          pts = pts.filter((p) => !pointInBox(p, toBox));
          pts.push([toBox.x, toBox.y]);
        }

        if (pts.length >= 2) {
          const fromM = regionMeasured.get(edge.from);
          const toM = regionMeasured.get(edge.to);
          const pNext = pts[1] as Point;
          const pPrev = pts[pts.length - 2] as Point;
          if (fromM && fromBox && !fromM.glyphWithCaption) {
            pts[0] = clipShapePoint(fromM, fromBox.x, fromBox.y, pNext[0], pNext[1]);
          } else if (fromBox) {
            pts[0] = clipRect(fromBox, pNext);
          }
          if (toM && toBox && !toM.glyphWithCaption) {
            pts[pts.length - 1] = clipShapePoint(toM, toBox.x, toBox.y, pPrev[0], pPrev[1]);
          } else if (toBox) {
            pts[pts.length - 1] = clipRect(toBox, pPrev);
          }
        }
      }

      // Shorten path ends by circle radius when endpoint is a hollow circle marker.
      const circleDots: string[] = [];
      if (edge.startMark === "circle" && pts.length >= 2) {
        const first = pts[0] as Point;
        const second = pts[1] as Point;
        const [ux, uy] = unitVec(first, second);
        const cx = first[0] + ux * 4.5;
        const cy = first[1] + uy * 4.5;
        circleDots.push(
          circle(cx, cy, 4.5, { fill: color.canvas, stroke: color.arrow, "stroke-width": 1.4 }),
        );
        pts[0] = [first[0] + ux * 9, first[1] + uy * 9];
      }
      if (edge.endMark === "circle" && pts.length >= 2) {
        const last = pts[pts.length - 1] as Point;
        const prev = pts[pts.length - 2] as Point;
        const [ux, uy] = unitVec(last, prev);
        const cx = last[0] + ux * 4.5;
        const cy = last[1] + uy * 4.5;
        circleDots.push(
          circle(cx, cy, 4.5, { fill: color.canvas, stroke: color.arrow, "stroke-width": 1.4 }),
        );
        pts[pts.length - 1] = [last[0] + ux * 9, last[1] + uy * 9];
      }

      mid.push(
        path(roundedPath(pts, edge.from === edge.to ? 6 : 8), {
          fill: "none",
          stroke: color.line,
          "stroke-width": edge.style === "thick" ? 2.4 : 1.35,
          "stroke-dasharray": edge.style === "dotted" ? "5 4" : undefined,
          "marker-start": markToAttr(edge.startMark),
          "marker-end": markToAttr(edge.endMark),
        }),
      );
      if (circleDots.length > 0) mid.push(...circleDots);

      if (edge.label) {
        const lbl = fitWidth(edge.label, 180, EDGE_LABEL_STYLE);
        const lw = textWidth(lbl, EDGE_LABEL_STYLE) + 12;
        const wasCompound = edge.from !== layoutFrom || edge.to !== layoutTo;
        let lx = wasCompound ? undefined : laidEdge?.x;
        let ly = wasCompound ? undefined : laidEdge?.y;
        if (edge.from === edge.to && fromBox) {
          lx = fromBox.x + fromBox.width / 2 + 34 + lw / 2;
          ly = fromBox.y;
        } else if (lx === undefined || ly === undefined) {
          const mp = polylineMidpoint(pts);
          lx = mp[0];
          ly = mp[1];
        }
        front.push(
          rect(lx - lw / 2, ly - 9, lw, 18, {
            rx: 4,
            fill: color.canvas,
            "fill-opacity": 0.94,
            stroke: color.innerStroke,
            "stroke-width": 0.7,
          }),
        );
        front.push(
          text(lx, ly, lbl, {
            size: FONT.small,
            anchor: "middle",
            fill: color.textSec,
          }),
        );
      }
    }

    // 4. Collapsed subgraph nodes.
    for (const sg of collapsedSubgraphs) {
      const b = nodeBoxes.get(sg.id);
      if (!b) continue;
      const sx = b.x - b.width / 2;
      const sy = b.y - b.height / 2;
      front.push(
        rect(sx, sy, b.width, b.height, {
          rx: 8,
          fill: color.groupHdr,
          stroke: color.nodeStroke,
          "stroke-width": 1.4,
          "stroke-dasharray": "5 3",
        }),
      );
      front.push(
        text(
          b.x,
          sy + 18,
          fitWidth(sg.title || sg.id, b.width - 20, { size: FONT.body, bold: true }),
          {
            size: FONT.body,
            weight: "600",
            anchor: "middle",
          },
        ),
      );
      front.push(
        text(b.x, sy + 36, `${String(countSubgraphItems(sg.id))} items (collapsed)`, {
          size: FONT.small,
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }

    // 5. Regular nodes.
    for (const nd of regionNodes) {
      const b = nodeBoxes.get(nd.id);
      const m = regionMeasured.get(nd.id);
      if (b && m) front.push(drawNodeShape(m, b.x, b.y, dir));
    }

    // Compute exact normalized bounds covering padded subgraph headers and self-loop labels.
    let minX = Number.POSITIVE_INFINITY;
    let minY = Number.POSITIVE_INFINITY;
    let maxX = Number.NEGATIVE_INFINITY;
    let maxY = Number.NEGATIVE_INFINITY;
    for (const b of nodeBoxes.values()) {
      minX = Math.min(minX, b.x - b.width / 2);
      minY = Math.min(minY, b.y - b.height / 2);
      maxX = Math.max(maxX, b.x + b.width / 2);
      maxY = Math.max(maxY, b.y + b.height / 2);
    }
    for (const { edge, key, layoutFrom, layoutTo, syntheticLoop } of regionEdges) {
      if (edge.style === "invisible") continue;
      const laidEdge = syntheticLoop
        ? undefined
        : (g.edge(layoutFrom, layoutTo, key) as
            | {
                points?: { x: number; y: number }[];
                x?: number;
                y?: number;
                width?: number;
                height?: number;
              }
            | undefined);
      for (const p of laidEdge?.points ?? []) {
        minX = Math.min(minX, p.x - 10);
        minY = Math.min(minY, p.y - 10);
        maxX = Math.max(maxX, p.x + 10);
        maxY = Math.max(maxY, p.y + 10);
      }
      if (edge.from === edge.to || syntheticLoop) {
        const fb = nodeBoxes.get(edge.from);
        if (fb) {
          const lw = edge.label
            ? textWidth(fitWidth(edge.label, 180, EDGE_LABEL_STYLE), EDGE_LABEL_STYLE) + 12
            : 0;
          maxX = Math.max(maxX, fb.x + fb.width / 2 + 38 + lw);
        }
      } else if (laidEdge?.x !== undefined && laidEdge.y !== undefined && laidEdge.width) {
        minX = Math.min(minX, laidEdge.x - laidEdge.width / 2);
        maxX = Math.max(maxX, laidEdge.x + laidEdge.width / 2);
        minY = Math.min(minY, laidEdge.y - (laidEdge.height ?? 20) / 2);
        maxY = Math.max(maxY, laidEdge.y + (laidEdge.height ?? 20) / 2);
      }
    }

    if (!Number.isFinite(minX)) {
      return { width: 100, height: 60, back: [], mid: [], front: [] };
    }
    const pad = scopeSgId === undefined ? PAD : 8;
    const dx = pad - minX;
    const dy = pad - minY;
    const wrapShift = (items: string[]): string[] =>
      items.length === 0
        ? []
        : [el("g", { transform: `translate(${n(dx)},${n(dy)})` }, items.join(""))];

    return {
      width: maxX - minX + pad * 2,
      height: maxY - minY + pad * 2,
      back: wrapShift(back),
      mid: wrapShift(mid),
      front: wrapShift(front),
    };
  };

  const root = layoutRegion(undefined, model.direction, 0);
  const defs = markerDefs(["arrow-filled", "arrow-cross"]);
  return svgDoc(root.width, root.height, [...root.back, ...root.mid, ...root.front].join(""), defs);
}
