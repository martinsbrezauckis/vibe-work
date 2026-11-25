/**
 * Gantt drawing: a title, a time axis with grid lines and readable tick labels, section bands, a
 * label column, status-coloured bars, diamonds for milestones, shaded excluded days and elbow
 * arrows for `after` dependencies. All times are UTC so the same source always draws the same.
 */
import { DiagramLimitError } from "../../types";
import { fitWidth, textWidth } from "../../measure";
import { el, line, markerDefs, markerRef, n, path, polygon, rect, svgDoc, text } from "../../svg";
import { FONT, color } from "../../theme";
import type { GanttModel, GanttTask } from "./parse";
import { DAY, WEEKDAYS, formatAxis } from "./time";

const CHART_W = 640;
const PAD = 20;
const ROW = 30;
const SECTION_ROW = 28;
const BAR_H = 16;
const AXIS_H = 30;
const MAX_TICKS = 200;

const UNIT_MS: Record<string, number> = {
  millisecond: 1,
  second: 1000,
  minute: 60_000,
  hour: 3_600_000,
  day: DAY,
  week: 7 * DAY,
};

type Step = { count: number; unit: string };
const AUTO_STEPS: Step[] = [
  { count: 1, unit: "hour" },
  { count: 3, unit: "hour" },
  { count: 6, unit: "hour" },
  { count: 12, unit: "hour" },
  { count: 1, unit: "day" },
  { count: 2, unit: "day" },
  { count: 1, unit: "week" },
  { count: 2, unit: "week" },
  { count: 1, unit: "month" },
  { count: 3, unit: "month" },
  { count: 6, unit: "month" },
  { count: 12, unit: "month" },
  { count: 24, unit: "month" },
  { count: 60, unit: "month" },
  { count: 120, unit: "month" },
  { count: 600, unit: "month" },
];

function stepMs(step: Step): number {
  return step.unit === "month"
    ? step.count * 30.4375 * DAY
    : step.count * (UNIT_MS[step.unit] ?? DAY);
}

/** First tick at or before `t0`, aligned to the unit in UTC. */
function floorTo(t0: number, step: Step, weekday: number): number {
  const d = new Date(t0);
  if (step.unit === "month") {
    // Multi-month steps start on multiples of the step from January, so a yearly step lands on January 1.
    const months = d.getUTCFullYear() * 12 + d.getUTCMonth();
    const aligned = Math.floor(months / step.count) * step.count;
    return Date.UTC(Math.floor(aligned / 12), aligned % 12, 1);
  }
  if (step.unit === "week") {
    const day = Math.floor(t0 / DAY) * DAY;
    return day - ((new Date(day).getUTCDay() - weekday + 7) % 7) * DAY;
  }
  const unit = UNIT_MS[step.unit] ?? DAY;
  return Math.floor(t0 / unit) * unit;
}

function ticksOf(t0: number, t1: number, step: Step, weekday: number): number[] {
  const out: number[] = [];
  let cursor = floorTo(t0, step, weekday);
  for (let count = 0; count < MAX_TICKS; count++) {
    if (!Number.isFinite(cursor))
      throw new DiagramLimitError("Invalid gantt axis tick; use a smaller tickInterval");
    if (cursor > t1) return out;
    if (cursor >= t0) out.push(cursor);
    let next: number;
    if (step.unit === "month") {
      const d = new Date(cursor);
      next = Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + step.count, 1);
    } else next = cursor + step.count * (UNIT_MS[step.unit] ?? DAY);
    if (!Number.isFinite(next) || next <= cursor)
      throw new DiagramLimitError("Invalid gantt axis tick; use a smaller tickInterval");
    cursor = next;
    if (cursor > t1) return out;
  }
  throw new DiagramLimitError(`Diagram too large: more than ${String(MAX_TICKS)} axis ticks`);
}

