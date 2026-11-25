/**
 * Wardley Map drawing: Value-Chain (Visibility) vs Evolution grid with stage dividers, pipeline
 * capsules, dependency & flow connectors, red dashed evolution arrows, sourcing strategy glyphs,
 * inertia bars, notes, numbered annotations with legend box, and accelerator/deaccelerator forces.
 */
import { StepBudget } from "../../limits";
import { fitWidth, textWidth, wrapText } from "../../measure";
import { circle, el, line, markerDefs, markerRef, n, polygon, rect, svgDoc, text } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { WardleyModel, WardleyNode, WardleyStrategy } from "./parse";

const ML = 56;
const MR = 36;
const MB = 68;

const STRATEGY_COLOR: Record<WardleyStrategy, string> = {
  build: series(0),
  buy: series(1),
  outsource: series(4),
  market: series(6),
};

function drawStrategyMarker(cx: number, cy: number, strategy: WardleyStrategy | undefined): string {
  if (!strategy) return "";
  const stroke = STRATEGY_COLOR[strategy];
  switch (strategy) {
    case "build":
      return polygon(
        [
          [cx, cy - 11],
          [cx + 10, cy + 7],
          [cx - 10, cy + 7],
        ],
        { fill: color.canvas, stroke, "stroke-width": 1.6 },
      );
    case "buy":
      return polygon(
        [
          [cx, cy - 10],
          [cx + 10, cy],
          [cx, cy + 10],
          [cx - 10, cy],
        ],
        { fill: color.canvas, stroke, "stroke-width": 1.6 },
      );
    case "outsource":
      return rect(cx - 8, cy - 8, 16, 16, {
        rx: 2,
        fill: color.canvas,
        stroke,
        "stroke-width": 1.6,
      });
    case "market":
      return circle(cx, cy, 10, {
        fill: color.canvas,
        stroke,
        "stroke-width": 1.6,
      });
  }
}

function drawNodeGlyph(
  cx: number,
  cy: number,
  node: WardleyNode,
  px: number,
  pw: number,
  py: number,
  ph: number,
): string {
  let out = "";
  if (node.kind === "anchor") {
    out += circle(cx, cy, 8, {
      fill: color.accent,
      "fill-opacity": 0.22,
      stroke: color.accent,
      "stroke-width": 2,
    });
    out += circle(cx, cy, 3, { fill: color.accent });
  } else {
    out += drawStrategyMarker(cx, cy, node.strategy);
    out += circle(cx, cy, 5, {
      fill: color.canvas,
      stroke: node.strategy ? STRATEGY_COLOR[node.strategy] : color.nodeStroke,
      "stroke-width": 1.8,
    });
  }

  if (node.inertia) {
    out += line(cx + 12, cy - 11, cx + 12, cy + 11, {
      stroke: color.crit,
      "stroke-width": 3.5,
      "stroke-linecap": "round",
    });
  }

  const labelStyle = { size: FONT.small + 1, bold: node.kind === "anchor" };
  const label = fitWidth(node.name, 160, labelStyle);
  const lw = textWidth(label, labelStyle);
  const [dx, dy] = node.labelOffset ?? [10, -10];
  const lx = Math.max(px + 4, Math.min(px + pw - lw - 4, cx + dx));
  const ly = Math.max(py + 12, Math.min(py + ph - 10, cy + dy));
  out += rect(lx - 3, ly - 8, lw + 6, 16, {
    rx: 3,
    fill: color.canvas,
    "fill-opacity": 0.85,
  });
  out += text(lx, ly, label, {
    size: FONT.small + 1,
    weight: node.kind === "anchor" ? "700" : "500",
    fill: node.kind === "anchor" ? color.text : color.textSec,
  });
  return out;
}

