/**
 * Kanban board drawing: multi-column panels with category header stripes, stacked task cards,
 * priority accent bars, wrapped labels, and passive pill badges for ticket/assignee/metadata.
 */
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import { g, rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { KanbanModel, KanbanPriority, KanbanTask } from "./parse";

const PAD = 24;
const COL_W = 232;
const COL_GAP = 14;
const ROW_GAP = 20;
const CARD_PAD_X = 12;
const CARD_PAD_Y = 10;
const CARD_GAP = 10;
const LINE_H = 1.3;
const MAX_COLS_PER_ROW = 60;

interface PillSpec {
  label: string;
  accent: string;
  filled: boolean;
}

interface LaidTask {
  task: KanbanTask;
  lines: string[];
  textH: number;
  pills: { label: string; accent: string; filled: boolean; x: number; y: number; w: number }[];
  pillAreaH: number;
  height: number;
}

function priorityColor(priority: KanbanPriority): string {
  if (priority === "Very High") return color.crit;
  if (priority === "High") return color.accent;
  if (priority === "Low") return color.info;
  return color.textMuted;
}

function pillsOf(task: KanbanTask): PillSpec[] {
  const pills: PillSpec[] = [];
  if (task.ticket)
    pills.push({ label: clampLabel(task.ticket, 48), accent: color.info, filled: true });
  if (task.priority)
    pills.push({ label: task.priority, accent: priorityColor(task.priority), filled: true });
  if (task.assigned) {
    pills.push({
      label: `@${clampLabel(task.assigned, 48)}`,
      accent: color.innerStroke,
      filled: false,
    });
  }
  for (const extra of task.extras) {
    pills.push({
      label: clampLabel(`${extra.key}: ${extra.value}`, 60),
      accent: color.innerStroke,
      filled: false,
    });
  }
  return pills;
}

function layoutTask(task: KanbanTask, cardW: number, budget: StepBudget): LaidTask {
  const bodyStyle = { size: FONT.body };
  const pillStyle = { size: FONT.small };
  const hasBar = task.priority !== undefined;
  const innerLeft = hasBar ? 14 : CARD_PAD_X;
  const maxTextW = cardW - innerLeft - CARD_PAD_X;
  const lines = wrapText(task.title, maxTextW, bodyStyle, 8).map((l) =>
    fitWidth(l, maxTextW, bodyStyle),
  );
  const textH = measureLines(lines, bodyStyle, LINE_H).height;

  const specs = pillsOf(task);
  const laidPills: LaidTask["pills"] = [];
  let px = 0;
  let py = 0;
  const pillH = 18;
  for (const spec of specs) {
    budget.tick();
    const label = fitWidth(spec.label, maxTextW - 14, pillStyle);
    const pw = Math.ceil(textWidth(label, pillStyle) + 14);
    if (px > 0 && px + pw > maxTextW) {
      px = 0;
      py += pillH + 5;
    }
    laidPills.push({
      label,
      accent: spec.accent,
      filled: spec.filled,
      x: px,
      y: py,
      w: pw,
    });
    px += pw + 6;
  }
  const pillAreaH = laidPills.length > 0 ? py + pillH + 6 : 0;
  const height = Math.max(40, Math.ceil(CARD_PAD_Y * 2 + textH + pillAreaH));
  return { task, lines, textH, pills: laidPills, pillAreaH, height };
}

/** Draw a parsed kanban board as a complete SVG string. */
export function drawKanban(model: KanbanModel): string {
  const budget = new StepBudget();
  const hdrStyle = { size: FONT.body, bold: true };
  const countStyle = { size: FONT.small, bold: true };
  const cardW = COL_W - 20;

  const laidCols = model.columns.map((col, idx) => {
    budget.tick();
    const countLabel = String(col.tasks.length);
    const countW = Math.ceil(textWidth(countLabel, countStyle) + 14);
    const hdrLines = wrapText(col.title, COL_W - 28 - countW, hdrStyle, 3).map((l) =>
      fitWidth(l, COL_W - 24 - countW, hdrStyle),
    );
    const hdrH = Math.max(36, Math.ceil(measureLines(hdrLines, hdrStyle, LINE_H).height + 16));
    const tasks = col.tasks.map((t) => layoutTask(t, cardW, budget));
    const tasksH =
      tasks.reduce((sum, t) => sum + t.height, 0) + Math.max(0, tasks.length - 1) * CARD_GAP;
    const minBodyH = tasks.length > 0 ? tasksH + 22 : 62;
    return { col, idx, countLabel, countW, hdrLines, hdrH, tasks, contentH: hdrH + minBodyH };
  });

  // Keep normal boards in a single horizontal row; wrap extreme column counts (>60) cleanly.
  const rows: (typeof laidCols)[] = [];
  for (let i = 0; i < laidCols.length; i += MAX_COLS_PER_ROW) {
    rows.push(laidCols.slice(i, i + MAX_COLS_PER_ROW));
  }

  const maxRowCols = Math.max(1, ...rows.map((r) => r.length));
  const width = PAD * 2 + maxRowCols * COL_W + Math.max(0, maxRowCols - 1) * COL_GAP;

  let cursorY = PAD;
  let body = "";

  for (const rowCols of rows) {
    budget.tick();
    const rowColH = Math.max(110, ...rowCols.map((c) => c.contentH));
    rowCols.forEach((col, cPos) => {
      budget.tick();
      const x = PAD + cPos * (COL_W + COL_GAP);
      const y = cursorY;
      const tone = series(col.idx);
      body += rect(x, y, COL_W, rowColH, {
        rx: 8,
        fill: color.groupFill,
        stroke: color.innerStroke,
        "stroke-width": 1.1,
      });
      body += rect(x, y, COL_W, col.hdrH, {
        rx: 8,
        fill: tone,
        "fill-opacity": 0.16,
        stroke: tone,
        "stroke-width": 1.2,
      });
      const titleY = y + col.hdrH / 2 - ((col.hdrLines.length - 1) * FONT.body * LINE_H) / 2;
      body += textLines(
        x + 12,
        titleY,
        col.hdrLines,
        { size: FONT.body, weight: "600", fill: color.text },
        LINE_H,
      );
      const badgeX = x + COL_W - 10 - col.countW;
      const badgeY = y + (col.hdrH - 18) / 2;
      body += rect(badgeX, badgeY, col.countW, 18, {
        rx: 9,
        fill: tone,
        "fill-opacity": 0.22,
      });
      body += text(badgeX + col.countW / 2, badgeY + 9, col.countLabel, {
        size: FONT.small,
        weight: "600",
        anchor: "middle",
        fill: color.text,
      });

      let cardY = y + col.hdrH + 11;
      const cardX = x + 10;
      for (const laid of col.tasks) {
        budget.tick();
        const hasBar = laid.task.priority !== undefined;
        const innerLeft = cardX + (hasBar ? 14 : CARD_PAD_X);
        body += rect(cardX, cardY, cardW, laid.height, {
          rx: 6,
          fill: color.nodeFill,
          stroke: color.nodeStroke,
          "stroke-width": 1,
        });
        if (laid.task.priority) {
          body += rect(cardX, cardY, 4, laid.height, {
            rx: 2,
            fill: priorityColor(laid.task.priority),
          });
        }
        body += textLines(
          innerLeft,
          cardY + CARD_PAD_Y + (FONT.body * LINE_H) / 2,
          laid.lines,
          { size: FONT.body, fill: color.text },
          LINE_H,
        );

        const pillsTop = cardY + CARD_PAD_Y + laid.textH + 6;
        for (const pill of laid.pills) {
          budget.tick();
          const px = innerLeft + pill.x;
          const py = pillsTop + pill.y;
          body += rect(px, py, pill.w, 18, {
            rx: 4,
            fill: pill.filled ? pill.accent : color.groupHdr,
            "fill-opacity": pill.filled ? 0.18 : 1,
            stroke: pill.accent,
            "stroke-width": 0.9,
          });
          body += text(px + 7, py + 9, pill.label, {
            size: FONT.small,
            fill: pill.filled ? color.text : color.textSec,
          });
        }
        cardY += laid.height + CARD_GAP;
      }
    });
    cursorY += rowColH + ROW_GAP;
  }

  const height = cursorY - ROW_GAP + PAD;
  return svgDoc(width, height, g({}, body));
}
