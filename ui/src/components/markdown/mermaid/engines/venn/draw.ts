/**
 * Venn diagram drawing. Places overlapping circles sized from optional `:size` values, finds
 * interior Chebyshev centers for each set and union region so labels and attached `text` nodes
 * sit inside their own region, and places any non-realizable high-arity intersection into a clean
 * legend below the circles. Uses app theme tokens only.
 */
import { StepBudget } from "../../limits";
import { clampLabel, measureLines, wrapText } from "../../measure";
import { circle, g, rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { VennModel, VennSet, VennTextNode } from "./parse";

const PAD = 24;
const GRID_STEPS = 32;
const LINE_H = 17;

interface PlacedCircle {
  set: VennSet;
  cx: number;
  cy: number;
  r: number;
  index: number;
}

interface RegionSpec {
  idSet: Set<string>;
  title: string;
  texts: VennTextNode[];
  /** Colour index when representing a single set; undefined for unions. */
  singleIndex?: number | undefined;
}

function computeRadii(sets: VennSet[]): number[] {
  const sizes = sets.map((s) => s.size).filter((v): v is number => v !== undefined);
  const baseR = sets.length <= 2 ? 128 : sets.length === 3 ? 116 : 102;
  if (sizes.length === 0) return sets.map(() => baseR);
  const maxS = Math.max(...sizes);
  return sets.map((s) => {
    if (s.size === undefined) return baseR;
    return 86 + 64 * Math.sqrt(s.size / maxS);
  });
}

function placeCircles(sets: VennSet[]): PlacedCircle[] {
  const radii = computeRadii(sets);
  const nSets = sets.length;
  const avgR = radii.reduce((a, b) => a + b, 0) / nSets;

  return sets.map((set, index) => {
    const r = radii[index] ?? 116;
    let cx = 0;
    let cy = 0;
    if (nSets === 2) {
      const r0 = radii[0] ?? 128;
      const r1 = radii[1] ?? 128;
      const sep = (r0 + r1) * 0.54;
      cx = index === 0 ? -sep / 2 : sep / 2;
    } else if (nSets === 3) {
      const d = avgR * 0.96;
      if (index === 0) {
        cx = -d * 0.52;
        cy = -d * 0.3;
      } else if (index === 1) {
        cx = d * 0.52;
        cy = -d * 0.3;
      } else {
        cx = 0;
        cy = d * 0.6;
      }
    } else if (nSets >= 4) {
      const ringR = Math.min(130, avgR * 0.68);
      const angle = -Math.PI / 2 + (index * Math.PI * 2) / nSets;
      cx = ringR * Math.cos(angle);
      cy = ringR * Math.sin(angle);
    }
    return { set, cx, cy, r, index };
  });
}

function findRegionCenter(
  circles: PlacedCircle[],
  includeIds: Set<string>,
  bounds: { minX: number; maxX: number; minY: number; maxY: number },
  budget: StepBudget,
): { x: number; y: number; clearance: number } {
  if (circles.length > 8) {
    const matching = circles.filter((c) => includeIds.has(c.set.id));
    const count = Math.max(1, matching.length);
    const sx = matching.reduce((s, c) => s + c.cx, 0) / count;
    const sy = matching.reduce((s, c) => s + c.cy, 0) / count;
    budget.tick();
    return { x: sx, y: sy, clearance: circles.length === 1 ? (matching[0]?.r ?? 80) : 0 };
  }

  let bestX = 0;
  let bestY = 0;
  let bestClearance = Number.NEGATIVE_INFINITY;
  const spanX = bounds.maxX - bounds.minX;
  const spanY = bounds.maxY - bounds.minY;

  for (let gx = 0; gx <= GRID_STEPS; gx++) {
    const px = bounds.minX + (spanX * gx) / GRID_STEPS;
    for (let gy = 0; gy <= GRID_STEPS; gy++) {
      const py = bounds.minY + (spanY * gy) / GRID_STEPS;
      budget.tick();
      let clearance = Number.POSITIVE_INFINITY;
      for (const c of circles) {
        const dist = Math.hypot(px - c.cx, py - c.cy);
        const signed = includeIds.has(c.set.id) ? c.r - dist : dist - c.r;
        if (signed < clearance) clearance = signed;
      }
      if (clearance > bestClearance) {
        bestClearance = clearance;
        bestX = px;
        bestY = py;
      }
    }
  }
  return { x: bestX, y: bestY, clearance: bestClearance };
}

/** Draw a parsed Venn diagram as a complete passive SVG string. */
export function drawVenn(model: VennModel): string {
  const budget = new StepBudget();
  const circles = placeCircles(model.sets);

  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  for (const c of circles) {
    minX = Math.min(minX, c.cx - c.r);
    minY = Math.min(minY, c.cy - c.r);
    maxX = Math.max(maxX, c.cx + c.r);
    maxY = Math.max(maxY, c.cy + c.r);
  }

  const titleText = model.title ? clampLabel(model.title) : "";
  const titleLines = titleText ? wrapText(titleText, 480, { size: FONT.title, bold: true }, 2) : [];
  const titleH = titleLines.length > 0 ? titleLines.length * 22 + 12 : 0;

  const regions: RegionSpec[] = [];
  model.sets.forEach((s, index) => {
    regions.push({
      idSet: new Set([s.id]),
      title: s.label,
      texts: s.texts,
      singleIndex: index,
    });
  });
  for (const u of model.unions) {
    if (u.label === undefined && u.texts.length === 0) continue;
    regions.push({
      idSet: new Set(u.sets),
      title: u.label ?? "",
      texts: u.texts,
    });
  }

  const placedLabels: {
    x: number;
    y: number;
    lines: string[];
    subLines: string[];
    singleIndex?: number | undefined;
  }[] = [];
  const fallbackLegend: { label: string; texts: string[] }[] = [];
  const bounds = { minX, maxX, minY, maxY };

  for (const reg of regions) {
    const center =
      circles.length === 1 && reg.singleIndex === 0
        ? { x: circles[0]?.cx ?? 0, y: circles[0]?.cy ?? 0, clearance: circles[0]?.r ?? 120 }
        : findRegionCenter(circles, reg.idSet, bounds, budget);

    const maxW = Math.max(76, Math.min(140, center.clearance * 1.6));
    const headLines = reg.title
      ? wrapText(reg.title, maxW, { size: FONT.body, bold: true }, 3)
      : [];
    const subLines = reg.texts.flatMap((t) => wrapText(t.label, maxW, { size: FONT.small + 1 }, 2));

    if (center.clearance >= 14 && placedLabels.length < 36) {
      placedLabels.push({
        x: center.x,
        y: center.y,
        lines: headLines,
        subLines,
        singleIndex: reg.singleIndex,
      });
    } else {
      const desc = reg.title
        ? `${[...reg.idSet].join(" ∩ ")}: ${reg.title}`
        : [...reg.idSet].join(" ∩ ");
      fallbackLegend.push({
        label: clampLabel(desc),
        texts: reg.texts.map((t) => clampLabel(t.label)),
      });
    }
  }

  const legendEntries = fallbackLegend.map((item) =>
    item.texts.length > 0 ? `${item.label} (${item.texts.join(", ")})` : item.label,
  );
  const legendH = legendEntries.length > 0 ? legendEntries.length * 20 + 16 : 0;
  const legendW =
    legendEntries.length > 0
      ? Math.max(
          ...legendEntries.map((line) => measureLines([line], { size: FONT.small + 1 }).width),
        ) + 28
      : 0;
  const titleW =
    titleLines.length > 0 ? measureLines(titleLines, { size: FONT.title, bold: true }).width : 0;

  const diagW = maxX - minX;
  const width = Math.max(diagW + PAD * 2, titleW + PAD * 2, legendW + PAD * 2, 260);
  const height = maxY - minY + PAD * 2 + titleH + legendH;
  const shiftX = (width - diagW) / 2 - minX;
  const shiftY = PAD + titleH - minY;

  let body = "";
  if (titleLines.length > 0) {
    body += textLines(
      width / 2,
      PAD + 8,
      titleLines,
      { size: FONT.title, weight: "600", anchor: "middle", fill: color.text },
      22 / FONT.title,
    );
  }

  const circlesSvg = circles
    .map((c) =>
      circle(c.cx + shiftX, c.cy + shiftY, c.r, {
        fill: series(c.index),
        "fill-opacity": 0.18,
        stroke: series(c.index),
        "stroke-width": 2,
      }),
    )
    .join("");
  body += g({}, circlesSvg);

  const labelsSvg = placedLabels
    .map((lbl) => {
      const totalRows = lbl.lines.length + lbl.subLines.length;
      if (totalRows === 0) return "";
      const startY = lbl.y + shiftY - ((totalRows - 1) * LINE_H) / 2;
      let out = "";
      if (lbl.lines.length > 0) {
        out += textLines(
          lbl.x + shiftX,
          startY,
          lbl.lines,
          {
            size: lbl.singleIndex !== undefined ? FONT.body : FONT.small + 1,
            weight: "600",
            anchor: "middle",
            fill: color.text,
          },
          LINE_H / (lbl.singleIndex !== undefined ? FONT.body : FONT.small + 1),
        );
      }
      if (lbl.subLines.length > 0) {
        const subStart = startY + lbl.lines.length * LINE_H;
        out += textLines(
          lbl.x + shiftX,
          subStart,
          lbl.subLines,
          {
            size: FONT.small,
            anchor: "middle",
            fill: color.textSec,
          },
          LINE_H / FONT.small,
        );
      }
      return out;
    })
    .join("");
  body += g({}, labelsSvg);

  if (legendEntries.length > 0) {
    const legendTop = maxY + shiftY + 14;
    const legendBoxW = Math.min(width - PAD * 2, legendW);
    const legendX = (width - legendBoxW) / 2;
    body += rect(legendX, legendTop, legendBoxW, legendEntries.length * 20 + 10, {
      rx: 6,
      fill: color.groupFill,
      stroke: color.innerStroke,
      "stroke-width": 1,
    });
    legendEntries.forEach((entry, idx) => {
      body += text(legendX + 12, legendTop + 15 + idx * 20, entry, {
        size: FONT.small + 1,
        fill: color.textSec,
      });
    });
  }

  return svgDoc(width, height, body);
}
