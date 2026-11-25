/**
 * Layered (Sugiyama-style) graph layout shared by the class, state and ER engines and free for
 * later engines to reuse. Steps: reverse cycle-closing edges, longest-path ranks, virtual nodes
 * for long edges, barycentre ordering sweeps, least-squares cross-axis placement, orthogonal
 * routing with label slots. Written for this app; every loop ticks a StepBudget.
 */
import { DiagramInputError } from "../types";
import type { StepBudget } from "../limits";

/** Flow direction of the layout: top to bottom, bottom to top, left to right, right to left. */
export type Direction = "TB" | "BT" | "LR" | "RL";
/** An x, y coordinate in user units. */
export type Point = [number, number];

/** A node to place, with its drawn size. */
export interface LayoutNode {
  id: string;
  width: number;
  height: number;
}
/** A directed connection by node id, with the size of its label when it has one. */
export interface LayoutEdge {
  from: string;
  to: string;
  labelWidth?: number | undefined;
  labelHeight?: number | undefined;
}
/** Direction, gaps between nodes and ranks, and the outer margin. */
export interface LayoutOptions {
  direction: Direction;
  nodeGap?: number;
  rankGap?: number;
  margin?: number;
}
/** A node with its centre and drawn size. */
export interface PlacedNode {
  id: string;
  /** Centre. */
  x: number;
  y: number;
  width: number;
  height: number;
}
/** A routed edge: points from source to target, whether it is a self loop, and its label slot. */
export interface PlacedEdge {
  from: string;
  to: string;
  /** Route from `from` to `to`, empty for a self loop. */
  points: Point[];
  selfLoop: boolean;
  /** Centre of the label slot when the edge has a label. */
  label?: { x: number; y: number; width: number; height: number } | undefined;
}
/** Placed nodes by id, routed edges in input order, and the size of the drawing. */
export interface Layout {
  nodes: Map<string, PlacedNode>;
  edges: PlacedEdge[];
  width: number;
  height: number;
}

interface Item {
  real: boolean;
  w: number;
  h: number;
  rank: number;
  order: number;
  c: number;
  f: number;
  ups: Item[];
  downs: Item[];
}
interface Chain {
  edge: number;
  reversed: boolean;
  items: Item[];
}

const LOOP_ROOM = 36;

/** Least-squares placement of centres `desired` keeping `sep[i]` between item i-1 and i. */
function resolve(desired: number[], sep: number[]): number[] {
  const offset: number[] = [];
  let sum = 0;
  desired.forEach((_, i) => {
    sum += i === 0 ? 0 : (sep[i] ?? 0);
    offset.push(sum);
  });
  const blocks: { mean: number; count: number }[] = [];
  desired.forEach((d, i) => {
    blocks.push({ mean: d - (offset[i] ?? 0), count: 1 });
    for (;;) {
      const last = blocks[blocks.length - 1];
      const prev = blocks[blocks.length - 2];
      if (!last || !prev || prev.mean <= last.mean) break;
      blocks.splice(blocks.length - 2, 2, {
        mean: (prev.mean * prev.count + last.mean * last.count) / (prev.count + last.count),
        count: prev.count + last.count,
      });
    }
  });
  const out: number[] = [];
  for (const block of blocks)
    for (let k = 0; k < block.count; k++) out.push(block.mean + (offset[out.length] ?? 0));
  return out;
}

/** Orthogonal route with rounded corners as an SVG path `d`. */
export function roundedPath(points: readonly Point[], radius = 8): string {
  const first = points[0];
  if (!first) return "";
  let d = `M${r2(first[0])},${r2(first[1])}`;
  for (let i = 1; i < points.length; i++) {
    const cur = points[i] as Point;
    const prev = points[i - 1] as Point;
    const next = points[i + 1];
    if (!next) {
      d += ` L${r2(cur[0])},${r2(cur[1])}`;
      break;
    }
    const lenIn = Math.hypot(cur[0] - prev[0], cur[1] - prev[1]);
    const lenOut = Math.hypot(next[0] - cur[0], next[1] - cur[1]);
    const rr = Math.min(radius, lenIn / 2, lenOut / 2);
    if (rr < 0.5) {
      d += ` L${r2(cur[0])},${r2(cur[1])}`;
      continue;
    }
    const a: Point = [
      cur[0] - ((cur[0] - prev[0]) / lenIn) * rr,
      cur[1] - ((cur[1] - prev[1]) / lenIn) * rr,
    ];
    const b: Point = [
      cur[0] + ((next[0] - cur[0]) / lenOut) * rr,
      cur[1] + ((next[1] - cur[1]) / lenOut) * rr,
    ];
    d += ` L${r2(a[0])},${r2(a[1])} Q${r2(cur[0])},${r2(cur[1])} ${r2(b[0])},${r2(b[1])}`;
  }
  return d;
}
const r2 = (v: number): string => String(Math.round(v * 100) / 100);

