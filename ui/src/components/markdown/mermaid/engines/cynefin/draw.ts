/**
 * Cynefin framework drawing: 2x2 quadrants (Complex, Complicated, Chaotic, Clear) separated by a
 * deterministic wavy boundary and Clear/Chaotic cliff, centered Confusion ellipse (capped at three
 * items plus overflow pill), and curved domain transitions.
 */
import { StepBudget } from "../../limits";
import { fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import { el, line, markerDefs, markerRef, n, path, rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { CynefinDomain, CynefinModel } from "./parse";

const PAD = 24;
const QUAD_W = 380;
const MIN_QUAD_H = 256;
const CONF_RX = 132;
const CONF_RY = 76;
const PILL_LINE = 16;
const PILL_GAP = 8;

interface QuadMeta {
  domain: Exclude<CynefinDomain, "confusion">;
  heading: string;
  subtitle: string;
  col: 0 | 1;
  row: 0 | 1;
  tint: string;
}

const QUADS: readonly QuadMeta[] = [
  {
    domain: "complex",
    heading: "Complex",
    subtitle: "Probe · Sense · Respond · Emergent",
    col: 0,
    row: 0,
    tint: series(6),
  },
  {
    domain: "complicated",
    heading: "Complicated",
    subtitle: "Sense · Analyse · Respond · Good",
    col: 1,
    row: 0,
    tint: series(1),
  },
  {
    domain: "chaotic",
    heading: "Chaotic",
    subtitle: "Act · Sense · Respond · Novel",
    col: 0,
    row: 1,
    tint: series(2),
  },
  {
    domain: "clear",
    heading: "Clear",
    subtitle: "Sense · Categorise · Respond · Best",
    col: 1,
    row: 1,
    tint: series(4),
  },
];

interface ItemPill {
  lines: string[];
  width: number;
  height: number;
}

function layoutPills(items: readonly string[], maxW: number, budget: StepBudget): ItemPill[] {
  const style = { size: FONT.small + 1 };
  return items.map((item) => {
    budget.tick();
    const lines = wrapText(item, maxW - 20, style, 3);
    const measured = measureLines(lines, style, 1.3);
    return {
      lines,
      width: Math.min(maxW, Math.max(84, measured.width + 20)),
      height: lines.length * PILL_LINE + 10,
    };
  });
}

function wavyVerticalPath(cx: number, top: number, bottom: number, budget: StepBudget): string {
  const segments = 24;
  const step = (bottom - top) / segments;
  let d = `M${n(cx)},${n(top)}`;
  for (let i = 1; i <= segments; i++) {
    budget.tick();
    const y = top + i * step;
    const t = i / segments;
    // Gentle organic S-wave above the cliff; sharpest curve near the bottom Clear/Chaotic fold.
    const amp = t > 0.85 ? 10 : 7;
    const x = cx + Math.sin(t * Math.PI * 3.5 + 0.4) * amp;
    d += ` L${n(x)},${n(y)}`;
  }
  return d;
}

function cliffPath(cx: number, bottom: number): string {
  // Recognizable Cynefin "cliff" fold between Clear (bottom-right) and Chaotic (bottom-left).
  return (
    `M${n(cx - 22)},${n(bottom - 26)} ` +
    `Q${n(cx - 2)},${n(bottom - 30)} ${n(cx + 8)},${n(bottom - 14)} ` +
    `L${n(cx + 16)},${n(bottom)}`
  );
}

/** Draw a parsed Cynefin framework diagram as a complete SVG string. */
export function drawCynefin(model: CynefinModel): string {
  const budget = new StepBudget();
  const maxPillW = QUAD_W - 44;
  const quadPills = new Map<Exclude<CynefinDomain, "confusion">, ItemPill[]>();
  let maxTopItemsH = 0;
  let maxBotItemsH = 0;
  for (const meta of QUADS) {
    const pills = layoutPills(model.domains[meta.domain], maxPillW, budget);
    quadPills.set(meta.domain, pills);
    const needed = pills.reduce((sum, p) => sum + p.height + PILL_GAP, 0) + (pills.length ? 12 : 0);
    if (meta.row === 0) maxTopItemsH = Math.max(maxTopItemsH, needed);
    else maxBotItemsH = Math.max(maxBotItemsH, needed);
  }

  const headBand = 54;
  const topQuadH = Math.max(MIN_QUAD_H, headBand + maxTopItemsH + CONF_RY + 22);
  const botQuadH = Math.max(MIN_QUAD_H, headBand + maxBotItemsH + CONF_RY + 22);
  const quadH = Math.max(topQuadH, botQuadH);

  const titleLines = model.title
    ? wrapText(model.title, QUAD_W * 2 - 40, { size: FONT.title, bold: true }, 2)
    : [];
  const titleH = titleLines.length ? titleLines.length * FONT.title * 1.3 + 16 : 0;

  const left = PAD;
  const top = PAD + titleH;
  const totalW = QUAD_W * 2;
  const totalH = quadH * 2;
  const cx = left + QUAD_W;
  const cy = top + quadH;
  const svgW = totalW + PAD * 2;
  const svgH = top + totalH + PAD;

  const back: string[] = [];
  const mid: string[] = [];
  const front: string[] = [];

  if (titleLines.length > 0) {
    front.push(
      textLines(svgW / 2, PAD + 10, titleLines, {
        size: FONT.title,
        weight: "700",
        anchor: "middle",
      }),
    );
  }

  // Draw quadrants and their items outside the central Confusion ellipse.
  const domainAnchors = new Map<CynefinDomain, [number, number]>();
  for (const meta of QUADS) {
    const qx = left + meta.col * QUAD_W;
    const qy = top + meta.row * quadH;
    back.push(
      rect(qx, qy, QUAD_W, quadH, {
        fill: meta.tint,
        "fill-opacity": 0.08,
      }),
    );
    const headingX = meta.col === 0 ? qx + 18 : qx + QUAD_W - 18;
    const anchor = meta.col === 0 ? "start" : "end";
    const headingY = meta.row === 0 ? qy + 22 : qy + quadH - 36;
    const subY = meta.row === 0 ? qy + 40 : qy + quadH - 18;
    front.push(
      text(headingX, headingY, meta.heading, {
        size: FONT.body + 1,
        weight: "700",
        anchor,
      }),
    );
    front.push(
      text(headingX, subY, meta.subtitle, {
        size: FONT.small,
        anchor,
        fill: color.textSec,
      }),
    );

    const pills = quadPills.get(meta.domain) ?? [];
    let cursorY = meta.row === 0 ? qy + headBand : qy + CONF_RY + 18;
    const xStart = qx + 18;
    for (const pill of pills) {
      budget.tick();
      const px = meta.col === 0 ? xStart : qx + QUAD_W - 18 - pill.width;
      front.push(
        rect(px, cursorY, pill.width, pill.height, {
          rx: 6,
          fill: color.nodeFill,
          stroke: color.nodeStroke,
          "stroke-width": 1,
        }),
      );
      front.push(
        textLines(
          px + 10,
          cursorY + 5 + (FONT.small + 1) * 0.65,
          pill.lines,
          { size: FONT.small + 1 },
          PILL_LINE / (FONT.small + 1),
        ),
      );
      cursorY += pill.height + PILL_GAP;
    }

    domainAnchors.set(meta.domain, [
      qx + (meta.col === 0 ? QUAD_W * 0.44 : QUAD_W * 0.56),
      qy + (meta.row === 0 ? quadH * 0.44 : quadH * 0.56),
    ]);
  }
  domainAnchors.set("confusion", [cx, cy]);

  // Outer frame + horizontal axis + wavy ordered/unordered divider + Clear/Chaotic cliff.
  mid.push(
    rect(left, top, totalW, totalH, {
      rx: 10,
      fill: "none",
      stroke: color.nodeStroke,
      "stroke-width": 1.4,
    }),
  );
  mid.push(
    line(left, cy, left + totalW, cy, {
      stroke: color.line,
      "stroke-width": 1.3,
    }),
  );
  mid.push(
    path(wavyVerticalPath(cx, top, top + totalH, budget), {
      fill: "none",
      stroke: color.line,
      "stroke-width": 1.5,
    }),
  );
  mid.push(
    path(cliffPath(cx, top + totalH), {
      fill: "none",
      stroke: color.crit,
      "stroke-width": 2,
      "stroke-linecap": "round",
    }),
  );

  // Center Confusion ellipse (capped at 3 visible items + overflow pill).
  const confusionItems = model.domains.confusion;
  const shownConfusion = confusionItems.slice(0, 3);
  const overflowCount = Math.max(0, confusionItems.length - 3);
  const confStyle = { size: FONT.small };
  const confH = Math.max(
    CONF_RY * 2,
    46 + shownConfusion.length * 22 + (overflowCount > 0 ? 22 : 0),
  );
  const confRy = confH / 2;

  mid.push(
    el("ellipse", {
      cx,
      cy,
      rx: CONF_RX,
      ry: confRy,
      fill: color.canvas,
      stroke: color.nodeStroke,
      "stroke-width": 1.4,
    }),
  );
  mid.push(
    el("ellipse", {
      cx,
      cy,
      rx: CONF_RX,
      ry: confRy,
      fill: series(7),
      "fill-opacity": 0.14,
      stroke: "none",
    }),
  );
  front.push(
    text(cx, cy - confRy + 18, "Confusion", {
      size: FONT.body,
      weight: "700",
      anchor: "middle",
    }),
  );
  let confY = cy - confRy + 36;
  for (const item of shownConfusion) {
    budget.tick();
    const label = fitWidth(item, CONF_RX * 2 - 44, confStyle);
    const pw = Math.min(CONF_RX * 2 - 36, Math.max(70, textWidth(label, confStyle) + 18));
    front.push(
      rect(cx - pw / 2, confY - 8, pw, 18, {
        rx: 5,
        fill: color.nodeFill,
        stroke: color.innerStroke,
        "stroke-width": 1,
      }),
    );
    front.push(
      text(cx, confY + 1, label, {
        size: FONT.small,
        anchor: "middle",
      }),
    );
    confY += 22;
  }
  if (overflowCount > 0) {
    const more = `+${String(overflowCount)} more`;
    const mw = textWidth(more, confStyle) + 16;
    front.push(
      rect(cx - mw / 2, confY - 8, mw, 18, {
        rx: 9,
        fill: color.groupHdr,
        stroke: color.innerStroke,
        "stroke-width": 1,
      }),
    );
    front.push(
      text(cx, confY + 1, more, {
        size: FONT.small,
        weight: "600",
        anchor: "middle",
        fill: color.textSec,
      }),
    );
  }

  // Transitions between domains.
  const pairCounts = new Map<string, number>();
  for (const tr of model.transitions) {
    const key = [tr.from, tr.to].sort().join("|");
    pairCounts.set(key, (pairCounts.get(key) ?? 0) + 1);
  }
  const seenPair = new Map<string, number>();
  for (const tr of model.transitions) {
    budget.tick();
    const p0 = domainAnchors.get(tr.from) ?? [cx, cy];
    const p1 = domainAnchors.get(tr.to) ?? [cx, cy];
    const dx = p1[0] - p0[0];
    const dy = p1[1] - p0[1];
    const len = Math.hypot(dx, dy) || 1;
    const ux = dx / len;
    const uy = dy / len;
    const startTrim = tr.from === "confusion" ? 72 : 44;
    const endTrim = tr.to === "confusion" ? 72 : 44;
    const sx = p0[0] + ux * Math.min(startTrim, len * 0.35);
    const sy = p0[1] + uy * Math.min(startTrim, len * 0.35);
    const ex = p1[0] - ux * Math.min(endTrim, len * 0.35);
    const ey = p1[1] - uy * Math.min(endTrim, len * 0.35);

    const key = [tr.from, tr.to].sort().join("|");
    const idx = seenPair.get(key) ?? 0;
    seenPair.set(key, idx + 1);
    const totalInPair = pairCounts.get(key) ?? 1;
    const curveDir = tr.from < tr.to ? 1 : -1;
    const spread = totalInPair > 1 ? curveDir * (22 + idx * 14) : 20;
    const mx = (sx + ex) / 2 - uy * spread;
    const my = (sy + ey) / 2 + ux * spread;

    front.push(
      path(`M${n(sx)},${n(sy)} Q${n(mx)},${n(my)} ${n(ex)},${n(ey)}`, {
        fill: "none",
        stroke: color.arrow,
        "stroke-width": 1.6,
        "marker-end": markerRef("arrow-filled"),
      }),
    );

    if (tr.label) {
      const lx = 0.25 * sx + 0.5 * mx + 0.25 * ex;
      const ly = 0.25 * sy + 0.5 * my + 0.25 * ey;
      const lText = fitWidth(tr.label, 200, { size: FONT.small + 1 });
      const lw = textWidth(lText, { size: FONT.small + 1 }) + 14;
      front.push(
        rect(lx - lw / 2, ly - 10, lw, 20, {
          rx: 5,
          fill: color.canvas,
          "fill-opacity": 0.94,
          stroke: color.innerStroke,
          "stroke-width": 0.8,
        }),
      );
      front.push(
        text(lx, ly, lText, {
          size: FONT.small + 1,
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }
  }

  return svgDoc(svgW, svgH, [...back, ...mid, ...front].join(""), markerDefs(["arrow-filled"]));
}
