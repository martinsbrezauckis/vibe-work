/**
 * Timeline diagram drawing: supports LR (left-to-right axis with stacked event cards) and TD
 * (top-to-bottom spine with lateral event cards), section banners, `<br>` line wrapping, and
 * per-section or per-period category colours using theme tokens only.
 */
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import { g, line, markerDefs, markerRef, rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { TimelineModel, TimelinePeriod } from "./parse";

const PAD = 24;
const COL_MIN = 112;
const COL_GAP = 14;
const EV_GAP = 8;
const LINE_H = 1.3;

interface ColouredPeriod {
  period: TimelinePeriod;
  colorIndex: number;
  sectionIndex: number;
}

interface SectionGroup {
  name: string;
  colorIndex: number;
  startIndex: number;
  count: number;
}

function collectGroups(
  model: TimelineModel,
  budget: StepBudget,
): { items: ColouredPeriod[]; groups: SectionGroup[] } {
  const items: ColouredPeriod[] = [];
  const groups: SectionGroup[] = [];
  let sectionColor = 0;
  let periodColor = 0;
  for (let si = 0; si < model.sections.length; si++) {
    const section = model.sections[si];
    if (!section || section.periods.length === 0) continue;
    const cIdx = model.hasExplicitSections ? sectionColor++ : 0;
    groups.push({
      name: section.name,
      colorIndex: cIdx,
      startIndex: items.length,
      count: section.periods.length,
    });
    for (const period of section.periods) {
      budget.tick();
      items.push({
        period,
        colorIndex: model.hasExplicitSections ? cIdx : periodColor++,
        sectionIndex: si,
      });
    }
  }
  return { items, groups };
}

function drawPeriodCard(
  x: number,
  y: number,
  w: number,
  h: number,
  lines: readonly string[],
  tone: string,
): string {
  const box = rect(x, y, w, h, {
    rx: 8,
    fill: tone,
    "fill-opacity": 0.2,
    stroke: tone,
    "stroke-width": 1.4,
  });
  const textY = y + h / 2 - ((lines.length - 1) * FONT.body * LINE_H) / 2;
  return (
    box +
    textLines(x + w / 2, textY, lines, { size: FONT.body, weight: "600", anchor: "middle" }, LINE_H)
  );
}

function drawEventCard(
  x: number,
  y: number,
  w: number,
  h: number,
  lines: readonly string[],
  tone: string,
): string {
  const bg = rect(x, y, w, h, {
    rx: 5,
    fill: tone,
    "fill-opacity": 0.1,
    stroke: color.innerStroke,
    "stroke-width": 1,
  });
  const stripe = rect(x, y, 4, h, { rx: 2, fill: tone });
  const textY = y + h / 2 - ((lines.length - 1) * FONT.small * LINE_H) / 2;
  const label = textLines(x + 12, textY, lines, { size: FONT.small, fill: color.textSec }, LINE_H);
  return bg + stripe + label;
}

function drawLR(model: TimelineModel, budget: StepBudget): string {
  const { items, groups } = collectGroups(model, budget);
  const titleStyle = { size: FONT.title, bold: true };
  const periodStyle = { size: FONT.body, bold: true };
  const eventStyle = { size: FONT.small };
  const sectionStyle = { size: FONT.body, bold: true };

  const maxColBudget = Math.floor((15_200 - PAD * 2) / Math.max(1, items.length) - COL_GAP);
  const colMax = Math.max(COL_MIN, Math.min(188, maxColBudget));
  let idealColW = COL_MIN;
  for (const item of items) {
    budget.tick();
    idealColW = Math.max(idealColW, textWidth(clampLabel(item.period.label), periodStyle) + 22);
    for (const ev of item.period.events) {
      budget.tick();
      idealColW = Math.max(idealColW, Math.min(colMax, textWidth(clampLabel(ev), eventStyle) + 22));
    }
  }
  const colW = Math.min(colMax, Math.ceil(idealColW));
  const contentW = items.length * colW + Math.max(0, items.length - 1) * COL_GAP;
  const titleText = model.title
    ? fitWidth(clampLabel(model.title), Math.max(460, contentW), titleStyle)
    : "";
  const titleW = titleText ? textWidth(titleText, titleStyle) + PAD * 2 : 0;
  const width = Math.max(PAD * 2 + contentW, titleW, 320);

  const titleH = titleText ? 36 : 0;
  const secLayouts = groups.map((grp) => {
    budget.tick();
    const spanW = grp.count * colW + (grp.count - 1) * COL_GAP;
    const lines =
      model.hasExplicitSections && grp.name
        ? wrapText(grp.name, Math.max(60, spanW - 14), sectionStyle, 3)
        : [];
    return { grp, spanW, lines };
  });
  const sectionH = model.hasExplicitSections
    ? Math.max(
        32,
        ...secLayouts.map((s) =>
          s.lines.length > 0 ? measureLines(s.lines, sectionStyle, LINE_H).height + 14 : 0,
        ),
      )
    : 0;

  const periodWrapped = items.map((item) => {
    budget.tick();
    const pLines = wrapText(item.period.label, colW - 16, periodStyle, 4).map((l) =>
      fitWidth(l, colW - 12, periodStyle),
    );
    const pH = Math.max(36, Math.ceil(measureLines(pLines, periodStyle, LINE_H).height + 16));
    const evCards = item.period.events.map((ev) => {
      budget.tick();
      const eLines = wrapText(ev, colW - 22, eventStyle, 6).map((l) =>
        fitWidth(l, colW - 18, eventStyle),
      );
      const eH = Math.max(28, Math.ceil(measureLines(eLines, eventStyle, LINE_H).height + 12));
      return { lines: eLines, height: eH };
    });
    return { item, pLines, pH, evCards };
  });

  const periodH = Math.max(38, ...periodWrapped.map((p) => p.pH));
  const maxEventsH = Math.max(
    0,
    ...periodWrapped.map(
      (p) =>
        p.evCards.reduce((sum, ev) => sum + ev.height, 0) +
        Math.max(0, p.evCards.length - 1) * EV_GAP,
    ),
  );

  const secTop = PAD + titleH;
  const periodTop = secTop + (sectionH > 0 ? sectionH + 14 : 0);
  const axisY = periodTop + periodH / 2;
  const eventsTop = periodTop + periodH + 20;
  const height = eventsTop + maxEventsH + PAD;

  let body = "";
  if (titleText) {
    body += text(width / 2, PAD + 10, titleText, {
      size: FONT.title,
      weight: "700",
      anchor: "middle",
    });
  }

  if (sectionH > 0) {
    for (const s of secLayouts) {
      budget.tick();
      if (!s.grp.name) continue;
      const sx = PAD + s.grp.startIndex * (colW + COL_GAP);
      const tone = series(s.grp.colorIndex);
      body += rect(sx, secTop, s.spanW, sectionH, {
        rx: 6,
        fill: tone,
        "fill-opacity": 0.14,
        stroke: tone,
        "stroke-width": 1.2,
      });
      const ty = secTop + sectionH / 2 - ((s.lines.length - 1) * FONT.body * LINE_H) / 2;
      body += textLines(
        sx + s.spanW / 2,
        ty,
        s.lines,
        { size: FONT.body, weight: "600", anchor: "middle" },
        LINE_H,
      );
    }
  }

  // Horizontal axis line behind period boxes.
  body += line(PAD - 8, axisY, PAD + contentW + 10, axisY, {
    stroke: color.arrow,
    "stroke-width": 1.6,
    "marker-end": markerRef("arrow-open"),
  });

  for (let i = 0; i < periodWrapped.length; i++) {
    budget.tick();
    const entry = periodWrapped[i];
    if (!entry) continue;
    const x = PAD + i * (colW + COL_GAP);
    const cx = x + colW / 2;
    const tone = series(entry.item.colorIndex);
    // Background canvas mask keeps the axis line from cutting across translucent period boxes.
    body += rect(x, periodTop, colW, periodH, { rx: 8, fill: color.canvas });
    body += drawPeriodCard(x, periodTop, colW, periodH, entry.pLines, tone);

    let ey = eventsTop;
    if (entry.evCards.length > 0) {
      const totalEvH =
        entry.evCards.reduce((sum, c) => sum + c.height, 0) + (entry.evCards.length - 1) * EV_GAP;
      body += line(cx, periodTop + periodH, cx, eventsTop + totalEvH - 4, {
        stroke: tone,
        "stroke-width": 1.2,
        "stroke-dasharray": "3 3",
      });
    }
    for (const card of entry.evCards) {
      budget.tick();
      body += drawEventCard(x, ey, colW, card.height, card.lines, tone);
      ey += card.height + EV_GAP;
    }
  }

  return svgDoc(width, height, g({}, body), markerDefs(["arrow-open"]));
}

function drawTD(model: TimelineModel, budget: StepBudget): string {
  const { items, groups } = collectGroups(model, budget);
  const titleStyle = { size: FONT.title, bold: true };
  const periodStyle = { size: FONT.body, bold: true };
  const eventStyle = { size: FONT.small };
  const sectionStyle = { size: FONT.body, bold: true };

  const secColW = model.hasExplicitSections ? 140 : 0;
  const periodW = 148;
  const evCardW = 190;
  const evGap = 10;
  const rowGap = 14;
  const maxEventsPerRow = 3;

  const spineX = PAD + (secColW > 0 ? secColW + 16 : 0) + periodW / 2;
  const periodX = spineX - periodW / 2;
  const eventsStartX = periodX + periodW + 24;

  const rows = items.map((item) => {
    budget.tick();
    const pLines = wrapText(item.period.label, periodW - 18, periodStyle, 4).map((l) =>
      fitWidth(l, periodW - 14, periodStyle),
    );
    const pH = Math.max(36, Math.ceil(measureLines(pLines, periodStyle, LINE_H).height + 16));
    const evCards = item.period.events.map((ev) => {
      budget.tick();
      const eLines = wrapText(ev, evCardW - 22, eventStyle, 6).map((l) =>
        fitWidth(l, evCardW - 18, eventStyle),
      );
      const eH = Math.max(30, Math.ceil(measureLines(eLines, eventStyle, LINE_H).height + 14));
      return { lines: eLines, height: eH };
    });
    const evRows: { cards: { lines: string[]; height: number }[]; height: number }[] = [];
    for (let k = 0; k < evCards.length; k += maxEventsPerRow) {
      const slice = evCards.slice(k, k + maxEventsPerRow);
      evRows.push({
        cards: slice,
        height: Math.max(30, ...slice.map((c) => c.height)),
      });
    }
    const evTotalH =
      evRows.reduce((s, r) => s + r.height, 0) + Math.max(0, evRows.length - 1) * EV_GAP;
    const rowH = Math.max(pH, evTotalH);
    const colsUsed = Math.min(maxEventsPerRow, evCards.length);
    return { item, pLines, pH, evRows, rowH, colsUsed };
  });

  const maxColsUsed = Math.max(1, ...rows.map((r) => r.colsUsed));
  const contentRight = eventsStartX + maxColsUsed * evCardW + Math.max(0, maxColsUsed - 1) * evGap;
  const titleText = model.title
    ? fitWidth(clampLabel(model.title), Math.max(460, contentRight), titleStyle)
    : "";
  const titleW = titleText ? textWidth(titleText, titleStyle) + PAD * 2 : 0;
  const width = Math.max(contentRight + PAD, titleW);

  const titleH = titleText ? 36 : 0;
  let cursorY = PAD + titleH;
  const rowYs: number[] = [];
  for (let i = 0; i < rows.length; i++) {
    rowYs.push(cursorY);
    cursorY += (rows[i]?.rowH ?? 40) + rowGap;
  }
  const bottomY = cursorY - rowGap;
  const height = bottomY + PAD + 10;

  let body = "";
  if (titleText) {
    body += text(width / 2, PAD + 10, titleText, {
      size: FONT.title,
      weight: "700",
      anchor: "middle",
    });
  }

  // Section side bands spanning their period rows.
  if (model.hasExplicitSections) {
    for (const grp of groups) {
      budget.tick();
      if (!grp.name) continue;
      const startY = rowYs[grp.startIndex] ?? PAD;
      const lastIdx = grp.startIndex + grp.count - 1;
      const endY = (rowYs[lastIdx] ?? startY) + (rows[lastIdx]?.rowH ?? 36);
      const secH = Math.max(32, endY - startY);
      const tone = series(grp.colorIndex);
      body += rect(PAD, startY, secColW, secH, {
        rx: 6,
        fill: tone,
        "fill-opacity": 0.14,
        stroke: tone,
        "stroke-width": 1.2,
      });
      const sLines = wrapText(grp.name, secColW - 14, sectionStyle, 4);
      const ty = startY + secH / 2 - ((sLines.length - 1) * FONT.body * LINE_H) / 2;
      body += textLines(
        PAD + secColW / 2,
        ty,
        sLines,
        { size: FONT.body, weight: "600", anchor: "middle" },
        LINE_H,
      );
    }
  }

  // Vertical spine behind period boxes.
  body += line(spineX, PAD + titleH - 6, spineX, bottomY + 8, {
    stroke: color.arrow,
    "stroke-width": 1.6,
    "marker-end": markerRef("arrow-open"),
  });

  for (let i = 0; i < rows.length; i++) {
    budget.tick();
    const row = rows[i];
    if (!row) continue;
    const ry = rowYs[i] ?? PAD;
    const tone = series(row.item.colorIndex);
    const pY = ry + (row.rowH - row.pH) / 2;
    const midY = pY + row.pH / 2;
    body += rect(periodX, pY, periodW, row.pH, { rx: 8, fill: color.canvas });
    body += drawPeriodCard(periodX, pY, periodW, row.pH, row.pLines, tone);
    body += line(periodX + periodW, midY, eventsStartX, midY, {
      stroke: tone,
      "stroke-width": 1.3,
    });

    let ey = ry;
    for (const evRow of row.evRows) {
      budget.tick();
      evRow.cards.forEach((card, ci) => {
        const cx = eventsStartX + ci * (evCardW + evGap);
        body += drawEventCard(cx, ey, evCardW, card.height, card.lines, tone);
      });
      ey += evRow.height + EV_GAP;
    }
  }

  return svgDoc(width, height, g({}, body), markerDefs(["arrow-open"]));
}

/** Draw a parsed timeline diagram (LR or TD) as a complete SVG string. */
export function drawTimeline(model: TimelineModel): string {
  const budget = new StepBudget();
  return model.direction === "TD" ? drawTD(model, budget) : drawLR(model, budget);
}
