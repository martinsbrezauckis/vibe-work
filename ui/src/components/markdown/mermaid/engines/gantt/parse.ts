/**
 * Gantt parser. Grammar follows Mermaid's gantt page: `dateFormat`, `axisFormat`, `tickInterval`,
 * `excludes`, `title`, `section`, and tasks `Title : [tags,] [id,] [start,] [end|duration]` where
 * tags are done/active/crit/milestone, start is a date or `after id [id...]` and the end is a
 * date, a duration or `until id`. Dates resolve to UTC; dependency cycles and unknown ids are errors.
 * `todayMarker`, `weekend` and `inclusiveEndDates` lines are read and have no effect on the drawing.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";
import { DAY, WEEKDAYS, addDuration, compileDateFormat, inRange, parseDuration } from "./time";

/** A task with resolved UTC start and end, status flags and the ids it depends on. */
export interface GanttTask {
  title: string;
  section: number;
  id: string;
  tags: string[];
  status: "done" | "active" | "crit" | "none";
  done: boolean;
  active: boolean;
  crit: boolean;
  milestone: boolean;
  startMs: number;
  endMs: number;
  /** Ids this task starts after or ends at. */
  dependencies: string[];
}
/** A parsed gantt chart with its settings, sections and resolved tasks. */
export interface GanttModel {
  title: string;
  dateFormat: string;
  axisFormat: string;
  tickInterval: { count: number; unit: string } | undefined;
  weekday: number;
  excludes: string[];
  sections: { name: string }[];
  tasks: GanttTask[];
}

type StartSpec = { kind: "date"; ms: number } | { kind: "after"; ids: string[] } | { kind: "prev" };
type EndSpec =
  | { kind: "date"; ms: number }
  | { kind: "duration"; amount: number; unit: string }
  | { kind: "until"; ids: string[] };
interface RawTask {
  title: string;
  section: number;
  id: string | undefined;
  tags: string[];
  start: StartSpec;
  end: EndSpec;
}

const TAGS = new Set(["done", "active", "crit", "milestone"]);
const EXCLUDE_DATE = /^\d{4}-\d{2}-\d{2}$/;
// Task dates are limited to 1900-2199, so longer tick intervals cannot add useful axis detail.
const MAX_TICK_SPAN_MS = 300 * 366 * DAY;
const TICK_UNIT_MS: Record<string, number> = {
  millisecond: 1,
  second: 1000,
  minute: 60_000,
  hour: 3_600_000,
  day: DAY,
  week: 7 * DAY,
};

