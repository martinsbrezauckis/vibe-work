/** UTC date helpers for the gantt engine: dateFormat parsing, axisFormat printing, duration arithmetic. */
import { DiagramInputError } from "../../types";

/** Milliseconds in a day. */
export const DAY = 86_400_000;
const MIN_MS = Date.UTC(1900, 0, 1);
const MAX_MS = Date.UTC(2200, 0, 1);

// Longest tokens first so `YYYY` is never read as two `YY`.
const TOKENS: [string, string][] = [
  ["YYYY", "(\\d{4})"],
  ["SSS", "(\\d{3})"],
  ["YY", "(\\d{2})"],
  ["MM", "(\\d{2})"],
  ["DD", "(\\d{2})"],
  ["HH", "(\\d{2})"],
  ["mm", "(\\d{2})"],
  ["ss", "(\\d{2})"],
  ["M", "(\\d{1,2})"],
  ["D", "(\\d{1,2})"],
  ["H", "(\\d{1,2})"],
  ["m", "(\\d{1,2})"],
  ["s", "(\\d{1,2})"],
  ["X", "(\\d{1,12})"],
  ["x", "(\\d{1,15})"],
];

/** Strict parser from text to UTC milliseconds; undefined when the text does not fit the format. */
export type DateParser = (input: string) => number | undefined;

/** Turn a dayjs-style format into a strict parser of UTC milliseconds. */
export function compileDateFormat(format: string): DateParser {
  const order: string[] = [];
  let pattern = "";
  for (let i = 0; i < format.length;) {
    const token = TOKENS.find(([t]) => format.startsWith(t, i));
    if (token) {
      order.push(token[0]);
      pattern += token[1];
      i += token[0].length;
    } else {
      const ch = format.charAt(i);
      if (/[A-Za-z]/.test(ch)) throw new DiagramInputError(`dateFormat token not supported: ${ch}`);
      pattern += ch.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
      i++;
    }
  }
  const hasYear = order.some((t) => t === "YYYY" || t === "YY" || t === "X" || t === "x");
  if (!hasYear) throw new DiagramInputError("dateFormat needs a year");
  const re = new RegExp(`^${pattern}$`);
  return (input) => {
    const m = re.exec(input.trim());
    if (!m) return undefined;
    const part = { Y: 1970, M: 1, D: 1, H: 0, m: 0, s: 0, S: 0 };
    let epoch: number | undefined;
    order.forEach((t, i) => {
      const v = Number(m[i + 1]);
      if (t === "YYYY") part.Y = v;
      else if (t === "YY") part.Y = v > 68 ? 1900 + v : 2000 + v;
      else if (t === "MM" || t === "M") part.M = v;
      else if (t === "DD" || t === "D") part.D = v;
      else if (t === "HH" || t === "H") part.H = v;
      else if (t === "mm" || t === "m") part.m = v;
      else if (t === "ss" || t === "s") part.s = v;
      else if (t === "SSS") part.S = v;
      else if (t === "X") epoch = v * 1000;
      else epoch = v;
    });
    const ms = epoch ?? Date.UTC(part.Y, part.M - 1, part.D, part.H, part.m, part.s, part.S);
    if (epoch === undefined) {
      const back = new Date(ms);
      if (
        back.getUTCFullYear() !== part.Y ||
        back.getUTCMonth() !== part.M - 1 ||
        back.getUTCDate() !== part.D ||
        part.H > 23 ||
        part.m > 59 ||
        part.s > 59
      ) {
        return undefined;
      }
    }
    return inRange(ms) ? ms : undefined;
  };
}

/** Whether a time is a finite UTC time between 1900 and 2199. */
export function inRange(ms: number): boolean {
  return Number.isFinite(ms) && ms >= MIN_MS && ms < MAX_MS;
}

const UNIT_MS: Record<string, number> = {
  ms: 1,
  s: 1000,
  m: 60_000,
  h: 3_600_000,
  d: DAY,
  w: 7 * DAY,
};

/** `3d`, `1.5h`, `2w`, `1M`, `1y`: amount and unit, or undefined. */
export function parseDuration(text: string): { amount: number; unit: string } | undefined {
  const m = /^(\d{1,6}(?:\.\d{1,3})?)(ms|s|m|h|d|w|M|y)$/.exec(text.trim());
  return m ? { amount: Number(m[1]), unit: m[2] ?? "d" } : undefined;
}

/** Add months or years on the calendar (UTC), clamping the day. */
function addCalendar(ms: number, months: number): number {
  const d = new Date(ms);
  const target = d.getUTCFullYear() * 12 + d.getUTCMonth() + months;
  const year = Math.floor(target / 12);
  const month = target - year * 12;
  const last = new Date(Date.UTC(year, month + 1, 0)).getUTCDate();
  return Date.UTC(
    year,
    month,
    Math.min(d.getUTCDate(), last),
    d.getUTCHours(),
    d.getUTCMinutes(),
    d.getUTCSeconds(),
    d.getUTCMilliseconds(),
  );
}

/** Add a duration to a time; months and years follow the calendar. */
export function addDuration(start: number, duration: { amount: number; unit: string }): number {
  if (duration.unit === "M") return addCalendar(start, Math.round(duration.amount));
  if (duration.unit === "y") return addCalendar(start, Math.round(duration.amount * 12));
  return start + duration.amount * (UNIT_MS[duration.unit] ?? DAY);
}

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const two = (v: number): string => String(v).padStart(2, "0");

/** d3-style strftime subset; unknown directives print as written. */
export function formatAxis(ms: number, format: string): string {
  const d = new Date(ms);
  return format.replace(/%([a-zA-Z%])/g, (whole, code: string) => {
    switch (code) {
      case "Y":
        return String(d.getUTCFullYear());
      case "y":
        return two(d.getUTCFullYear() % 100);
      case "m":
        return two(d.getUTCMonth() + 1);
      case "d":
        return two(d.getUTCDate());
      case "e":
        return String(d.getUTCDate()).padStart(2, " ").trim();
      case "b":
        return (MONTHS[d.getUTCMonth()] ?? "").slice(0, 3);
      case "B":
        return MONTHS[d.getUTCMonth()] ?? "";
      case "a":
        return (DAYS[d.getUTCDay()] ?? "").slice(0, 3);
      case "A":
        return DAYS[d.getUTCDay()] ?? "";
      case "H":
        return two(d.getUTCHours());
      case "I":
        return two(d.getUTCHours() % 12 || 12);
      case "M":
        return two(d.getUTCMinutes());
      case "S":
        return two(d.getUTCSeconds());
      case "p":
        return d.getUTCHours() < 12 ? "AM" : "PM";
      case "%":
        return "%";
      default:
        return whole;
    }
  });
}

/** Lower-case weekday names, Sunday first. */
export const WEEKDAYS = [
  "sunday",
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
];
