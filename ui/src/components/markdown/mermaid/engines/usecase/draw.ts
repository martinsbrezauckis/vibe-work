/**
 * Use case diagram drawing: lays out each system boundary internally, lays out boundaries plus
 * top-level actors, ellipse/rect use cases, JSON tables, and notes via layeredLayout, and draws
 * actor variants, business slashes, stereotypes, package tabs, and relationship markers.
 */
import { StepBudget } from "../../limits";
import { layeredLayout, roundedPath, type Point } from "../../layout/layered";
import { fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import {
  circle,
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
} from "../../svg";
import { FONT, color, series } from "../../theme";
import type { RelMarker, UsecaseBoundary, UsecaseElement, UsecaseModel } from "./parse";

const PAD = 16;
const LINE_H = 16;

interface SizedElem {
  elem: UsecaseElement;
  width: number;
  height: number;
  lines: string[];
  colW?: number;
}

function measureElement(elem: UsecaseElement): SizedElem {
  if (elem.kind === "actor") {
    const lines = wrapText(elem.label, 140, { size: FONT.body, bold: true }, 3);
    const labelW = measureLines(lines, { size: FONT.body, bold: true }).width;
    const stereoW = elem.stereotype ? textWidth(`«${elem.stereotype}»`, { size: FONT.small }) : 0;
    const iconH = elem.variant === "icon" ? 36 : 48;
    const stereoH = elem.stereotype ? 15 : 0;
    return {
      elem,
      lines,
      width: Math.max(68, labelW + 16, stereoW + 12),
      height: iconH + stereoH + lines.length * LINE_H + 10,
    };
  }
  if (elem.kind === "usecase") {
    const style = { size: FONT.small + 1, bold: true };
    const lines = wrapText(elem.label, 170, style, 4);
    const m = measureLines(lines, style, 1.3);
    const stereoW = elem.stereotype ? textWidth(`«${elem.stereotype}»`, { size: FONT.small }) : 0;
    const stereoH = elem.stereotype ? 14 : 0;
    const padX = elem.shape === "ellipse" ? 44 : 26;
    const padY = elem.shape === "ellipse" ? 26 : 18;
    return {
      elem,
      lines,
      width: Math.max(106, Math.max(m.width, stereoW) + padX),
      height: Math.max(46, lines.length * LINE_H + stereoH + padY),
    };
  }
  const small = { size: FONT.small };
  const keyW = Math.max(40, ...elem.rows.map((r) => textWidth(r.key, small)));
  const valW = Math.min(180, Math.max(40, ...elem.rows.map((r) => textWidth(r.value, small))));
  const titleW = textWidth(elem.id, { size: FONT.body, bold: true }) + 24;
  return {
    elem,
    lines: [],
    colW: keyW + 10,
    width: Math.max(140, titleW, keyW + valW + 46),
    height: 30 + Math.max(1, elem.rows.length) * 20 + 6,
  };
}

function drawElement(s: SizedElem, cx: number, cy: number): string {
  const elem = s.elem;
  const x = cx - s.width / 2;
  const top = cy - s.height / 2;

  if (elem.kind === "actor") {
    let out = "";
    let yCursor = top + 4;
    if (elem.stereotype) {
      out += text(cx, yCursor + 6, `«${elem.stereotype}»`, {
        size: FONT.small,
        anchor: "middle",
        fill: color.textSec,
      });
      yCursor += 15;
    }
    const stroke = { stroke: color.nodeStroke, "stroke-width": 1.5, fill: "none" };
    if (elem.variant === "icon") {
      const iconText = fitWidth(elem.icon ?? "icon", 64, { size: FONT.small });
      const bw = Math.max(48, textWidth(iconText, { size: FONT.small }) + 16);
      out += rect(cx - bw / 2, yCursor + 2, bw, 26, {
        rx: 6,
        fill: color.nodeFill,
        stroke: color.nodeStroke,
        "stroke-width": 1.3,
      });
      out += text(cx, yCursor + 15, iconText, {
        size: FONT.small,
        anchor: "middle",
        fill: color.textSec,
      });
      yCursor += 34;
    } else if (elem.variant === "awesome") {
      out += circle(cx, yCursor + 10, 8, { fill: color.nodeStroke });
      out += path(
        `M${n(cx - 14)},${n(yCursor + 40)} C${n(cx - 14)},${n(yCursor + 24)} ${n(cx + 14)},${n(yCursor + 24)} ${n(cx + 14)},${n(yCursor + 40)} Z`,
        { fill: color.nodeStroke },
      );
      yCursor += 46;
    } else {
      const fillHead = elem.variant === "hollow" ? color.canvas : color.nodeFill;
      out += circle(cx, yCursor + 9, 7.5, {
        fill: fillHead,
        stroke: color.nodeStroke,
        "stroke-width": elem.variant === "hollow" ? 2 : 1.5,
      });
      out += line(cx, yCursor + 16.5, cx, yCursor + 31, stroke);
      out += line(cx - 11, yCursor + 22, cx + 11, yCursor + 22, stroke);
      out += line(cx, yCursor + 31, cx - 9, yCursor + 43, stroke);
      out += line(cx, yCursor + 31, cx + 9, yCursor + 43, stroke);
      if (elem.business) {
        out += line(cx + 4, yCursor + 3, cx + 12, yCursor + 13, {
          stroke: color.accent,
          "stroke-width": 1.6,
        });
      }
      yCursor += 48;
    }
    out += textLines(
      cx,
      yCursor + 4,
      s.lines,
      { size: FONT.body, weight: "600", anchor: "middle" },
      LINE_H / FONT.body,
    );
    return out;
  }

  if (elem.kind === "usecase") {
    let out = "";
    if (elem.shape === "ellipse") {
      out += el("ellipse", {
        cx,
        cy,
        rx: s.width / 2,
        ry: s.height / 2,
        fill: color.nodeFill,
        stroke: color.nodeStroke,
        "stroke-width": 1.3,
      });
      if (elem.business) {
        out += line(
          cx + s.width * 0.22,
          cy - s.height * 0.42,
          cx + s.width * 0.42,
          cy - s.height * 0.14,
          {
            stroke: color.accent,
            "stroke-width": 1.6,
          },
        );
      }
    } else {
      out += rect(x, top, s.width, s.height, {
        rx: 6,
        fill: color.nodeFill,
        stroke: color.nodeStroke,
        "stroke-width": 1.3,
      });
    }
    let textY = cy - ((s.lines.length - 1) * LINE_H) / 2;
    if (elem.stereotype) {
      out += text(cx, top + 12, `«${elem.stereotype}»`, {
        size: FONT.small,
        anchor: "middle",
        fill: color.textSec,
      });
      textY += 7;
    }
    out += textLines(
      cx,
      textY,
      s.lines,
      { size: FONT.small + 1, weight: "600", anchor: "middle" },
      LINE_H / (FONT.small + 1),
    );
    return out;
  }

  // JSON table card.
  let out = rect(x, top, s.width, s.height, {
    rx: 5,
    fill: color.nodeFill,
    stroke: color.nodeStroke,
    "stroke-width": 1.2,
  });
  out += rect(x, top, s.width, 28, {
    rx: 5,
    fill: color.groupHdr,
    stroke: color.nodeStroke,
    "stroke-width": 1.2,
  });
  out += text(cx, top + 14, fitWidth(elem.id, s.width - 16, { size: FONT.body, bold: true }), {
    size: FONT.body,
    weight: "700",
    anchor: "middle",
  });
  const colW = s.colW ?? 60;
  elem.rows.forEach((row, idx) => {
    const ry = top + 28 + idx * 20;
    if (idx % 2 === 1) {
      out += rect(x + 1, ry, s.width - 2, 20, { fill: color.groupFill, "fill-opacity": 0.7 });
    }
    out += text(x + 10, ry + 10, fitWidth(row.key, colW - 4, { size: FONT.small }), {
      size: FONT.small,
      fill: color.textSec,
    });
    out += text(
      x + colW + 14,
      ry + 10,
      fitWidth(row.value, s.width - colW - 22, { size: FONT.small }),
      {
        size: FONT.small,
        fill: color.text,
      },
    );
  });
  return out;
}

function boundaryVisual(
  b: UsecaseBoundary,
  sized: Map<string, SizedElem>,
  model: UsecaseModel,
  budget: StepBudget,
): {
  width: number;
  height: number;
  relPos: Map<string, { x: number; y: number }>;
} {
  const members = b.memberIds
    .map((id) => sized.get(id))
    .filter((s): s is SizedElem => s !== undefined);
  const titleW = textWidth(b.title, { size: FONT.body, bold: true }) + 32;
  if (members.length === 0) {
    return { width: Math.max(160, titleW), height: 80, relPos: new Map() };
  }
  const memberSet = new Set(b.memberIds);
  const innerNodes = members.map((m) => ({ id: m.elem.id, width: m.width, height: m.height }));
  const innerEdges = model.relations
    .filter((r) => memberSet.has(r.from) && memberSet.has(r.to))
    .map((r) => ({
      from: r.from,
      to: r.to,
      labelWidth: r.label ? textWidth(r.label, { size: FONT.small }) + 12 : undefined,
      labelHeight: r.label ? 18 : undefined,
    }));
  const layout = layeredLayout(
    innerNodes,
    innerEdges,
    { direction: model.direction, margin: 18, nodeGap: 28, rankGap: 44 },
    budget,
  );
  const headerH = b.type === "package" ? 36 : 32;
  const width = Math.max(layout.width, titleW + 16);
  const offsetX = (width - layout.width) / 2;
  const relPos = new Map<string, { x: number; y: number }>();
  for (const [id, pos] of layout.nodes) {
    relPos.set(id, { x: pos.x + offsetX, y: pos.y + headerH });
  }
  return { width, height: layout.height + headerH, relPos };
}

function markerAttr(m: RelMarker): string | undefined {
  if (m === "arrow") return markerRef("arrow-filled");
  if (m === "triangle") return markerRef("triangle-hollow");
  if (m === "cross") return markerRef("arrow-cross");
  return undefined;
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

/** Draw a parsed use case diagram as a complete SVG string. */
export function drawUsecase(model: UsecaseModel): string {
  const budget = new StepBudget();
  const sized = new Map(model.elements.map((e) => [e.id, measureElement(e)]));
  const boundaryLayouts = new Map<string, ReturnType<typeof boundaryVisual>>();
  for (const b of model.boundaries) {
    boundaryLayouts.set(b.id, boundaryVisual(b, sized, model, budget));
  }

  const topNodes: { id: string; width: number; height: number }[] = [];
  const containerOf = (id: string): string => {
    const el = model.elements.find((e) => e.id === id);
    return el && el.kind !== "json" && el.boundaryId ? `\u0000b:${el.boundaryId}` : id;
  };
  for (const b of model.boundaries) {
    const bl = boundaryLayouts.get(b.id);
    if (bl) topNodes.push({ id: `\u0000b:${b.id}`, width: bl.width, height: bl.height });
  }
  for (const e of model.elements) {
    if (e.kind !== "json" && e.boundaryId) continue;
    const s = sized.get(e.id);
    if (s) topNodes.push({ id: e.id, width: s.width, height: s.height });
  }
  const noteLines = model.notes.map((note) => wrapText(note.text, 190, { size: FONT.small + 1 }));
  model.notes.forEach((_, i) => {
    const m = measureLines(noteLines[i] ?? [], { size: FONT.small + 1 });
    topNodes.push({ id: `\u0000n:${String(i)}`, width: m.width + 20, height: m.height + 14 });
  });

  const topEdges: {
    from: string;
    to: string;
    labelWidth?: number | undefined;
    labelHeight?: number | undefined;
  }[] = [];
  for (const r of model.relations) {
    const a = containerOf(r.from);
    const b = containerOf(r.to);
    if (a !== b) {
      topEdges.push({
        from: a,
        to: b,
        labelWidth: r.label ? textWidth(r.label, { size: FONT.small }) + 12 : undefined,
        labelHeight: r.label ? 18 : undefined,
      });
    }
  }
  model.notes.forEach((note, i) => {
    topEdges.push({ from: `\u0000n:${String(i)}`, to: containerOf(note.target) });
  });

  const outer = layeredLayout(
    topNodes,
    topEdges,
    { direction: model.direction, margin: PAD, nodeGap: 40, rankGap: 60 },
    budget,
  );

  // Resolve absolute positions for every element (including inside boundaries).
  const absPos = new Map<string, { x: number; y: number; width: number; height: number }>();
  for (const e of model.elements) {
    const s = sized.get(e.id);
    if (!s) continue;
    if (e.kind !== "json" && e.boundaryId) {
      const bp = outer.nodes.get(`\u0000b:${e.boundaryId}`);
      const bl = boundaryLayouts.get(e.boundaryId);
      const rel = bl?.relPos.get(e.id);
      if (bp && bl && rel) {
        absPos.set(e.id, {
          x: bp.x - bl.width / 2 + rel.x,
          y: bp.y - bl.height / 2 + rel.y,
          width: s.width,
          height: s.height,
        });
      }
    } else {
      const p = outer.nodes.get(e.id);
      if (p) absPos.set(e.id, p);
    }
  }

  const back: string[] = [];
  const mid: string[] = [];
  const front: string[] = [];

  // Draw system boundaries.
  model.boundaries.forEach((b, bi) => {
    const bp = outer.nodes.get(`\u0000b:${b.id}`);
    const bl = boundaryLayouts.get(b.id);
    if (!bp || !bl) return;
    const x = bp.x - bl.width / 2;
    const y = bp.y - bl.height / 2;
    const tint = series(bi);
    if (b.type === "package") {
      const tabW = Math.min(
        bl.width - 24,
        textWidth(b.title, { size: FONT.body, bold: true }) + 28,
      );
      back.push(
        rect(x, y, tabW, 24, {
          rx: 4,
          fill: tint,
          "fill-opacity": 0.14,
          stroke: color.nodeStroke,
          "stroke-width": 1.2,
        }),
      );
      back.push(
        rect(x, y + 20, bl.width, bl.height - 20, {
          rx: 6,
          fill: tint,
          "fill-opacity": 0.06,
          stroke: color.nodeStroke,
          "stroke-width": 1.2,
        }),
      );
      back.push(
        text(x + 12, y + 12, fitWidth(b.title, tabW - 16, { size: FONT.body, bold: true }), {
          size: FONT.body,
          weight: "700",
          fill: color.text,
        }),
      );
    } else {
      back.push(
        rect(x, y, bl.width, bl.height, {
          rx: 8,
          fill: tint,
          "fill-opacity": 0.06,
          stroke: color.nodeStroke,
          "stroke-width": 1.2,
        }),
      );
      back.push(
        text(bp.x, y + 16, fitWidth(b.title, bl.width - 20, { size: FONT.body, bold: true }), {
          size: FONT.body,
          weight: "700",
          anchor: "middle",
          fill: color.text,
        }),
      );
    }
  });

  // Draw relationships.
  model.relations.forEach((rel, ri) => {
    budget.tick();
    const a = absPos.get(rel.from);
    const b = absPos.get(rel.to);
    if (!a || !b) return;
    const p1 = clipBox(a.x, a.y, a.width, a.height, b.x, b.y);
    const p2 = clipBox(b.x, b.y, b.width, b.height, a.x, a.y);
    const pts: Point[] =
      rel.from === rel.to
        ? [
            [a.x + a.width / 2, a.y - 10],
            [a.x + a.width / 2 + 24, a.y - 10],
            [a.x + a.width / 2 + 24, a.y + 10],
            [a.x + a.width / 2, a.y + 10],
          ]
        : [p1, p2];

    const strokeColor =
      rel.kind === "include" ? color.info : rel.kind === "extend" ? color.accent : color.line;
    mid.push(
      path(roundedPath(pts, 6), {
        fill: "none",
        stroke: strokeColor,
        "stroke-width": 1.4,
        "stroke-dasharray": rel.dashed ? "5 4" : undefined,
        "marker-start": markerAttr(rel.startMarker),
        "marker-end": markerAttr(rel.endMarker),
      }),
    );
    if (rel.startMarker === "circle") {
      mid.push(
        circle(p1[0], p1[1], 4.5, { fill: color.canvas, stroke: strokeColor, "stroke-width": 1.4 }),
      );
    }
    if (rel.endMarker === "circle") {
      mid.push(
        circle(p2[0], p2[1], 4.5, { fill: color.canvas, stroke: strokeColor, "stroke-width": 1.4 }),
      );
    }
    if (rel.label) {
      const mx = (p1[0] + p2[0]) / 2;
      const my = (p1[1] + p2[1]) / 2 + ((ri % 3) - 1) * 4;
      const lbl = fitWidth(rel.label, 160, { size: FONT.small });
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

  // Draw elements.
  for (const e of model.elements) {
    const s = sized.get(e.id);
    const p = absPos.get(e.id);
    if (s && p) front.push(drawElement(s, p.x, p.y));
  }

  // Draw notes and dotted note connectors.
  model.notes.forEach((note, i) => {
    const np = outer.nodes.get(`\u0000n:${String(i)}`);
    const tp = absPos.get(note.target);
    if (!np) return;
    if (tp) {
      const p1 = clipBox(np.x, np.y, np.width, np.height, tp.x, tp.y);
      const p2 = clipBox(tp.x, tp.y, tp.width, tp.height, np.x, np.y);
      mid.push(
        line(p1[0], p1[1], p2[0], p2[1], {
          stroke: color.textFaint,
          "stroke-width": 1.1,
          "stroke-dasharray": "3 3",
        }),
      );
    }
    const lines = noteLines[i] ?? [];
    front.push(
      rect(np.x - np.width / 2, np.y - np.height / 2, np.width, np.height, {
        rx: 4,
        fill: color.groupHdr,
        stroke: color.nodeStroke,
        "stroke-width": 1,
      }),
    );
    front.push(
      textLines(
        np.x - np.width / 2 + 10,
        np.y - ((lines.length - 1) * (FONT.small + 1) * 1.3) / 2,
        lines,
        { size: FONT.small + 1 },
      ),
    );
  });

  return svgDoc(
    outer.width,
    outer.height,
    [...back, ...mid, ...front].join(""),
    markerDefs(["arrow-filled", "triangle-hollow", "arrow-cross"]),
  );
}