/** Lay nodes and edges out in ranks; throws DiagramInputError for an edge to an unknown node and DiagramLimitError when the step budget runs out. */
export function layeredLayout(
  nodes: readonly LayoutNode[],
  edges: readonly LayoutEdge[],
  options: LayoutOptions,
  budget: StepBudget,
): Layout {
  const horizontal = options.direction === "LR" || options.direction === "RL";
  const nodeGap = options.nodeGap ?? 36;
  const rankGap = options.rankGap ?? 56;
  const margin = options.margin ?? 16;
  const index = new Map<string, number>();
  nodes.forEach((node, i) => index.set(node.id, i));
  const selfLoopRoom = new Map<string, number>();
  const real: Item[] = nodes.map((node, i) => ({
    real: true,
    w: horizontal ? node.height : node.width,
    h: horizontal ? node.width : node.height,
    rank: 0,
    order: i,
    c: 0,
    f: 0,
    ups: [],
    downs: [],
  }));
  const laidOut: { from: number; to: number; edge: number }[] = [];
  edges.forEach((edge, e) => {
    const from = index.get(edge.from);
    const to = index.get(edge.to);
    if (from === undefined || to === undefined)
      throw new DiagramInputError("Edge to an unknown node");
    if (from === to) {
      const labelCross = (horizontal ? edge.labelHeight : edge.labelWidth) ?? 0;
      selfLoopRoom.set(
        edge.from,
        Math.max(selfLoopRoom.get(edge.from) ?? 0, LOOP_ROOM + labelCross),
      );
    } else laidOut.push({ from, to, edge: e });
  });
  real.forEach((item, i) => {
    const room = selfLoopRoom.get(nodes[i]?.id ?? "");
    if (room) item.w += room * 2;
  });

  // 1. Reverse the edges that close cycles (iterative DFS, input order).
  const out: number[][] = nodes.map(() => []);
  laidOut.forEach((e, k) => out[e.from]?.push(k));
  const state = new Array<number>(nodes.length).fill(0);
  const reversed = new Set<number>();
  for (let root = 0; root < nodes.length; root++) {
    if (state[root] !== 0) continue;
    const stack: { node: number; next: number }[] = [{ node: root, next: 0 }];
    state[root] = 1;
    while (stack.length > 0) {
      budget.tick();
      const top = stack[stack.length - 1] as { node: number; next: number };
      const k = out[top.node]?.[top.next++];
      if (k === undefined) {
        state[top.node] = 2;
        stack.pop();
        continue;
      }
      const to = (laidOut[k] as { to: number }).to;
      if (state[to] === 1) reversed.add(k);
      else if (state[to] === 0) {
        state[to] = 1;
        stack.push({ node: to, next: 0 });
      }
    }
  }
  const dag = laidOut.map((e, k) => (reversed.has(k) ? { ...e, from: e.to, to: e.from } : e));

  // 2. Longest-path ranks (Kahn), then pull pure sources down next to their first successor.
  const indeg = new Array<number>(nodes.length).fill(0);
  const succ: number[][] = nodes.map(() => []);
  for (const e of dag) {
    indeg[e.to] = (indeg[e.to] ?? 0) + 1;
    succ[e.from]?.push(e.to);
  }
  const queue = real.map((_, i) => i).filter((i) => indeg[i] === 0);
  for (let head = 0; head < queue.length; head++) {
    budget.tick();
    const u = queue[head] as number;
    for (const v of succ[u] ?? []) {
      (real[v] as Item).rank = Math.max((real[v] as Item).rank, (real[u] as Item).rank + 1);
      indeg[v] = (indeg[v] ?? 1) - 1;
      if (indeg[v] === 0) queue.push(v);
    }
  }
  real.forEach((item, i) => {
    const next = succ[i] ?? [];
    if (next.length > 0 && !dag.some((e) => e.to === i)) {
      item.rank = Math.min(...next.map((v) => (real[v] as Item).rank)) - 1;
    }
  });
  const minRank = Math.min(0, ...real.map((item) => item.rank));
  for (const item of real) item.rank -= minRank;

  // 3. Virtual nodes for edges that span several ranks.
  const chains: Chain[] = [];
  for (const [k, e] of dag.entries()) {
    const a = real[e.from] as Item;
    const b = real[e.to] as Item;
    const items = [a];
    for (let rank = a.rank + 1; rank < b.rank; rank++) {
      budget.tick();
      items.push({ real: false, w: 2, h: 0, rank, order: 0, c: 0, f: 0, ups: [], downs: [] });
    }
    items.push(b);
    items.forEach((item, i) => {
      const nextItem = items[i + 1];
      if (nextItem) {
        item.downs.push(nextItem);
        nextItem.ups.push(item);
      }
    });
    chains.push({ edge: (laidOut[k] as { edge: number }).edge, reversed: reversed.has(k), items });
  }
  const rankCount = Math.max(-1, ...real.map((item) => item.rank)) + 1;
  const layers: Item[][] = Array.from({ length: Math.max(rankCount, 0) }, () => []);
  for (const item of real) layers[item.rank]?.push(item);
  for (const chain of chains)
    for (const item of chain.items) if (!item.real) layers[item.rank]?.push(item);
  const renumber = (layer: Item[]): void => {
    layer.forEach((item, i) => (item.order = i));
  };
  layers.forEach(renumber);

  // 4. Crossing reduction sweeps.
  const sweep = (layer: Item[], pick: (item: Item) => Item[]): void => {
    const keyed = layer.map((item) => {
      const near = pick(item);
      const key = near.length
        ? near.reduce((s, other) => s + other.order, 0) / near.length
        : item.order;
      return { item, key };
    });
    budget.tick(layer.length);
    keyed.sort((x, y) => x.key - y.key || x.item.order - y.item.order);
    keyed.forEach((entry, i) => (layer[i] = entry.item));
    renumber(layer);
  };
  for (let pass = 0; pass < 8; pass++) {
    for (let r = 1; r < layers.length; r++) sweep(layers[r] as Item[], (item) => item.ups);
    for (let r = layers.length - 2; r >= 0; r--) sweep(layers[r] as Item[], (item) => item.downs);
  }

  // 5. Cross-axis placement.
  const separation = (a: Item, b: Item): number =>
    (a.w + b.w) / 2 + (a.real && b.real ? nodeGap : 12);
  const seps = (layer: Item[]): number[] =>
    layer.map((item, i) => (i === 0 ? 0 : separation(layer[i - 1] as Item, item)));
  for (const layer of layers) {
    const placed = resolve(
      layer.map(() => 0),
      seps(layer),
    );
    layer.forEach((item, i) => (item.c = placed[i] ?? 0));
  }
  for (let pass = 0; pass < 6; pass++) {
    const order = pass % 2 === 0 ? [...layers.keys()] : [...layers.keys()].reverse();
    for (const r of order) {
      const layer = layers[r] as Item[];
      budget.tick(layer.length);
      const desired = layer.map((item) => {
        const near = pass % 2 === 0 ? item.ups : item.downs;
        return near.length ? near.reduce((s, other) => s + other.c, 0) / near.length : item.c;
      });
      const placed = resolve(desired, seps(layer));
      layer.forEach((item, i) => (item.c = placed[i] ?? 0));
    }
  }

  // 6. Flow-axis placement with room for edge labels.
  const labelFlow = (e: number): number => {
    const edge = edges[e] as LayoutEdge;
    const size = horizontal ? edge.labelWidth : edge.labelHeight;
    return size ? size + 8 : 0;
  };
  const gapExtra = new Array<number>(layers.length).fill(0);
  const pairs = new Map<string, number[]>();
  for (const chain of chains) {
    const edge = edges[chain.edge] as LayoutEdge;
    const key = [edge.from, edge.to].sort().join("\u0000");
    pairs.set(key, [...(pairs.get(key) ?? []), chain.edge]);
  }
  const labelRank = (chain: Chain): number =>
    Math.floor((chain.items.length - 2) / 2) + (chain.items[0] as Item).rank;
  for (const members of pairs.values()) {
    const byRank = new Map<number, number>();
    for (const chain of chains.filter((c) => members.includes(c.edge))) {
      const rank = labelRank(chain);
      byRank.set(rank, (byRank.get(rank) ?? 0) + labelFlow(chain.edge));
    }
    for (const [rank, total] of byRank) gapExtra[rank] = Math.max(gapExtra[rank] ?? 0, total);
  }
  let flow = margin;
  layers.forEach((layer, r) => {
    const thick = Math.max(0, ...layer.map((item) => item.h));
    for (const item of layer) item.f = flow + thick / 2;
    flow += thick + rankGap + (gapExtra[r] ?? 0);
  });
  const totalFlow = Math.max(flow - rankGap + margin, margin * 2);
  const lows = Math.min(0, ...layers.flatMap((layer) => layer.map((item) => item.c - item.w / 2)));
  const highs = Math.max(0, ...layers.flatMap((layer) => layer.map((item) => item.c + item.w / 2)));
  const shift = margin - (layers.length ? lows : 0);
  const totalCross = (layers.length ? highs : 0) - (layers.length ? lows : 0) + margin * 2;
  for (const layer of layers) for (const item of layer) item.c += shift;

  // 7. Orientation, node boxes and routes.
  const toXY = (c: number, f: number): Point => {
    switch (options.direction) {
      case "TB":
        return [c, f];
      case "BT":
        return [c, totalFlow - f];
      case "LR":
        return [f, c];
      default:
        return [totalFlow - f, c];
    }
  };
  const placedNodes = new Map<string, PlacedNode>();
  nodes.forEach((node, i) => {
    const item = real[i] as Item;
    const [x, y] = toXY(item.c, item.f);
    placedNodes.set(node.id, { id: node.id, x, y, width: node.width, height: node.height });
  });
  const placedEdges: PlacedEdge[] = edges.map((edge) => ({
    from: edge.from,
    to: edge.to,
    points: [],
    selfLoop: edge.from === edge.to,
  }));
  // Edges leaving or entering the same node side get their own spot along that side, ordered by where
  // the other end lies, so end glyphs and arrowheads do not pile up on the centre.
  const outOffset = new Map<number, number>();
  const inOffset = new Map<number, number>();
  const portsOut = new Map<Item, { chain: number; other: number }[]>();
  const portsIn = new Map<Item, { chain: number; other: number }[]>();
  const addPort = (
    map: Map<Item, { chain: number; other: number }[]>,
    item: Item,
    port: { chain: number; other: number },
  ): void => {
    map.set(item, [...(map.get(item) ?? []), port]);
  };
  chains.forEach((chain, ci) => {
    const its = chain.items;
    addPort(portsOut, its[0] as Item, { chain: ci, other: (its[1] as Item).c });
    addPort(portsIn, its[its.length - 1] as Item, {
      chain: ci,
      other: (its[its.length - 2] as Item).c,
    });
  });
  const spread = (
    map: Map<Item, { chain: number; other: number }[]>,
    into: Map<number, number>,
  ): void => {
    for (const [item, list] of map) {
      budget.tick(list.length);
      list.sort((a, b) => a.other - b.other || a.chain - b.chain);
      const spacing = Math.min(20, Math.max(6, (item.w - 24) / Math.max(list.length - 1, 1)));
      list.forEach((port, k) => into.set(port.chain, (k - (list.length - 1) / 2) * spacing));
    }
  };
  spread(portsOut, outOffset);
  spread(portsIn, inOffset);
  for (const [ci, chain] of chains.entries()) {
    const edge = edges[chain.edge] as LayoutEdge;
    const members = pairs.get([edge.from, edge.to].sort().join("\u0000")) ?? [chain.edge];
    const raw: Point[] = [];
    const items = chain.items;
    let labelAt: { c: number; f: number } | undefined;
    const lr = labelRank(chain);
    items.forEach((item, i) => {
      const nextItem = items[i + 1];
      const c0 = item.c + (i === 0 ? (outOffset.get(ci) ?? 0) : 0);
      const out0 = item.real ? item.f + item.h / 2 : item.f;
      if (!nextItem) {
        if (i === 0) raw.push([c0, out0]);
        return;
      }
      const c1 = nextItem.c + (i + 1 === items.length - 1 ? (inOffset.get(ci) ?? 0) : 0);
      const in1 = nextItem.real ? nextItem.f - nextItem.h / 2 : nextItem.f;
      const mid = (out0 + in1) / 2;
      if (i === 0) raw.push([c0, out0]);
      raw.push([c0, mid], [c1, mid]);
      if (item.rank === lr && !labelAt) {
        const own = members.filter(
          (m) => m !== chain.edge && labelRank(chains.find((c) => c.edge === m) as Chain) === lr,
        );
        const before = own.filter((m) => m < chain.edge).reduce((s, m) => s + labelFlow(m), 0);
        const total = own.reduce((s, m) => s + labelFlow(m), 0) + labelFlow(chain.edge);
        const stacked = labelFlow(chain.edge) / 2 + before - total / 2;
        labelAt = { c: (c0 + c1) / 2, f: mid + stacked };
      }
      raw.push([c1, in1]);
    });
    const dedup = raw.filter(
      (p, i) => i === 0 || p[0] !== (raw[i - 1] as Point)[0] || p[1] !== (raw[i - 1] as Point)[1],
    );
    const ordered = chain.reversed ? [...dedup].reverse() : dedup;
    const target = placedEdges[chain.edge] as PlacedEdge;
    target.points = ordered.map(([c, f]) => toXY(c, f));
    if (edge.labelWidth && edge.labelHeight && labelAt) {
      const [x, y] = toXY(labelAt.c, labelAt.f);
      target.label = { x, y, width: edge.labelWidth, height: edge.labelHeight };
    }
  }
  return {
    nodes: placedNodes,
    edges: placedEdges,
    width: horizontal ? totalFlow : totalCross,
    height: horizontal ? totalCross : totalFlow,
  };
}
