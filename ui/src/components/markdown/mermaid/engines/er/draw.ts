/**
 * ER diagram drawing: entity tables (type, name, key badges, comment) in the layered layout, solid
 * (identifying) or dashed (non-identifying) connectors with crow's-foot, bar and ring ends drawn as
 * plain geometry, and label pills.
 */
import { StepBudget } from "../../limits";
import { fitWidth, textWidth, wrapText } from "../../measure";
import { circle, el, line, n, path, rect, svgDoc, text } from "../../svg";
import { FONT, color } from "../../theme";
import { layeredLayout, roundedPath, type Point } from "../../layout/layered";
import type { Cardinality, ErEntity, ErModel } from "./parse";

const ROW = 24;
const HEAD = 34;
const CELL = 10;
const SMALL = { size: FONT.small + 1 };

interface Table {
  width: number;
  height: number;
  columns: number[];
  hasComment: boolean;
  title: string;
}

function tableOf(entity: ErEntity): Table {
  const title = wrapText(entity.label, 280, { size: FONT.body, bold: true }, 2).join(" ");
  const attrs = entity.attributes;
  const typeW = Math.max(0, ...attrs.map((a) => textWidth(a.type, SMALL)));
  const nameW = Math.max(0, ...attrs.map((a) => textWidth(a.name, SMALL)));
  const keyW = Math.max(
    0,
    ...attrs.map((a) =>
      a.keys.length ? textWidth(a.keys.join(","), { size: FONT.small, bold: true }) + 12 : 0,
    ),
  );
  const hasComment = attrs.some((a) => a.comment !== "");
  const commentW = Math.min(
    220,
    Math.max(0, ...attrs.map((a) => textWidth(a.comment, { size: FONT.small }))),
  );
  const columns = [typeW, nameW, keyW, hasComment ? commentW : 0];
  const gaps = Math.max(0, columns.filter((w) => w > 0).length - 1) * CELL;
  const body = columns.reduce((a, b) => a + b, 0) + gaps + CELL * 2;
  const width = Math.max(110, textWidth(title, { size: FONT.body, bold: true }) + 28, body);
  return { width, height: HEAD + attrs.length * ROW, columns, hasComment, title };
}

function drawTable(entity: ErEntity, table: Table, cx: number, cy: number): string {
  const x = cx - table.width / 2;
  const top = cy - table.height / 2;
  let out = rect(x, top, table.width, table.height, {
    rx: 4,
    fill: color.nodeFill,
    stroke: color.nodeStroke,
    "stroke-width": 1.2,
  });
  out += rect(x, top, table.width, HEAD, {
    rx: 4,
    fill: color.groupHdr,
    stroke: color.nodeStroke,
    "stroke-width": 1.2,
  });
  out += rect(x + 0.6, top + HEAD - 6, table.width - 1.2, 6, { fill: color.groupHdr });
  out += text(cx, top + HEAD / 2, table.title, {
    size: FONT.body,
    weight: "600",
    anchor: "middle",
  });
  const [typeW = 0, nameW = 0, keyW = 0, commentW = 0] = table.columns;
  entity.attributes.forEach((a, i) => {
    const y = top + HEAD + i * ROW;
    if (i % 2 === 1)
      out += rect(x + 1, y, table.width - 2, ROW, { fill: color.groupFill, "fill-opacity": 0.7 });
    const mid = y + ROW / 2;
    let cursor = x + CELL;
    out += text(cursor, mid, fitWidth(a.type, typeW + 2, SMALL), {
      size: FONT.small + 1,
      fill: color.textSec,
    });
    cursor += typeW + CELL;
    out += text(cursor, mid, fitWidth(a.name, nameW + 2, SMALL), { size: FONT.small + 1 });
    cursor += nameW + CELL;
    if (keyW > 0) {
      if (a.keys.length) {
        out += rect(cursor, mid - 8, keyW, 16, {
          rx: 4,
          fill: color.keyBadge,
          stroke: color.nodeStroke,
          "stroke-width": 0.8,
        });
        out += text(cursor + keyW / 2, mid, a.keys.join(","), {
          size: FONT.small,
          weight: "700",
          anchor: "middle",
        });
      }
      cursor += keyW + CELL;
    }
    if (table.hasComment && commentW > 0 && a.comment) {
      out += text(cursor, mid, fitWidth(a.comment, commentW + 2, { size: FONT.small }), {
        size: FONT.small,
        italic: true,
        fill: color.textMuted,
      });
    }
  });
  return out;
}

