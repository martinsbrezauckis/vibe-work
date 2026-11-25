/**
 * Agentflow diagram drawing: recursive `flow ... end` container layout via layeredLayout, collapsed
 * container folding and edge retargeting, domain shapes (`task`, `tool`, `input`, `decision`,
 * `refdoc`, `action`, `connector`), metadata subtitles, and semantic sequence/reference/failure edges.
 */
import { StepBudget } from "../../limits";
import { layeredLayout, roundedPath, type Point } from "../../layout/layered";
import { fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import {
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
import type { AgentContainer, AgentflowEdgeKind, AgentNode, AgentflowModel } from "./parse";

const PAD = 18;
const LINE_H = 16;

interface BoxSize {
  width: number;
  height: number;
  titleLines: string[];
  subLines: string[];
}

function subtitleOf(meta: Record<string, string>): string[] {
  const out: string[] = [];
  if (meta.model) out.push(`model: ${meta.model}`);
  if (meta.params && meta.returns) out.push(`${meta.params} -> ${meta.returns}`);
  else if (meta.params) out.push(`in: ${meta.params}`);
  else if (meta.returns) out.push(`out: ${meta.returns}`);
  if (meta.connectorRef) out.push(`ref: ${meta.connectorRef}`);
  if (meta.protocol) out.push(meta.endpoint ? `${meta.protocol}: ${meta.endpoint}` : meta.protocol);
  if (meta.value) out.push(`= ${meta.value}`);
  return out.slice(0, 2);
}

function measureNode(node: AgentNode): BoxSize {
  const titleLines = wrapText(node.label, 170, { size: FONT.body, bold: true }, 3);
  const subLines = subtitleOf(node.metadata).map((s) => fitWidth(s, 176, { size: FONT.small }));
  const titleW = measureLines(titleLines, { size: FONT.body, bold: true }).width;
  const subW = subLines.length ? measureLines(subLines, { size: FONT.small }).width : 0;
  const padX =
    node.shape === "decision" || node.shape === "diamond"
      ? 52
      : node.shape === "action" ||
          node.shape === "hexagon" ||
          node.shape === "tool" ||
          node.shape === "subroutine" ||
          node.shape === "input" ||
          node.shape === "lean-right"
        ? 40
        : 28;
  const padY = node.shape === "decision" || node.shape === "diamond" ? 34 : 18;
  const width = Math.max(102, Math.max(titleW, subW) + padX);
  const height = Math.max(44, titleLines.length * LINE_H + subLines.length * 14 + padY);
  return { width, height, titleLines, subLines };
}

function drawNodeShape(node: AgentNode, box: BoxSize, cx: number, cy: number): string {
  const x = cx - box.width / 2;
  const y = cy - box.height / 2;
  const w = box.width;
  const h = box.height;
  const strokeColor = node.isConnector ? color.info : color.nodeStroke;
  const attrs = {
    fill: color.nodeFill,
    stroke: strokeColor,
    "stroke-width": node.isConnector ? 1.5 : 1.3,
    "stroke-dasharray": node.isConnector ? "5 3" : undefined,
  };

  let svg: string;
  switch (node.shape) {
    case "tool":
    case "subroutine":
      svg =
        rect(x, y, w, h, { rx: 4, ...attrs }) +
        line(x + 9, y, x + 9, y + h, { stroke: strokeColor, "stroke-width": 1.1 }) +
        line(x + w - 9, y, x + w - 9, y + h, { stroke: strokeColor, "stroke-width": 1.1 });
      break;
    case "input":
    case "lean-right": {
      const skew = 12;
      svg = polygon(
        [
          [x + skew, y],
          [x + w, y],
          [x + w - skew, y + h],
          [x, y + h],
        ],
        attrs,
      );
      break;
    }
    case "decision":
    case "diamond":
      svg = polygon(
        [
          [cx, y],
          [x + w, cy],
          [cx, y + h],
          [x, cy],
        ],
        attrs,
      );
      break;
    case "refdoc":
    case "lin-doc":
      svg =
        rect(x, y, w, h, { rx: 3, ...attrs }) +
        line(x + 7, y, x + 7, y + h, { stroke: color.accent, "stroke-width": 2.2 });
      break;
    case "action":
    case "hexagon": {
      const d = 12;
      svg = polygon(
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
    default:
      svg = rect(x, y, w, h, { rx: 8, ...attrs });
      break;
  }

  const totalTextH = box.titleLines.length * LINE_H + box.subLines.length * 14;
  const startY = cy - totalTextH / 2 + LINE_H / 2;
  svg += textLines(
    cx,
    startY,
    box.titleLines,
    { size: FONT.body, weight: "600", anchor: "middle" },
    LINE_H / FONT.body,
  );
  if (box.subLines.length > 0) {
    const subY = startY + box.titleLines.length * LINE_H + 2;
    svg += textLines(
      cx,
      subY,
      box.subLines,
      { size: FONT.small, anchor: "middle", fill: color.textMuted },
      14 / FONT.small,
    );
  }
  return svg;
}

function clipBox(cx: number, cy: number, w: number, h: number, tx: number, ty: number): Point {
  const dx = tx - cx;
  const dy = ty - cy;
  if (Math.abs(dx) < 0.01 && Math.abs(dy) < 0.01) return [cx, cy];
  const sx = Math.abs(dx) > 0.01 ? w / 2 / Math.abs(dx) : Number.POSITIVE_INFINITY;
  const sy = Math.abs(dy) > 0.01 ? h / 2 / Math.abs(dy) : Number.POSITIVE_INFINITY;
  const t = Math.min(sx, sy);
  return [cx + dx * t, cy + dy * t];
}

function edgeStyle(kind: AgentflowEdgeKind): {
  stroke: string;
  dashed?: string;
  markerEnd?: string;
} {
  if (kind === "reference") return { stroke: color.textMuted, dashed: "4 4" };
  if (kind === "failure") return { stroke: color.crit, markerEnd: markerRef("arrow-cross") };
  return { stroke: color.line, markerEnd: markerRef("arrow-filled") };
}

/** Draw a parsed agentflow diagram as a complete SVG string. */
export function drawAgentflow(model: AgentflowModel): string {
  const budget = new StepBudget();

  // Map every node and container id to its outermost collapsed ancestor (if any), else itself.
  const collapsedAncestor = (id: string): string => {
    let cur: string | undefined = id;
    const node = model.nodes.get(id);
    if (node) cur = node.containerId;
    let topmostCollapsed: string | undefined;
    let guard = 0;
    while (cur !== undefined && guard < 32) {
      guard++;
      budget.tick();
      const c = model.containers.get(cur);
      if (!c) break;
      if (c.collapsed) topmostCollapsed = c.id;
      cur = c.parentId;
    }
    return topmostCollapsed ?? id;
  };

  // Remap edges through collapsed containers and discard internal edges within a collapsed flow.
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

  const nodeSizes = new Map<string, BoxSize>();
  for (const node of model.nodes.values()) nodeSizes.set(node.id, measureNode(node));
  for (const conn of model.connectors.values()) nodeSizes.set(conn.id, measureNode(conn));

  interface ScopeLayout {
    width: number;
    height: number;
    headerH: number;
    items: Map<string, { x: number; y: number; width: number; height: number }>;
  }

  const scopeLayouts = new Map<string | undefined, ScopeLayout>();

  const countFoldedItems = (contId: string): number => {
    let count = 0;
    for (const n of model.nodes.values()) if (n.containerId === contId) count++;
    for (const c of model.containers.values()) {
      if (c.parentId === contId) count += 1 + countFoldedItems(c.id);
    }
    return count;
  };

  const layoutScope = (scopeId: string | undefined, depth: number): ScopeLayout => {
    const existing = scopeLayouts.get(scopeId);
    if (existing) return existing;

    const childContainers = [...model.containers.values()].filter((c) => c.parentId === scopeId);
    const childNodes = [...model.nodes.values()].filter(
      (n) => n.containerId === scopeId && !n.isConnector,
    );
    const topConnectors = scopeId === undefined ? [...model.connectors.values()] : [];

    const layoutNodes: { id: string; width: number; height: number }[] = [];
    for (const c of childContainers) {
      if (c.collapsed) {
        const sub = `${String(countFoldedItems(c.id))} items (collapsed)`;
        const tw = textWidth(c.label, { size: FONT.body, bold: true });
        const sw = textWidth(sub, { size: FONT.small });
        layoutNodes.push({
          id: c.id,
          width: Math.max(132, Math.max(tw, sw) + 32),
          height: 54,
        });
      } else {
        const sub = layoutScope(c.id, depth + 1);
        layoutNodes.push({ id: c.id, width: sub.width, height: sub.height });
      }
    }
    for (const n of [...childNodes, ...topConnectors]) {
      const sz = nodeSizes.get(n.id);
      if (sz) layoutNodes.push({ id: n.id, width: sz.width, height: sz.height });
    }

    const memberSet = new Set(layoutNodes.map((x) => x.id));
    const liftInScope = (id: string): string | undefined => {
      if (memberSet.has(id)) return id;
      let cur = model.nodes.get(id)?.containerId ?? model.containers.get(id)?.parentId;
      let guard = 0;
      while (cur !== undefined && guard < 32) {
        guard++;
        if (memberSet.has(cur)) return cur;
        cur = model.containers.get(cur)?.parentId;
      }
      return undefined;
    };

    const scopeEdges: {
      from: string;
      to: string;
      labelWidth?: number | undefined;
      labelHeight?: number | undefined;
    }[] = [];
    for (const e of visibleEdges) {
      const a = liftInScope(e.from);
      const b = liftInScope(e.to);
      if (a && b && a !== b) {
        scopeEdges.push({
          from: a,
          to: b,
          labelWidth: e.label ? textWidth(e.label, { size: FONT.small }) + 12 : undefined,
          labelHeight: e.label ? 18 : undefined,
        });
      }
    }

    const cont = scopeId !== undefined ? model.containers.get(scopeId) : undefined;
    const headerH = cont ? (cont.metadata.instruction || cont.metadata.model ? 44 : 30) : 0;
    const titleW = cont
      ? Math.max(
          textWidth(cont.label, { size: FONT.body, bold: true }) + 28,
          cont.metadata.instruction || cont.metadata.model
            ? textWidth(
                fitWidth(cont.metadata.instruction ?? cont.metadata.model ?? "", 220, {
                  size: FONT.small,
                }),
                { size: FONT.small },
              ) + 28
            : 0,
        )
      : 0;

    if (layoutNodes.length === 0) {
      const empty: ScopeLayout = {
        width: Math.max(150, titleW),
        height: headerH + 44,
        headerH,
        items: new Map(),
      };
      scopeLayouts.set(scopeId, empty);
      return empty;
    }

    const laid = layeredLayout(
      layoutNodes,
      scopeEdges,
      { direction: model.direction, margin: 16, nodeGap: 30, rankGap: 48 },
      budget,
    );
    const width = Math.max(laid.width, titleW);
    const offsetX = (width - laid.width) / 2;
    const items = new Map<string, { x: number; y: number; width: number; height: number }>();
    for (const [id, p] of laid.nodes) {
      items.set(id, { x: p.x + offsetX, y: p.y + headerH, width: p.width, height: p.height });
    }
    const res: ScopeLayout = {
      width,
      height: laid.height + headerH,
      headerH,
      items,
    };
    scopeLayouts.set(scopeId, res);
    return res;
  };

  const rootLayout = layoutScope(undefined, 0);

  // Resolve absolute center + box coordinates for every visible container, node, and connector.
  const absPos = new Map<string, { x: number; y: number; width: number; height: number }>();
  const back: string[] = [];
  const mid: string[] = [];
  const front: string[] = [];

  const placeScope = (scopeId: string | undefined, ox: number, oy: number, depth: number): void => {
    const sc = scopeLayouts.get(scopeId);
    if (!sc) return;
    for (const [id, box] of sc.items) {
      const cx = ox + box.x;
      const cy = oy + box.y;
      absPos.set(id, { x: cx, y: cy, width: box.width, height: box.height });
      const cont: AgentContainer | undefined = model.containers.get(id);
      if (cont) {
        const x = cx - box.width / 2;
        const y = cy - box.height / 2;
        const tint = series(depth);
        if (cont.collapsed) {
          front.push(
            rect(x, y, box.width, box.height, {
              rx: 8,
              fill: color.groupHdr,
              stroke: tint,
              "stroke-width": 1.6,
              "stroke-dasharray": "5 3",
            }),
          );
          front.push(
            text(
              cx,
              y + 18,
              fitWidth(cont.label, box.width - 20, { size: FONT.body, bold: true }),
              {
                size: FONT.body,
                weight: "700",
                anchor: "middle",
              },
            ),
          );
          front.push(
            text(cx, y + 36, `${String(countFoldedItems(cont.id))} items (collapsed)`, {
              size: FONT.small,
              anchor: "middle",
              fill: color.textSec,
            }),
          );
        } else {
          back.push(
            rect(x, y, box.width, box.height, {
              rx: 8,
              fill: tint,
              "fill-opacity": 0.06,
              stroke: tint,
              "stroke-width": 1.4,
            }),
          );
          back.push(
            text(
              x + 12,
              y + 16,
              fitWidth(cont.label, box.width - 24, { size: FONT.body, bold: true }),
              {
                size: FONT.body,
                weight: "700",
                fill: color.text,
              },
            ),
          );
          const sub = cont.metadata.model ?? cont.metadata.instruction;
          if (sub) {
            back.push(
              text(x + 12, y + 32, fitWidth(sub, box.width - 24, { size: FONT.small }), {
                size: FONT.small,
                fill: color.textMuted,
              }),
            );
          }
          placeScope(cont.id, x, y, depth + 1);
        }
      }
    }
  };

  placeScope(undefined, PAD, PAD, 0);

  // Draw visible edges between resolved absolute boxes.
  visibleEdges.forEach((edge, ei) => {
    budget.tick();
    const a = absPos.get(edge.from);
    const b = absPos.get(edge.to);
    if (!a || !b) return;
    const p1 = clipBox(a.x, a.y, a.width, a.height, b.x, b.y);
    const p2 = clipBox(b.x, b.y, b.width, b.height, a.x, a.y);
    const pts: Point[] =
      edge.from === edge.to
        ? [
            [a.x + a.width / 2, a.y - 10],
            [a.x + a.width / 2 + 24, a.y - 10],
            [a.x + a.width / 2 + 24, a.y + 10],
            [a.x + a.width / 2, a.y + 10],
          ]
        : [p1, p2];
    const st = edgeStyle(edge.kind);
    mid.push(
      path(roundedPath(pts, 6), {
        fill: "none",
        stroke: st.stroke,
        "stroke-width": 1.4,
        "stroke-dasharray": st.dashed,
        "marker-end": st.markerEnd,
      }),
    );
    if (edge.label) {
      const mx = (p1[0] + p2[0]) / 2;
      const my = (p1[1] + p2[1]) / 2 + ((ei % 3) - 1) * 4;
      const lbl = fitWidth(edge.label, 140, { size: FONT.small });
      const lw = textWidth(lbl, { size: FONT.small }) + 12;
      front.push(
        rect(mx - lw / 2, my - 9, lw, 18, {
          rx: 4,
          fill: color.canvas,
          "fill-opacity": 0.94,
          stroke: color.innerStroke,
          "stroke-width": 0.8,
        }),
      );
      front.push(
        text(mx, my, lbl, {
          size: FONT.small,
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }
  });

  // Draw non-folded nodes and connectors.
  for (const node of [...model.nodes.values(), ...model.connectors.values()]) {
    const p = absPos.get(node.id);
    const sz = nodeSizes.get(node.id);
    if (p && sz) front.push(drawNodeShape(node, sz, p.x, p.y));
  }

  return svgDoc(
    rootLayout.width + PAD * 2,
    rootLayout.height + PAD * 2,
    el("g", {}, [...back, ...mid, ...front].join("")),
    markerDefs(["arrow-filled", "arrow-cross"]),
  );
}