/** Parse `gantt` source (text or prepared lines) and resolve every task to UTC times; throws DiagramInputError on anything unreadable, circular or out of range. */
export function parseGantt(input: string | PreparedSource): GanttModel {
  const [, ...body] = linesOf(input, "gantt");
  const model: GanttModel = {
    title: "",
    dateFormat: "YYYY-MM-DD",
    axisFormat: "%Y-%m-%d",
    tickInterval: undefined,
    weekday: 0,
    excludes: [],
    sections: [{ name: "" }],
    tasks: [],
  };
  let parseDate = compileDateFormat(model.dateFormat);
  const raws: RawTask[] = [];

  for (const raw of body) {
    const line = raw.trim().replace(/;$/, "").trim();
    let match: RegExpExecArray | null;
    if ((match = /^title\s+(.+)$/i.exec(line))) model.title = (match[1] ?? "").trim();
    else if ((match = /^dateFormat\s+(.+)$/i.exec(line))) {
      model.dateFormat = (match[1] ?? "").trim();
      parseDate = compileDateFormat(model.dateFormat);
    } else if ((match = /^axisFormat\s+(.+)$/i.exec(line)))
      model.axisFormat = (match[1] ?? "").trim();
    else if (
      (match = /^tickInterval\s+(\d+)(millisecond|second|minute|hour|day|week|month)$/i.exec(line))
    ) {
      const count = Number(match[1]);
      const unit = (match[2] ?? "").toLowerCase();
      if (!Number.isFinite(count))
        throw new DiagramInputError("tickInterval count is not finite; use a smaller whole number");
      const max =
        unit === "month" ? 300 * 12 : Math.floor(MAX_TICK_SPAN_MS / (TICK_UNIT_MS[unit] ?? DAY));
      if (!Number.isSafeInteger(count) || count < 1 || count > max)
        throw new DiagramInputError(
          `tickInterval count must be 1 to ${String(max)} ${unit}s; use a smaller count`,
        );
      model.tickInterval = { count, unit };
    } else if ((match = /^weekday\s+(\w+)$/i.exec(line))) {
      const day = WEEKDAYS.indexOf((match[1] ?? "").toLowerCase());
      if (day < 0) throw new DiagramInputError("Unknown weekday");
      model.weekday = day;
    } else if ((match = /^excludes\s+(.+)$/i.exec(line))) {
      for (const part of (match[1] ?? "").split(/[\s,]+/).filter(Boolean)) {
        const item = part.toLowerCase();
        if (item !== "weekends" && !WEEKDAYS.includes(item) && !EXCLUDE_DATE.test(item)) {
          throw new DiagramInputError(`Cannot exclude: ${part.slice(0, 30)}`);
        }
        model.excludes.push(item);
      }
      checkCount(model.excludes.length, MAX_EDGES, "excluded days");
    } else if (
      /^(?:todayMarker|weekend|inclusiveEndDates|topAxis|acc(?:Title|Descr))\b/i.test(line)
    )
      continue;
    else if ((match = /^section\s+(.+)$/i.exec(line))) {
      model.sections.push({ name: (match[1] ?? "").trim() });
      checkCount(model.sections.length, MAX_NODES, "sections");
    } else if (line.includes(":")) {
      const cut = line.indexOf(":");
      const title = line.slice(0, cut).trim();
      const items = line
        .slice(cut + 1)
        .split(",")
        .map((item) => item.trim());
      if (!title || items.every((item) => item === ""))
        throw new DiagramInputError(`Task without a title or data: ${line.slice(0, 40)}`);
      const tags: string[] = [];
      while (items.length > 0 && TAGS.has((items[0] ?? "").toLowerCase()))
        tags.push((items.shift() ?? "").toLowerCase());
      const date = (text: string): number | undefined => parseDate(text);
      const startOf = (text: string): StartSpec => {
        const after = /^after\s+(.+)$/i.exec(text);
        if (after) return { kind: "after", ids: (after[1] ?? "").split(/\s+/).filter(Boolean) };
        const ms = date(text);
        if (ms === undefined)
          throw new DiagramInputError(
            `Not a start date in ${model.dateFormat}: ${text.slice(0, 30)}`,
          );
        return { kind: "date", ms };
      };
      const endOf = (text: string): EndSpec => {
        const until = /^until\s+(.+)$/i.exec(text);
        if (until) return { kind: "until", ids: (until[1] ?? "").split(/\s+/).filter(Boolean) };
        const duration = parseDuration(text);
        if (duration) return { kind: "duration", ...duration };
        const ms = date(text);
        if (ms === undefined)
          throw new DiagramInputError(`Not an end date or duration: ${text.slice(0, 30)}`);
        return { kind: "date", ms };
      };
      let id: string | undefined;
      let start: StartSpec = { kind: "prev" };
      let end: EndSpec;
      if (items.length === 1) end = endOf(items[0] ?? "");
      else if (items.length === 2) {
        start = startOf(items[0] ?? "");
        end = endOf(items[1] ?? "");
      } else if (items.length === 3) {
        id = items[0] ?? "";
        if (!/^[^\s,]+$/.test(id)) throw new DiagramInputError("Bad task id");
        start = startOf(items[1] ?? "");
        end = endOf(items[2] ?? "");
      } else
        throw new DiagramInputError(`Task needs 1 to 3 items after the tags: ${line.slice(0, 40)}`);
      raws.push({ title, section: model.sections.length - 1, id, tags, start, end });
      checkCount(raws.length, MAX_NODES, "tasks");
    } else throw new DiagramInputError(`Unreadable gantt line: ${line.slice(0, 40)}`);
  }
  if (raws.length === 0) throw new DiagramInputError("A gantt chart needs tasks");
  const named = new Map<string, number>();
  raws.forEach((task, i) => {
    if (task.id === undefined) return;
    if (named.has(task.id)) throw new DiagramInputError(`Duplicate task id: ${task.id}`);
    named.set(task.id, i);
  });

  const excludedWeekdays = new Set<number>();
  const excludedDates = new Set<number>();
  for (const item of model.excludes) {
    if (item === "weekends") {
      excludedWeekdays.add(0);
      excludedWeekdays.add(6);
    } else if (WEEKDAYS.includes(item)) excludedWeekdays.add(WEEKDAYS.indexOf(item));
    else excludedDates.add(Date.parse(`${item}T00:00:00Z`));
  }
  const excluded = (ms: number): boolean => {
    const day = Math.floor(ms / DAY) * DAY;
    return excludedWeekdays.has(new Date(day).getUTCDay()) || excludedDates.has(day);
  };

  const resolved: ({ startMs: number; endMs: number } | undefined)[] = raws.map(() => undefined);
  const visiting = new Set<number>();
  let steps = 0;
  const resolve = (index: number): { startMs: number; endMs: number } => {
    const done = resolved[index];
    if (done) return done;
    if (visiting.has(index)) throw new DiagramInputError("Tasks depend on each other in a circle");
    visiting.add(index);
    const task = raws[index] as RawTask;
    const ref = (ids: string[]): { startMs: number; endMs: number }[] =>
      ids.map((id) => {
        const at = named.get(id);
        if (at === undefined) throw new DiagramInputError(`Unknown task id: ${id.slice(0, 30)}`);
        return resolve(at);
      });
    let startMs: number;
    if (task.start.kind === "date") startMs = task.start.ms;
    else if (task.start.kind === "after")
      startMs = Math.max(...ref(task.start.ids).map((r) => r.endMs));
    else if (index === 0) throw new DiagramInputError("The first task needs a start date");
    else startMs = resolve(index - 1).endMs;
    let endMs: number;
    if (task.end.kind === "date") endMs = task.end.ms;
    else if (task.end.kind === "until")
      endMs = Math.min(...ref(task.end.ids).map((r) => r.startMs));
    else {
      endMs = addDuration(startMs, task.end);
      if (task.end.unit === "d" || task.end.unit === "w") {
        for (let day = Math.floor(startMs / DAY) * DAY; day < endMs; day += DAY) {
          if (++steps > 200_000)
            throw new DiagramInputError("Diagram too large: too many days to count");
          if (excluded(day)) endMs += DAY;
        }
      }
    }
    if (!inRange(startMs) || !inRange(endMs))
      throw new DiagramInputError("Dates must fall between 1900 and 2199");
    if (endMs < startMs)
      throw new DiagramInputError(`Task ends before it starts: ${task.title.slice(0, 30)}`);
    visiting.delete(index);
    const out = { startMs, endMs };
    resolved[index] = out;
    return out;
  };

  raws.forEach((task, index) => {
    const times = resolve(index);
    const dependencies = [
      ...(task.start.kind === "after" ? task.start.ids : []),
      ...(task.end.kind === "until" ? task.end.ids : []),
    ];
    checkCount(model.tasks.length + dependencies.length, MAX_EDGES, "dependencies");
    const flags = {
      done: task.tags.includes("done"),
      active: task.tags.includes("active"),
      crit: task.tags.includes("crit"),
      milestone: task.tags.includes("milestone"),
    };
    model.tasks.push({
      title: task.title,
      section: task.section,
      id: task.id ?? `task${String(index + 1)}`,
      tags: task.tags,
      status: flags.done ? "done" : flags.active ? "active" : flags.crit ? "crit" : "none",
      ...flags,
      ...times,
      dependencies,
    });
  });
  return model;
}