/** Crow's-foot, bar and ring glyphs at `p`, where `d` is the unit vector pointing away from the entity. */
function endGlyph(p: Point, d: Point, card: Cardinality): string {
  const stroke = { stroke: color.line, "stroke-width": 1.4, fill: "none" };
  const perp: Point = [-d[1], d[0]];
  const at = (dist: number, side = 0): Point => [
    p[0] + d[0] * dist + perp[0] * side,
    p[1] + d[1] * dist + perp[1] * side,
  ];
  const bar = (dist: number): string => {
    const a = at(dist, -7);
    const b = at(dist, 7);
    return line(a[0], a[1], b[0], b[1], stroke);
  };
  const feet = (): string => {
    const apex = at(14);
    const l = at(0, -8);
    const r = at(0, 8);
    return (
      path(`M${n(l[0])},${n(l[1])} L${n(apex[0])},${n(apex[1])} L${n(r[0])},${n(r[1])}`, stroke) +
      line(p[0], p[1], apex[0], apex[1], stroke)
    );
  };
  const ring = (dist: number): string =>
    circle(at(dist)[0], at(dist)[1], 5, {
      fill: color.canvas,
      stroke: color.line,
      "stroke-width": 1.4,
    });
  switch (card) {
    case "one":
      return bar(8) + bar(13);
    case "zero-one":
      return bar(8) + ring(16);
    case "one-many":
      return feet() + bar(19);
    default:
      return feet() + ring(24);
  }
}

function unit(a: Point, b: Point): Point {
  const len = Math.hypot(b[0] - a[0], b[1] - a[1]) || 1;
  return [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
}

/** Draw a parsed ER diagram as a complete SVG string. */
export function drawEr(model: ErModel): string {
  const tables = new Map(model.entities.map((e) => [e.id, tableOf(e)]));
  const nodes = model.entities.map((e) => ({
    id: e.id,
    width: tables.get(e.id)?.width ?? 0,
    height: tables.get(e.id)?.height ?? 0,
  }));
  const edges = model.relations.map((r) => ({
    from: r.from,
    to: r.to,
    labelWidth: r.label ? textWidth(r.label, SMALL) + 14 : undefined,
    labelHeight: r.label ? 20 : undefined,
  }));
  const layout = layeredLayout(
    nodes,
    edges,
    { direction: model.direction, rankGap: 80, nodeGap: 44 },
    new StepBudget(),
  );
  let maxX = layout.width;
  let out = "";
  const labels: string[] = [];
  model.relations.forEach((rel, i) => {
    const edge = layout.edges[i];
    if (!edge) return;
    let pts: Point[] = edge.points;
    if (edge.selfLoop) {
      const p = layout.nodes.get(rel.from);
      if (!p) return;
      const x = p.x + p.width / 2;
      pts = [
        [x, p.y - 14],
        [x + 40, p.y - 14],
        [x + 40, p.y + 14],
        [x, p.y + 14],
      ];
      edge.label = rel.label
        ? { x: x + 48 + textWidth(rel.label, SMALL) / 2, y: p.y, width: 0, height: 20 }
        : undefined;
      maxX = Math.max(maxX, x + 56 + textWidth(rel.label, SMALL));
    }
    const first = pts[0];
    const second = pts[1];
    const last = pts[pts.length - 1];
    const prev = pts[pts.length - 2];
    if (!first || !second || !last || !prev) return;
    out += path(roundedPath(pts, 8), {
      fill: "none",
      stroke: color.line,
      "stroke-width": 1.4,
      "stroke-dasharray": rel.identifying ? undefined : "6 4",
    });
    out += endGlyph(first, unit(first, second), rel.fromCard);
    out += endGlyph(last, unit(last, prev), rel.toCard);
    if (edge.label && rel.label) {
      const w = textWidth(rel.label, SMALL) + 14;
      labels.push(
        rect(edge.label.x - w / 2, edge.label.y - 10, w, 20, {
          rx: 10,
          fill: color.canvas,
          stroke: color.innerStroke,
          "stroke-width": 0.8,
        }),
      );
      labels.push(
        text(edge.label.x, edge.label.y, rel.label, {
          size: FONT.small + 1,
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }
  });
  for (const entity of model.entities) {
    const p = layout.nodes.get(entity.id);
    const table = tables.get(entity.id);
    if (p && table) out += drawTable(entity, table, p.x, p.y);
  }
  return svgDoc(maxX, layout.height, el("g", {}, out + labels.join("")));
}
