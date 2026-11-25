/**
 * State diagram drawing. Every region of the diagram (the top level and each concurrent region of
 * a composite) is laid out on its own with the layered layout; composites become one box in the
 * layout of the region that holds them, and are drawn around their own regions. A transition whose
 * end sits inside a composite is attached to that composite's box in the outer region.
 */
import { DiagramInputError } from "../../types";
import { StepBudget } from "../../limits";
import { layeredLayout, roundedPath, type Direction, type Point } from "../../layout/layered";
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
import { FONT, color } from "../../theme";
import type { StateModel, StateNode, StateTransition } from "./parse";

const LINE = 18;
const PAD = 14;
const LABEL = { size: FONT.small + 1 };

interface Laid {
  width: number;
  height: number;
  /** SVG for this region with its top-left corner at (ox, oy). */
  draw(ox: number, oy: number): string;
}
interface Ctx {
  model: StateModel;
  budget: StepBudget;
  byId: Map<string, StateNode>;
  direction: Direction;
}

function simpleSize(
  node: StateNode,
  direction: Direction,
): { width: number; height: number; lines: string[] } {
  const horizontal = direction === "LR" || direction === "RL";
  switch (node.kind) {
    case "start":
      return { width: 20, height: 20, lines: [] };
    case "end":
      return { width: 26, height: 26, lines: [] };
    case "choice":
      return { width: 32, height: 32, lines: [] };
    case "fork":
    case "join":
      return horizontal ? { width: 8, height: 72, lines: [] } : { width: 72, height: 8, lines: [] };
    default: {
      const lines = wrapText(node.label, 200, { size: FONT.body, bold: true }, 3);
      const head = lines.length * LINE + 14;
      const desc = node.description.flatMap((d) => wrapText(d, 220, LABEL, 4));
      const width = Math.max(
        72,
        measureLines(lines, { size: FONT.body, bold: true }).width + 28,
        measureLines(desc, LABEL).width + 28,
      );
      return { width, height: head + (desc.length ? desc.length * 16 + 10 : 0), lines };
    }
  }
}

function drawSimple(
  node: StateNode,
  cx: number,
  cy: number,
  size: { width: number; height: number; lines: string[] },
): string {
  switch (node.kind) {
    case "start":
      return circle(cx, cy, 9, { fill: color.text });
    case "end":
      return (
        circle(cx, cy, 11, { fill: "none", stroke: color.text, "stroke-width": 2 }) +
        circle(cx, cy, 6, { fill: color.text })
      );
    case "choice":
      return polygon(
        [
          [cx, cy - 16],
          [cx + 16, cy],
          [cx, cy + 16],
          [cx - 16, cy],
        ],
        { fill: color.nodeFill, stroke: color.nodeStroke, "stroke-width": 1.4 },
      );
    case "fork":
    case "join":
      return rect(cx - size.width / 2, cy - size.height / 2, size.width, size.height, {
        rx: 2,
        fill: color.text,
      });
    default: {
      const top = cy - size.height / 2;
      const head = size.lines.length * LINE + 14;
      let out = rect(cx - size.width / 2, top, size.width, size.height, {
        rx: 10,
        fill: color.nodeFill,
        stroke: color.nodeStroke,
        "stroke-width": 1.2,
      });
      out += textLines(
        cx,
        top + 7 + LINE / 2,
        size.lines,
        { size: FONT.body, weight: "600", anchor: "middle" },
        LINE / FONT.body,
      );
      if (node.description.length) {
        out += line(cx - size.width / 2, top + head, cx + size.width / 2, top + head, {
          stroke: color.nodeStroke,
          "stroke-width": 1,
        });
        const desc = node.description.flatMap((d) => wrapText(d, 220, LABEL, 4));
        out += textLines(
          cx,
          top + head + 14,
          desc,
          { size: FONT.small + 1, anchor: "middle", fill: color.textSec },
          16 / (FONT.small + 1),
        );
      }
      return out;
    }
  }
}

/** The child of `scope` that holds `id` (or `id` itself when it already is one). */
function lift(ctx: Ctx, id: string, scope: string | undefined): string {
  let current = id;
  for (let guard = 0; guard < 64; guard++) {
    ctx.budget.tick();
    const parent = ctx.byId.get(current)?.parent;
    if (parent === scope) return current;
    if (parent === undefined) break;
    current = parent;
  }
  throw new DiagramInputError("Transition leaves its composite state");
}

