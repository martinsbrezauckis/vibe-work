/**
 * Class diagram drawing: UML boxes with name, attribute and method compartments, the layered
 * layout, relationship ends (triangle, diamonds, arrows), cardinalities, labels, namespace frames
 * and notes. A namespace frame is the bounding box of its members; a non-member that the layout
 * happens to place inside that box is drawn over the frame, never hidden by it.
 */
import { StepBudget } from "../../limits";
import { fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import {
  el,
  line,
  markerDefs,
  markerRef,
  n,
  path,
  rect,
  svgDoc,
  text,
  textLines,
  type MarkerName,
} from "../../svg";
import { FONT, color } from "../../theme";
import { layeredLayout, roundedPath, type Point } from "../../layout/layered";
import type { ClassMember, ClassModel, ClassNode, Head } from "./parse";

const ROW = 18;
const PAD = 12;
const NOTE_WRAP = 200;
const HEAD_MARKER: Record<Head, MarkerName> = {
  inherit: "triangle-hollow",
  compose: "diamond-filled",
  aggregate: "diamond-hollow",
  arrow: "arrow-open",
};

interface Box {
  width: number;
  height: number;
  title: string[];
  stereotype: string;
  attributes: ClassMember[];
  methods: ClassMember[];
}

function memberText(m: ClassMember): string {
  return `${m.visibility}${m.visibility ? " " : ""}${m.text}`;
}

function boxOf(node: ClassNode): Box {
  const name = node.generic ? `${node.label}<${node.generic}>` : node.label;
  const title = wrapText(name, 260, { size: FONT.body, bold: true }, 3);
  const attributes = node.members.filter((m) => !m.method);
  const methods = node.members.filter((m) => m.method);
  const stereotype = node.stereotype ? `«${node.stereotype}»` : "";
  let width = 0;
  for (const m of node.members)
    width = Math.max(width, textWidth(memberText(m), { size: FONT.small + 1 }));
  width = Math.max(width, measureLines(title, { size: FONT.body, bold: true }).width);
  width = Math.max(width, stereotype ? textWidth(stereotype, { size: FONT.small }) : 0);
  const head = title.length * ROW + (stereotype ? 14 : 0) + 10;
  const rows = (list: ClassMember[]): number => Math.max(list.length * ROW, 8) + 8;
  return {
    width: Math.min(Math.max(width + PAD * 2, 96), 420),
    height: head + rows(attributes) + rows(methods),
    title,
    stereotype,
    attributes,
    methods,
  };
}

function drawBox(box: Box, cx: number, cy: number): string {
  const x = cx - box.width / 2;
  const top = cy - box.height / 2;
  let out = rect(x, top, box.width, box.height, {
    rx: 4,
    fill: color.nodeFill,
    stroke: color.nodeStroke,
    "stroke-width": 1.2,
  });
  const head = box.title.length * ROW + (box.stereotype ? 14 : 0) + 10;
  out += rect(x, top, box.width, head, { rx: 4, fill: color.groupHdr, stroke: "none" });
  out += rect(x, top + head - 6, box.width, 6, { fill: color.groupHdr });
  out += rect(x, top, box.width, box.height, {
    rx: 4,
    fill: "none",
    stroke: color.nodeStroke,
    "stroke-width": 1.2,
  });
  let y = top + 5;
  if (box.stereotype) {
    out += text(cx, y + 7, box.stereotype, {
      size: FONT.small,
      anchor: "middle",
      fill: color.textSec,
    });
    y += 14;
  }
  out += textLines(
    cx,
    y + ROW / 2,
    box.title,
    { size: FONT.body, weight: "600", anchor: "middle" },
    ROW / FONT.body,
  );
  y = top + head;
  const compartment = (list: ClassMember[]): number => {
    out += line(x, y, x + box.width, y, { stroke: color.nodeStroke, "stroke-width": 1 });
    list.forEach((m, i) => {
      out += text(
        x + PAD,
        y + 4 + i * ROW + ROW / 2,
        fitWidth(memberText(m), box.width - PAD * 2, { size: FONT.small + 1 }),
        { size: FONT.small + 1, italic: m.abstract, fill: m.static ? color.accent : color.text },
      );
    });
    return Math.max(list.length * ROW, 8) + 8;
  };
  y += compartment(box.attributes);
  compartment(box.methods);
  return out;
}

/** Point `distance` along the route from its start (or end when `fromEnd`), and the unit normal there. */
function along(
  points: readonly Point[],
  distance: number,
  fromEnd: boolean,
): { x: number; y: number; nx: number; ny: number } {
  const list = fromEnd ? [...points].reverse() : [...points];
  let left = distance;
  for (let i = 1; i < list.length; i++) {
    const a = list[i - 1] as Point;
    const b = list[i] as Point;
    const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
    if (len >= left || i === list.length - 1) {
      const t = len === 0 ? 0 : Math.min(left, len) / len;
      const ux = len === 0 ? 0 : (b[0] - a[0]) / len;
      const uy = len === 0 ? 1 : (b[1] - a[1]) / len;
      return { x: a[0] + (b[0] - a[0]) * t, y: a[1] + (b[1] - a[1]) * t, nx: -uy, ny: ux };
    }
    left -= len;
  }
  const only = list[0] ?? [0, 0];
  return { x: only[0], y: only[1], nx: 0, ny: 1 };
}

/** Draw a parsed class diagram as a complete SVG string. */
export function drawClass(model: ClassModel): string {
  const budget = new StepBudget();
  const boxes = new Map(model.classes.map((c) => [c.id, boxOf(c)]));
  const nodes = model.classes.map((c) => ({
    id: c.id,
    width: boxes.get(c.id)?.width ?? 0,
    height: boxes.get(c.id)?.height ?? 0,
  }));
  const noteLines = model.notes.map((note) =>
    wrapText(note.text, NOTE_WRAP, { size: FONT.small + 1 }),
  );
  model.notes.forEach((_, i) => {
    const size = measureLines(noteLines[i] ?? [], { size: FONT.small + 1 });
    nodes.push({ id: `\u0000note${String(i)}`, width: size.width + 20, height: size.height + 14 });
  });
  const labelStyle = { size: FONT.small + 1 };
  const edges = model.relations.map((r) => {
    const w = r.label ? textWidth(r.label, labelStyle) + 12 : 0;
    return {
      from: r.from,
      to: r.to,
      labelWidth: w || undefined,
      labelHeight: r.label ? 18 : undefined,
    };
  });
  model.notes.forEach((note, i) => {
    if (note.target)
      edges.push({
        from: `\u0000note${String(i)}`,
        to: note.target,
        labelWidth: undefined,
        labelHeight: undefined,
      });
  });
  const layout = layeredLayout(nodes, edges, { direction: model.direction }, budget);
  let minX = 0;
  let maxX = layout.width;
  let minY = 0;
  let maxY = layout.height;
  const back: string[] = [];
  const mid: string[] = [];
  const front: string[] = [];

  // Namespace frames behind everything.
  for (const space of model.namespaces) {
    const placed = space.classes.map((id) => layout.nodes.get(id)).filter((p) => p !== undefined);
    if (placed.length === 0) continue;
    const x1 = Math.min(...placed.map((p) => p.x - p.width / 2)) - 18;
    const x2 = Math.max(...placed.map((p) => p.x + p.width / 2)) + 18;
    const y1 = Math.min(...placed.map((p) => p.y - p.height / 2)) - 34;
    const y2 = Math.max(...placed.map((p) => p.y + p.height / 2)) + 18;
    back.push(
      rect(x1, y1, x2 - x1, y2 - y1, {
        rx: 8,
        fill: color.groupFill,
        stroke: color.innerStroke,
        "stroke-width": 1.2,
        "stroke-dasharray": "6 4",
      }),
    );
    back.push(
      text(x1 + 12, y1 + 16, fitWidth(space.name, x2 - x1 - 24, { size: FONT.body, bold: true }), {
        size: FONT.body,
        weight: "600",
        fill: color.textSec,
      }),
    );
    minX = Math.min(minX, x1 - 8);
    maxX = Math.max(maxX, x2 + 8);
    minY = Math.min(minY, y1 - 8);
    maxY = Math.max(maxY, y2 + 8);
  }

  model.relations.forEach((rel, i) => {
    const edge = layout.edges[i];
    if (!edge) return;
    const stroke = {
      stroke: color.line,
      "stroke-width": 1.3,
      "stroke-dasharray": rel.dashed ? "6 4" : undefined,
      fill: "none",
    };
    const markers = {
      "marker-start": rel.fromHead ? markerRef(HEAD_MARKER[rel.fromHead]) : undefined,
      "marker-end": rel.toHead ? markerRef(HEAD_MARKER[rel.toHead]) : undefined,
    };
    let pts: Point[] = edge.points;
    if (edge.selfLoop) {
      const p = layout.nodes.get(rel.from);
      if (!p) return;
      const x = p.x + p.width / 2;
      pts = [
        [x, p.y - 12],
        [x + 28, p.y - 12],
        [x + 28, p.y + 12],
        [x, p.y + 12],
      ];
      edge.label = rel.label
        ? { x: x + 36 + textWidth(rel.label, labelStyle) / 2, y: p.y, width: 0, height: 18 }
        : undefined;
      maxX = Math.max(maxX, x + 40 + textWidth(rel.label, labelStyle));
    }
    mid.push(path(roundedPath(pts, edge.selfLoop ? 6 : 8), { ...stroke, ...markers }));
    if (edge.label && rel.label) {
      const w = textWidth(rel.label, labelStyle) + 12;
      front.push(
        rect(edge.label.x - w / 2, edge.label.y - 9, w, 18, {
          rx: 4,
          fill: color.canvas,
          "fill-opacity": 0.92,
        }),
      );
      front.push(
        text(edge.label.x, edge.label.y, rel.label, {
          size: FONT.small + 1,
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }
    for (const [card, fromEnd] of [
      [rel.fromCard, false],
      [rel.toCard, true],
    ] as const) {
      if (!card) continue;
      const spot = along(pts, 26, fromEnd);
      front.push(
        text(
          spot.x + spot.nx * 12,
          spot.y + spot.ny * 12,
          fitWidth(card, 60, { size: FONT.small }),
          { size: FONT.small, anchor: "middle", fill: color.textSec },
        ),
      );
    }
  });

  model.classes.forEach((node) => {
    const p = layout.nodes.get(node.id);
    const box = boxes.get(node.id);
    if (p && box) front.push(drawBox(box, p.x, p.y));
  });
  model.notes.forEach((note, i) => {
    const p = layout.nodes.get(`\u0000note${String(i)}`);
    if (!p) return;
    const lines = noteLines[i] ?? [];
    front.push(
      rect(p.x - p.width / 2, p.y - p.height / 2, p.width, p.height, {
        rx: 3,
        fill: color.groupHdr,
        stroke: color.nodeStroke,
        "stroke-width": 1,
      }),
    );
    front.push(
      textLines(
        p.x - p.width / 2 + 10,
        p.y - ((lines.length - 1) * (FONT.small + 1) * 1.3) / 2,
        lines,
        { size: FONT.small + 1 },
      ),
    );
    if (note.target) {
      const k = model.relations.length + model.notes.slice(0, i).filter((x) => x.target).length;
      const edge = layout.edges[k];
      if (edge)
        back.push(
          path(roundedPath(edge.points, 6), {
            stroke: color.textFaint,
            "stroke-width": 1,
            "stroke-dasharray": "3 3",
            fill: "none",
          }),
        );
    }
  });
  const defs = markerDefs(["triangle-hollow", "diamond-filled", "diamond-hollow", "arrow-open"]);
  const body = el(
    "g",
    { transform: `translate(${n(-minX)},${n(-minY)})` },
    [...back, ...mid, ...front].join(""),
  );
  return svgDoc(maxX - minX, maxY - minY, body, defs);
}