/** Draw a parsed Wardley Map as a complete SVG string. */
export function drawWardley(model: WardleyModel): string {
  const budget = new StepBudget();
  const titleLines = model.title
    ? wrapText(model.title, model.width - 80, { size: FONT.title, bold: true }, 2)
    : [];
  const titleH = titleLines.length ? titleLines.length * FONT.title * 1.3 + 12 : 0;
  const mt = 24 + titleH;
  const px = ML;
  const py = mt;
  const pw = model.width - ML - MR;
  const ph = model.height - mt - MB;

  const toX = (evo: number): number => px + evo * pw;
  const toY = (vis: number): number => py + (1 - vis) * ph;

  const back: string[] = [];
  const mid: string[] = [];
  const front: string[] = [];

  if (titleLines.length > 0) {
    titleLines.forEach((lineText, idx) => {
      front.push(
        text(model.width / 2, 18 + idx * 20, lineText, {
          size: FONT.title,
          weight: "700",
          anchor: "middle",
        }),
      );
    });
  }

  // Plot frame and evolution stages.
  back.push(
    rect(px, py, pw, ph, {
      rx: 4,
      fill: color.groupFill,
      stroke: color.innerStroke,
      "stroke-width": 1,
    }),
  );

  let prevBound = 0;
  model.stages.forEach((stage, i) => {
    budget.tick();
    const x0 = toX(prevBound);
    const x1 = toX(stage.boundary);
    if (i > 0) {
      back.push(
        line(x0, py, x0, py + ph, {
          stroke: color.innerStroke,
          "stroke-width": 1,
          "stroke-dasharray": "5 5",
        }),
      );
    }
    const midX = (x0 + x1) / 2;
    front.push(
      text(
        midX,
        py + ph + 16,
        fitWidth(stage.label, Math.max(50, x1 - x0 - 10), { size: FONT.small + 1, bold: true }),
        {
          size: FONT.small + 1,
          weight: "600",
          anchor: "middle",
        },
      ),
    );
    if (stage.sublabel) {
      front.push(
        text(
          midX,
          py + ph + 32,
          fitWidth(stage.sublabel, Math.max(50, x1 - x0 - 10), { size: FONT.small }),
          {
            size: FONT.small,
            anchor: "middle",
            fill: color.textMuted,
          },
        ),
      );
    }
    prevBound = stage.boundary;
  });

  // Axes with arrows and labels.
  back.push(
    line(px, py + ph, px, py + 4, {
      stroke: color.line,
      "stroke-width": 1.4,
      "marker-end": markerRef("arrow-filled"),
    }),
  );
  back.push(
    line(px, py + ph, px + pw - 4, py + ph, {
      stroke: color.line,
      "stroke-width": 1.4,
      "marker-end": markerRef("arrow-filled"),
    }),
  );
  front.push(
    el(
      "text",
      {
        x: 18,
        y: py + ph / 2,
        transform: `rotate(-90 ${n(18)} ${n(py + ph / 2)})`,
        "text-anchor": "middle",
        "font-size": FONT.small + 1,
        "font-weight": "600",
        fill: color.textSec,
      },
      "Visibility",
    ),
  );
  front.push(
    text(px + pw / 2, py + ph + 50, "Evolution", {
      size: FONT.small + 1,
      weight: "600",
      anchor: "middle",
      fill: color.textSec,
    }),
  );

  const byName = new Map(model.nodes.map((node) => [node.name, node]));

  // Pipeline capsules.
  for (const pipe of model.pipelines) {
    budget.tick();
    const members = pipe.members
      .map((name) => byName.get(name))
      .filter((m): m is WardleyNode => m !== undefined);
    if (members.length === 0) continue;
    const minEvo = Math.min(...members.map((m) => m.evolution));
    const maxEvo = Math.max(...members.map((m) => m.evolution));
    const x1 = toX(Math.max(0, minEvo - 0.025));
    const x2 = toX(Math.min(1, maxEvo + 0.025));
    const cy = toY(pipe.visibility);
    mid.push(
      rect(x1, cy - 13, Math.max(24, x2 - x1), 26, {
        rx: 13,
        fill: color.nodeFill,
        "fill-opacity": 0.72,
        stroke: color.nodeStroke,
        "stroke-width": 1.2,
      }),
    );
  }

  // Links and flows.
  for (const link of model.links) {
    budget.tick();
    const a = byName.get(link.from);
    const b = byName.get(link.to);
    if (!a || !b) continue;
    const x1 = toX(a.evolution);
    const y1 = toY(a.visibility);
    const x2 = toX(b.evolution);
    const y2 = toY(b.visibility);
    const isFlow = link.kind === "flow" || link.kind === "rflow" || link.kind === "biflow";
    mid.push(
      line(x1, y1, x2, y2, {
        stroke: isFlow ? color.info : color.line,
        "stroke-width": isFlow ? 2 : 1.3,
        "stroke-dasharray": link.kind === "dashed" ? "5 4" : undefined,
        "marker-start":
          link.kind === "rflow" || link.kind === "biflow" ? markerRef("arrow-filled") : undefined,
        "marker-end":
          link.kind === "flow" || link.kind === "biflow" || link.kind === "dashed"
            ? markerRef("arrow-filled")
            : undefined,
      }),
    );
    if (link.label) {
      const mx = (x1 + x2) / 2;
      const my = (y1 + y2) / 2;
      const lbl = fitWidth(link.label, 140, { size: FONT.small });
      const lw = textWidth(lbl, { size: FONT.small }) + 10;
      mid.push(
        rect(mx - lw / 2, my - 9, lw, 18, {
          rx: 4,
          fill: color.canvas,
          "fill-opacity": 0.9,
          stroke: color.innerStroke,
          "stroke-width": 0.8,
        }),
      );
      mid.push(
        text(mx, my, lbl, {
          size: FONT.small,
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }
  }

  // Evolution and trend arrows.
  for (const node of model.nodes) {
    budget.tick();
    const cx = toX(node.evolution);
    const cy = toY(node.visibility);
    if (node.evolveTo !== undefined) {
      const ex = toX(node.evolveTo);
      mid.push(
        line(cx, cy, ex, cy, {
          stroke: color.crit,
          "stroke-width": 1.6,
          "stroke-dasharray": "5 3",
          "marker-end": markerRef("arrow-filled"),
        }),
      );
      mid.push(
        circle(ex, cy, 5, {
          fill: color.canvas,
          stroke: color.crit,
          "stroke-width": 1.6,
          "stroke-dasharray": "2 2",
        }),
      );
    }
    if (node.trendTo !== undefined) {
      const tx = toX(node.trendTo[0]);
      const ty = toY(node.trendTo[1]);
      mid.push(
        line(cx, cy, tx, ty, {
          stroke: color.warn,
          "stroke-width": 1.5,
          "stroke-dasharray": "4 4",
          "marker-end": markerRef("arrow-filled"),
        }),
      );
    }
  }

  // Nodes.
  for (const node of model.nodes) {
    budget.tick();
    front.push(drawNodeGlyph(toX(node.evolution), toY(node.visibility), node, px, pw, py, ph));
  }

  // Notes.
  for (const note of model.notes) {
    budget.tick();
    const nx = toX(note.evolution);
    const ny = toY(note.visibility);
    const lbl = fitWidth(note.text, 220, { size: FONT.small });
    const nw = textWidth(lbl, { size: FONT.small }) + 16;
    const clampedX = Math.max(px + 4, Math.min(px + pw - nw - 4, nx - nw / 2));
    const clampedY = Math.max(py + 4, Math.min(py + ph - 24, ny - 10));
    front.push(
      rect(clampedX, clampedY, nw, 20, {
        rx: 4,
        fill: color.groupHdr,
        stroke: color.nodeStroke,
        "stroke-width": 1,
      }),
    );
    front.push(
      text(clampedX + nw / 2, clampedY + 10, lbl, {
        size: FONT.small,
        anchor: "middle",
        fill: color.textSec,
      }),
    );
  }

  // Forces (accelerators / deaccelerators).
  for (const force of model.forces) {
    budget.tick();
    const fx = toX(force.evolution);
    const fy = toY(force.visibility);
    const prefix = force.kind === "accelerator" ? ">> " : "<< ";
    const lbl = fitWidth(`${prefix}${force.text}`, 180, { size: FONT.small, bold: true });
    const fw = textWidth(lbl, { size: FONT.small, bold: true }) + 16;
    const clampedX = Math.max(px + 4, Math.min(px + pw - fw - 4, fx - fw / 2));
    const clampedY = Math.max(py + 4, Math.min(py + ph - 24, fy - 11));
    const tone = force.kind === "accelerator" ? color.ok : color.warn;
    front.push(
      rect(clampedX, clampedY, fw, 22, {
        rx: 11,
        fill: tone,
        "fill-opacity": 0.14,
        stroke: tone,
        "stroke-width": 1.3,
      }),
    );
    front.push(
      text(clampedX + fw / 2, clampedY + 11, lbl, {
        size: FONT.small,
        weight: "600",
        anchor: "middle",
        fill: tone,
      }),
    );
  }

  // Numbered annotations + legend box.
  if (model.annotations.length > 0) {
    for (const ann of model.annotations) {
      budget.tick();
      const ax = toX(ann.evolution);
      const ay = toY(ann.visibility);
      front.push(
        circle(ax, ay, 9, { fill: color.accent, stroke: color.canvas, "stroke-width": 1.2 }),
      );
      front.push(
        text(ax, ay, String(ann.num), {
          size: FONT.small,
          weight: "700",
          anchor: "middle",
          fill: color.canvas,
        }),
      );
    }
    const [boxVis, boxEvo] = model.annotationsBox ?? [0.16, 0.04];
    const items = model.annotations.map((a) =>
      fitWidth(`${String(a.num)}. ${a.text}`, 240, { size: FONT.small }),
    );
    const bw = Math.max(140, ...items.map((t) => textWidth(t, { size: FONT.small }) + 20));
    const bh = items.length * 16 + 16;
    const bx = Math.max(px + 8, Math.min(px + pw - bw - 8, toX(boxEvo)));
    const by = Math.max(py + 8, Math.min(py + ph - bh - 8, toY(boxVis)));
    front.push(
      rect(bx, by, bw, bh, {
        rx: 6,
        fill: color.canvas,
        "fill-opacity": 0.94,
        stroke: color.nodeStroke,
        "stroke-width": 1,
      }),
    );
    items.forEach((item, i) => {
      front.push(
        text(bx + 10, by + 16 + i * 16, item, {
          size: FONT.small,
          fill: color.textSec,
        }),
      );
    });
  }

  return svgDoc(
    model.width,
    model.height,
    [...back, ...mid, ...front].join(""),
    markerDefs(["arrow-filled"]),
  );
}