function layoutRegion(ctx: Ctx, ids: string[], scope: string | undefined, margin: number): Laid {
  const sizes = new Map<
    string,
    { width: number; height: number; lines: string[]; composite?: Laid }
  >();
  for (const id of ids) {
    const node = ctx.byId.get(id) as StateNode;
    sizes.set(id, node.regions.length > 0 ? composite(ctx, node) : simpleSize(node, ctx.direction));
  }
  const inRegion = (t: StateTransition): boolean => t.scope === scope;
  const edges: {
    from: string;
    to: string;
    label: string;
    labelWidth?: number | undefined;
    labelHeight?: number | undefined;
  }[] = [];
  for (const t of ctx.model.transitions.filter(inRegion)) {
    const from = lift(ctx, t.from, scope);
    const to = lift(ctx, t.to, scope);
    if (!ids.includes(from)) continue;
    edges.push({
      from,
      to,
      label: t.label,
      labelWidth: t.label ? textWidth(t.label, LABEL) + 12 : undefined,
      labelHeight: t.label ? 18 : undefined,
    });
  }
  const noteList = ctx.model.notes.filter((note) => ids.includes(note.target));
  const nodes = ids.map((id) => ({
    id,
    width: sizes.get(id)?.width ?? 0,
    height: sizes.get(id)?.height ?? 0,
  }));
  const noteLines = noteList.map((note) => wrapText(note.text, 200, LABEL));
  noteList.forEach((note, i) => {
    const size = measureLines(noteLines[i] ?? [], LABEL);
    nodes.push({ id: `\u0000note${String(i)}`, width: size.width + 20, height: size.height + 14 });
    edges.push({ from: `\u0000note${String(i)}`, to: note.target, label: "" });
  });
  const layout = layeredLayout(nodes, edges, { direction: ctx.direction, margin }, ctx.budget);
  return {
    width: layout.width,
    height: layout.height,
    draw(ox, oy) {
      let out = "";
      const noteEdgeStart = edges.length - noteList.length;
      edges.forEach((edge, i) => {
        const placed = layout.edges[i];
        if (!placed) return;
        const isNote = i >= noteEdgeStart;
        let pts: Point[] = placed.points;
        if (placed.selfLoop) {
          const p = layout.nodes.get(edge.from);
          if (!p) return;
          const x = p.x + p.width / 2;
          pts = [
            [x, p.y - 10],
            [x + 26, p.y - 10],
            [x + 26, p.y + 10],
            [x, p.y + 10],
          ];
          placed.label = edge.label
            ? { x: x + 34 + textWidth(edge.label, LABEL) / 2, y: p.y, width: 0, height: 18 }
            : undefined;
        }
        const shifted = pts.map(([x, y]): Point => [x + ox, y + oy]);
        out += path(roundedPath(shifted, 8), {
          fill: "none",
          stroke: isNote ? color.textFaint : color.line,
          "stroke-width": isNote ? 1 : 1.4,
          "stroke-dasharray": isNote ? "3 3" : undefined,
          "marker-end": isNote ? undefined : markerRef("arrow-filled"),
        });
        if (placed.label && edge.label) {
          const w = textWidth(edge.label, LABEL) + 12;
          out += rect(placed.label.x + ox - w / 2, placed.label.y + oy - 9, w, 18, {
            rx: 4,
            fill: color.canvas,
            "fill-opacity": 0.92,
          });
          out += text(placed.label.x + ox, placed.label.y + oy, edge.label, {
            size: FONT.small + 1,
            anchor: "middle",
            fill: color.textSec,
          });
        }
      });
      for (const id of ids) {
        const p = layout.nodes.get(id);
        const size = sizes.get(id);
        if (!p || !size) continue;
        out += size.composite
          ? size.composite.draw(p.x + ox - size.width / 2, p.y + oy - size.height / 2)
          : drawSimple(ctx.byId.get(id) as StateNode, p.x + ox, p.y + oy, size);
      }
      noteList.forEach((_, i) => {
        const p = layout.nodes.get(`\u0000note${String(i)}`);
        if (!p) return;
        out += rect(p.x + ox - p.width / 2, p.y + oy - p.height / 2, p.width, p.height, {
          rx: 3,
          fill: color.groupHdr,
          stroke: color.nodeStroke,
          "stroke-width": 1,
        });
        const lines = noteLines[i] ?? [];
        out += textLines(
          p.x + ox - p.width / 2 + 10,
          p.y + oy - ((lines.length - 1) * (FONT.small + 1) * 1.3) / 2,
          lines,
          LABEL,
        );
      });
      return out;
    },
  };
}

function composite(
  ctx: Ctx,
  node: StateNode,
): { width: number; height: number; lines: string[]; composite: Laid } {
  const regions = node.regions.map((ids) => layoutRegion(ctx, ids, node.id, 10));
  const title = fitWidth(node.label, 360, { size: FONT.body, bold: true });
  const inner = Math.max(0, ...regions.map((r) => r.width));
  const width = Math.max(inner, textWidth(title, { size: FONT.body, bold: true }) + 8) + PAD * 2;
  const head = 32;
  const height =
    head + regions.reduce((sum, r) => sum + r.height, 0) + PAD + (regions.length - 1) * 10;
  const laid: Laid = {
    width,
    height,
    draw(ox, oy) {
      let out = rect(ox, oy, width, height, {
        rx: 12,
        fill: color.groupFill,
        stroke: color.nodeStroke,
        "stroke-width": 1.2,
      });
      out += text(ox + width / 2, oy + 16, title, {
        size: FONT.body,
        weight: "600",
        anchor: "middle",
      });
      out += line(ox, oy + head - 2, ox + width, oy + head - 2, {
        stroke: color.innerStroke,
        "stroke-width": 1,
      });
      let y = oy + head;
      regions.forEach((region, i) => {
        if (i > 0)
          out += line(ox + 8, y - 5, ox + width - 8, y - 5, {
            stroke: color.textFaint,
            "stroke-width": 1,
            "stroke-dasharray": "6 4",
          });
        out += region.draw(ox + (width - region.width) / 2, y);
        y += region.height + 10;
      });
      return out;
    },
  };
  return { width, height, lines: [], composite: laid };
}

/** Draw a parsed state diagram as a complete SVG string. */
export function drawState(model: StateModel): string {
  const ctx: Ctx = {
    model,
    budget: new StepBudget(),
    byId: new Map(model.states.map((s) => [s.id, s])),
    direction: model.direction,
  };
  const topIds = model.states.filter((s) => s.parent === undefined).map((s) => s.id);
  const laid = layoutRegion(ctx, topIds, undefined, 16);
  return svgDoc(
    laid.width,
    laid.height,
    el("g", {}, laid.draw(0, 0)),
    markerDefs(["arrow-filled"]),
  );
}
