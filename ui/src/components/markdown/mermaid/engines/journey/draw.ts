/**
 * User journey diagram drawing: actor legend, coloured section banners, wrapped task cards with
 * actor dots, dashed lifelines, and a 5-level satisfaction track with smiley/neutral/sad glyphs.
 */
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import { circle, g, line, n, path, polyline, rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { JourneyModel, JourneyTask } from "./parse";

const PAD = 24;
const AXIS_W = 28;
const COL_MIN = 92;
const COL_GAP = 10;
const TRACK_H = 140;
const FACE_R = 12;
const LINE_H = 1.3;

interface SectionSpan {
  sectionIndex: number;
  name: string;
  startCol: number;
  endCol: number;
  colorIndex: number;
}

function scoreColor(score: number): string {
  if (score >= 3.5) return color.ok;
  if (score >= 2.5) return color.warn;
  return color.crit;
}

function drawFace(cx: number, cy: number, score: number): string {
  const tone = scoreColor(score);
  const face = circle(cx, cy, FACE_R, {
    fill: tone,
    "fill-opacity": 0.2,
    stroke: tone,
    "stroke-width": 1.8,
  });
  const leftEye = circle(cx - 4, cy - 3, 1.4, { fill: tone });
  const rightEye = circle(cx + 4, cy - 3, 1.4, { fill: tone });
  let mouthD: string;
  if (score >= 3.5) {
    mouthD = `M${n(cx - 5)},${n(cy + 2)} Q${n(cx)},${n(cy + 7)} ${n(cx + 5)},${n(cy + 2)}`;
  } else if (score >= 2.5) {
    mouthD = `M${n(cx - 4)},${n(cy + 4)} L${n(cx + 4)},${n(cy + 4)}`;
  } else {
    mouthD = `M${n(cx - 5)},${n(cy + 6)} Q${n(cx)},${n(cy + 1)} ${n(cx + 5)},${n(cy + 6)}`;
  }
  const mouth = path(mouthD, {
    fill: "none",
    stroke: tone,
    "stroke-width": 1.6,
    "stroke-linecap": "round",
  });
  return g({}, face + leftEye + rightEye + mouth);
}

function buildSectionSpans(
  tasks: readonly JourneyTask[],
  sectionNames: readonly string[],
): SectionSpan[] {
  const spans: SectionSpan[] = [];
  let namedCounter = 0;
  for (let i = 0; i < tasks.length; i++) {
    const task = tasks[i];
    if (!task) continue;
    const last = spans[spans.length - 1];
    if (last && last.sectionIndex === task.section) {
      last.endCol = i;
      continue;
    }
    const name = sectionNames[task.section] ?? "";
    const colorIndex = task.section === 0 ? -1 : namedCounter++;
    spans.push({
      sectionIndex: task.section,
      name,
      startCol: i,
      endCol: i,
      colorIndex,
    });
  }
  return spans;
}

function layoutLegend(
  actors: readonly string[],
  maxRowWidth: number,
  budget: StepBudget,
): { items: { name: string; index: number; x: number; y: number; w: number }[]; height: number } {
  if (actors.length === 0) return { items: [], height: 0 };
  const style = { size: FONT.small };
  const items: { name: string; index: number; x: number; y: number; w: number }[] = [];
  let cx = 0;
  let row = 0;
  const rowH = 24;
  for (let i = 0; i < actors.length; i++) {
    budget.tick();
    const label = fitWidth(clampLabel(actors[i] ?? ""), 150, style);
    const pillW = Math.ceil(24 + textWidth(label, style));
    if (cx > 0 && cx + pillW > maxRowWidth) {
      cx = 0;
      row++;
    }
    items.push({ name: label, index: i, x: cx, y: row * rowH, w: pillW });
    cx += pillW + 8;
  }
  return { items, height: (row + 1) * rowH + 6 };
}

/** Draw a parsed user journey diagram as a complete SVG string. */
export function drawJourney(model: JourneyModel): string {
  const budget = new StepBudget();
  const titleStyle = { size: FONT.title, bold: true };
  const bodyStyle = { size: FONT.body };
  const sectionStyle = { size: FONT.body, bold: true };

  // Compute uniform column width clamped so even 120 tasks stay within MAX_SVG_SIDE.
  const maxForBudget = Math.floor(
    (15_200 - PAD * 2 - AXIS_W) / Math.max(1, model.tasks.length) - COL_GAP,
  );
  const colMax = Math.max(COL_MIN, Math.min(188, maxForBudget));
  let maxWordW = 0;
  for (const task of model.tasks) {
    budget.tick();
    maxWordW = Math.max(maxWordW, textWidth(clampLabel(task.title), bodyStyle) + 18);
  }
  const colW = Math.min(colMax, Math.max(COL_MIN, Math.ceil(maxWordW)));
  const startX = PAD + AXIS_W;
  const colsTotalW = model.tasks.length * colW + Math.max(0, model.tasks.length - 1) * COL_GAP;
  const titleText = model.title
    ? fitWidth(clampLabel(model.title), Math.max(460, colsTotalW), titleStyle)
    : "";
  const titleW = titleText ? textWidth(titleText, titleStyle) + PAD * 2 : 0;
  const minContentW = Math.max(360, colsTotalW);
  const width = Math.max(startX + minContentW + PAD, titleW);

  const legend = layoutLegend(model.actors, width - PAD * 2, budget);
  const titleH = titleText ? 34 : 0;
  const legendTop = PAD + titleH;
  const sectionTop = legendTop + legend.height;

  const spans = buildSectionSpans(
    model.tasks,
    model.sections.map((s) => s.name),
  );
  const hasNamedSections = spans.some((s) => s.sectionIndex > 0);
  const sectionWrapped = spans.map((span) => {
    budget.tick();
    const spanCols = span.endCol - span.startCol + 1;
    const spanW = spanCols * colW + (spanCols - 1) * COL_GAP;
    const lines = span.name ? wrapText(span.name, Math.max(48, spanW - 14), sectionStyle, 3) : [];
    return { span, spanW, lines };
  });
  const sectionH = hasNamedSections
    ? Math.max(
        34,
        ...sectionWrapped.map((s) =>
          s.lines.length > 0 ? measureLines(s.lines, sectionStyle, LINE_H).height + 14 : 0,
        ),
      )
    : 0;

  const taskTop = sectionTop + (sectionH > 0 ? sectionH + 10 : 0);
  const taskWrapped = model.tasks.map((task) => {
    budget.tick();
    const lines = wrapText(task.title, colW - 14, bodyStyle, 4).map((l) =>
      fitWidth(l, colW - 10, bodyStyle),
    );
    const textHeight = measureLines(lines, bodyStyle, LINE_H).height;
    const actorRows = Math.max(1, Math.ceil(task.actors.length / 5));
    const cardH = Math.max(
      48,
      Math.ceil(textHeight + 16 + (task.actors.length > 0 ? actorRows * 13 + 4 : 0)),
    );
    return { task, lines, textHeight, cardH };
  });
  const taskH = Math.max(52, ...taskWrapped.map((t) => t.cardH));

  const trackTop = taskTop + taskH + 26;
  const trackBottom = trackTop + TRACK_H;
  const height = trackBottom + PAD + 14;

  const actorIndex = new Map(model.actors.map((actor, idx) => [actor, idx]));
  const spanColorBySection = new Map<number, number>();
  for (const s of spans) {
    if (s.colorIndex >= 0 && !spanColorBySection.has(s.sectionIndex)) {
      spanColorBySection.set(s.sectionIndex, s.colorIndex);
    }
  }

  let body = "";
  if (titleText) {
    body += text(width / 2, PAD + 10, titleText, {
      size: FONT.title,
      weight: "700",
      anchor: "middle",
    });
  }

  if (legend.items.length > 0) {
    const itemsSvg = legend.items
      .map((item) => {
        const px = PAD + item.x;
        const py = legendTop + item.y;
        const dot = circle(px + 9, py + 10, 4.5, { fill: series(item.index) });
        const badge = rect(px, py, item.w, 20, {
          rx: 10,
          fill: color.groupFill,
          stroke: color.innerStroke,
          "stroke-width": 1,
        });
        const label = text(px + 17, py + 10, item.name, { size: FONT.small, fill: color.textSec });
        return badge + dot + label;
      })
      .join("");
    body += g({}, itemsSvg);
  }

  // Score grid lines (levels 5 down to 1).
  const scoreY = (score: number): number => trackTop + ((5 - score) / 4) * TRACK_H;
  for (let level = 1; level <= 5; level++) {
    const ly = scoreY(level);
    body += line(startX - 4, ly, startX + colsTotalW, ly, {
      stroke: color.innerStroke,
      "stroke-width": 1,
      "stroke-dasharray": "3 4",
    });
    body += text(startX - 10, ly, String(level), {
      size: FONT.small,
      anchor: "end",
      fill: color.textFaint,
    });
  }

  // Section banners.
  if (sectionH > 0) {
    for (const item of sectionWrapped) {
      budget.tick();
      if (item.span.colorIndex < 0) continue;
      const sx = startX + item.span.startCol * (colW + COL_GAP);
      const tone = series(item.span.colorIndex);
      body += rect(sx, sectionTop, item.spanW, sectionH, {
        rx: 6,
        fill: tone,
        "fill-opacity": 0.16,
        stroke: tone,
        "stroke-width": 1.2,
      });
      const textStartY =
        sectionTop + sectionH / 2 - ((item.lines.length - 1) * FONT.body * LINE_H) / 2;
      body += textLines(
        sx + item.spanW / 2,
        textStartY,
        item.lines,
        { size: FONT.body, weight: "600", anchor: "middle", fill: color.text },
        LINE_H,
      );
    }
  }

  // Task cards, lifelines, trend polyline, and satisfaction faces.
  const trendPoints: [number, number][] = [];
  const faces: string[] = [];
  for (let i = 0; i < taskWrapped.length; i++) {
    budget.tick();
    const item = taskWrapped[i];
    if (!item) continue;
    const x = startX + i * (colW + COL_GAP);
    const cx = x + colW / 2;
    const secColorIdx = spanColorBySection.get(item.task.section);
    const cardStroke = secColorIdx !== undefined ? series(secColorIdx) : color.nodeStroke;
    const cardFill = secColorIdx !== undefined ? series(secColorIdx) : color.nodeFill;
    body += rect(x, taskTop, colW, taskH, {
      rx: 6,
      fill: cardFill,
      "fill-opacity": secColorIdx !== undefined ? 0.1 : 1,
      stroke: cardStroke,
      "stroke-width": 1.1,
    });
    const textBaseY = taskTop + 14;
    body += textLines(
      cx,
      textBaseY,
      item.lines,
      { size: FONT.body, anchor: "middle", fill: color.text },
      LINE_H,
    );
    if (item.task.actors.length > 0) {
      const dotsStartY = taskTop + taskH - 10;
      const perRow = 5;
      item.task.actors.forEach((actor, aIdx) => {
        const rowOffset = Math.floor(aIdx / perRow);
        const colInRow = aIdx % perRow;
        const rowCount = Math.min(perRow, item.task.actors.length - rowOffset * perRow);
        const dotX = cx + (colInRow - (rowCount - 1) / 2) * 14;
        const dotY = dotsStartY - rowOffset * 12;
        const colorIdx = actorIndex.get(actor) ?? 0;
        body += circle(dotX, dotY, 4.5, {
          fill: series(colorIdx),
          stroke: color.canvas,
          "stroke-width": 1.2,
        });
      });
    }
    const sy = scoreY(item.task.score);
    body += line(cx, taskTop + taskH, cx, trackBottom + 4, {
      stroke: color.innerStroke,
      "stroke-width": 1,
      "stroke-dasharray": "3 3",
    });
    trendPoints.push([cx, sy]);
    faces.push(drawFace(cx, sy, item.task.score));
  }

  if (trendPoints.length >= 2) {
    body += polyline(trendPoints, {
      stroke: color.line,
      "stroke-width": 1.5,
      "stroke-linejoin": "round",
      "stroke-linecap": "round",
    });
  }
  body += faces.join("");

  return svgDoc(width, height, g({}, body));
}