function taskColor(task: GanttTask): { fill: string; opacity: number } {
  const base = task.crit ? color.crit : task.active ? color.accent : color.info;
  if (task.done) return { fill: task.crit ? color.crit : color.textFaint, opacity: 0.45 };
  return { fill: base, opacity: 0.92 };
}

/** Draw a parsed gantt chart as a complete SVG string. */
export function drawGantt(model: GanttModel): string {
  const starts = model.tasks.map((t) => t.startMs);
  const ends = model.tasks.map((t) => t.endMs);
  let t0 = Math.min(...starts);
  let t1 = Math.max(...ends);
  if (t1 - t0 < 3_600_000) t1 = t0 + DAY;
  const slack = (t1 - t0) * 0.02;
  t0 -= slack;
  t1 += slack;

  const labelStyle = { size: FONT.body };
  const labelW = Math.min(
    260,
    Math.max(120, ...model.tasks.map((t) => textWidth(t.title, labelStyle) + 24)),
  );
  const left = PAD + labelW;
  const x = (ms: number): number => left + ((ms - t0) / (t1 - t0)) * CHART_W;
  const titleH = model.title ? 34 : 0;
  const axisTop = PAD + titleH;
  const bodyTop = axisTop + AXIS_H;

  // Rows.
  const rows: { y: number; task?: GanttTask; section?: string; band: number }[] = [];
  let y = bodyTop;
  model.sections.forEach((section, si) => {
    const tasks = model.tasks.filter((t) => t.section === si);
    if (tasks.length === 0 && section.name === "") return;
    if (section.name !== "") {
      rows.push({ y, section: section.name, band: si });
      y += SECTION_ROW;
    }
    for (const task of tasks) {
      rows.push({ y, task, band: si });
      y += ROW;
    }
  });
  const bodyBottom = y;
  const width = left + CHART_W + PAD;
  const height = bodyBottom + PAD;

  let out = "";
  // Section bands.
  let bandStart = bodyTop;
  let bandOf = rows[0]?.band;
  const flush = (end: number, band: number | undefined): void => {
    if (band !== undefined && band % 2 === 0 && end > bandStart) {
      out += rect(PAD / 2, bandStart, width - PAD, end - bandStart, {
        fill: color.groupFill,
        "fill-opacity": 0.55,
      });
    }
  };
  for (const row of rows) {
    if (row.band !== bandOf) {
      flush(row.y, bandOf);
      bandStart = row.y;
      bandOf = row.band;
    }
  }
  flush(bodyBottom, bandOf);

  // Excluded days, when the range is short enough to show them.
  if (model.excludes.length > 0 && t1 - t0 <= 800 * DAY) {
    const weekdays = new Set<number>();
    const dates = new Set<number>();
    for (const item of model.excludes) {
      if (item === "weekends") {
        weekdays.add(0);
        weekdays.add(6);
      } else if (WEEKDAYS.includes(item)) weekdays.add(WEEKDAYS.indexOf(item));
      else dates.add(Date.parse(`${item}T00:00:00Z`));
    }
    for (let day = Math.floor(t0 / DAY) * DAY; day < t1; day += DAY) {
      if (weekdays.has(new Date(day).getUTCDay()) || dates.has(day)) {
        const x1 = Math.max(x(day), left);
        const x2 = Math.min(x(day + DAY), left + CHART_W);
        if (x2 > x1)
          out += rect(x1, bodyTop, x2 - x1, bodyBottom - bodyTop, {
            fill: color.textFaint,
            "fill-opacity": 0.14,
          });
      }
    }
  }

  // Axis and grid.
  const auto =
    AUTO_STEPS.find((s) => (t1 - t0) / stepMs(s) <= 12) ?? AUTO_STEPS[AUTO_STEPS.length - 1];
  const step: Step = model.tickInterval ?? auto ?? { count: 1, unit: "day" };
  const ticks = ticksOf(t0, t1, step, model.weekday);
  out += line(left, axisTop + AXIS_H - 2, left + CHART_W, axisTop + AXIS_H - 2, {
    stroke: color.line,
    "stroke-width": 1,
  });
  for (const tick of ticks) {
    const tx = x(tick);
    out += line(tx, axisTop + AXIS_H - 6, tx, bodyBottom, {
      stroke: color.innerStroke,
      "stroke-width": 1,
      "stroke-dasharray": "2 4",
    });
    const label = formatAxis(tick, model.axisFormat);
    out += text(tx, axisTop + AXIS_H / 2 - 4, fitWidth(label, 90, { size: FONT.small }), {
      size: FONT.small,
      anchor: "middle",
      fill: color.textSec,
    });
  }

  // Bars, labels and dependencies.
  const rowOf = new Map<string, number>();
  const arrows: string[] = [];
  for (const row of rows) {
    if (row.task) rowOf.set(row.task.id, row.y + ROW / 2);
  }
  for (const row of rows) {
    if (row.section !== undefined) {
      out += text(
        PAD,
        row.y + SECTION_ROW / 2,
        fitWidth(row.section, labelW + CHART_W, { size: FONT.body, bold: true }),
        { size: FONT.body, weight: "700", fill: color.text },
      );
      continue;
    }
    const task = row.task;
    if (!task) continue;
    const mid = row.y + ROW / 2;
    out += text(PAD + 10, mid, fitWidth(task.title, labelW - 16, labelStyle), {
      size: FONT.body,
      fill: color.text,
    });
    const paint = taskColor(task);
    if (task.milestone) {
      const cx = x((task.startMs + task.endMs) / 2);
      out += polygon(
        [
          [cx, mid - 9],
          [cx + 9, mid],
          [cx, mid + 9],
          [cx - 9, mid],
        ],
        { fill: color.accent, stroke: color.text, "stroke-width": 1.2 },
      );
    } else {
      const x1 = x(task.startMs);
      const w = Math.max(4, x(task.endMs) - x1);
      out += rect(x1, mid - BAR_H / 2, w, BAR_H, {
        rx: 3,
        fill: paint.fill,
        "fill-opacity": paint.opacity,
        stroke: paint.fill,
        "stroke-width": 1,
      });
    }
    for (const dep of task.dependencies) {
      const depTask = model.tasks.find((t) => t.id === dep);
      const depY = rowOf.get(dep);
      if (!depTask || depY === undefined) continue;
      const x1 =
        x(depTask.milestone ? (depTask.startMs + depTask.endMs) / 2 : depTask.endMs) +
        (depTask.milestone ? 9 : 0);
      const x2 =
        x(task.milestone ? (task.startMs + task.endMs) / 2 : task.startMs) -
        (task.milestone ? 9 : 0);
      // A task that starts later than its dependency ends gets an elbow into its start; one that starts
      // right at the end gets a short drop onto its bar, so the arrowhead never points backwards.
      const step = x2 - x1 >= 16;
      const dir = mid >= depY ? 1 : -1;
      const xd = Math.min(x1, x2) + 4;
      const d = step
        ? `M${n(x1)},${n(depY)} L${n(x2 - 8)},${n(depY)} L${n(x2 - 8)},${n(mid)} L${n(x2)},${n(mid)}`
        : `M${n(xd)},${n(depY + dir * (BAR_H / 2))} L${n(xd)},${n(mid - dir * (BAR_H / 2))}`;
      arrows.push(
        path(d, {
          fill: "none",
          stroke: color.textMuted,
          "stroke-width": 1,
          "stroke-linejoin": "round",
          "marker-end": markerRef("arrow-open"),
        }),
      );
    }
  }
  out += arrows.join("");
  if (model.title) {
    out += text(
      width / 2,
      PAD + 10,
      fitWidth(model.title, width - PAD * 2, { size: FONT.title, bold: true }),
      { size: FONT.title, weight: "700", anchor: "middle" },
    );
  }
  return svgDoc(width, height, el("g", {}, out), markerDefs(["arrow-open"]));
}
