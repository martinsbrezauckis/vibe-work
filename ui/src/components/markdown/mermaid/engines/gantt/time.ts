/** UTC date helpers for the gantt engine: dateFormat parsing, axisFormat printing, duration arithmetic. */
import { DiagramInputError } from "../../types";

/** Milliseconds in a day. */
export const DAY = 86_400_000;
const MIN_MS = Date.UTC(1900, 0, 1);
const MAX_MS = Date.UTC(2200, 0, 1);

// Longest tokens first so `YYYY` is never read as two `YY`.
const TOKENS: [string, number, number][] = [
  ["YYYY", 4, 4],
  ["SSS", 3, 3],
  ["YY", 2, 2],
  ["MM", 2, 2],
  ["DD", 2, 2],
  ["HH", 2, 2],
  ["mm", 2, 2],
  ["ss", 2, 2],
  ["M", 1, 2],
  ["D", 1, 2],
  ["H", 1, 2],
  ["m", 1, 2],
  ["s", 1, 2],
  ["X", 1, 12],
  ["x", 1, 15],
];

type FormatPart =
  | { kind: "digits"; token: string; min: number; max: number }
  | { kind: "literal"; value: string; min: 1; max: 1 };

/** Match fixed literals and greedy digit captures without regex backtracking. */
function matchDateParts(parts: readonly FormatPart[], input: string): number[] | undefined {
  const length = input.length;
  const minSuffix = new Array<number>(parts.length + 1).fill(0);
  const maxSuffix = new Array<number>(parts.length + 1).fill(0);
  for (let p = parts.length - 1; p >= 0; p--) {
    const part = parts[p];
    if (!part) continue;
    minSuffix[p] = (minSuffix[p + 1] ?? 0) + part.min;
    maxSuffix[p] = (maxSuffix[p + 1] ?? 0) + part.max;
  }
  if (length < (minSuffix[0] ?? 0) || length > (maxSuffix[0] ?? 0)) return undefined;

  // A date can contain only ASCII digits and the literal characters named by
  // its format. Rejecting an impossible character before state matching also
  // avoids exploring every ambiguous numeric-literal split on a bad suffix.
  const literals = new Set<string>();
  for (const part of parts) if (part.kind === "literal") literals.add(part.value);
  for (let i = 0; i < length; i++) {
    const code = input.charCodeAt(i);
    if (code < 48 || code > 57) {
      if (!literals.has(input[i] ?? "")) return undefined;
    }
  }

  // A run made only of digit tokens has every intermediate width from its
  // minimum to maximum. Reserve enough digits for the suffix, then give each
  // capture the longest width the old greedy regex would choose.
  if (parts.every((part) => part.kind === "digits")) {
    const values: number[] = [];
    let cursor = 0;
    for (let p = 0; p < parts.length; p++) {
      const part = parts[p];
      if (!part) continue;
      const width = Math.min(part.max, length - cursor - (minSuffix[p + 1] ?? 0));
      if (width < part.min || length - cursor - width > (maxSuffix[p + 1] ?? 0)) return undefined;
      values.push(Number(input.slice(cursor, cursor + width)));
      cursor += width;
    }
    return values;
  }

  // A row records which input offsets can match the remaining format. Bitsets keep
  // even a maximal source bounded in memory, and each digit width is at most 15.
  const words = Math.ceil((length + 1) / 32);
  const rows: Uint32Array[] = [];
  rows.length = parts.length + 1;
  const last = new Uint32Array(words);
  last[length >>> 5] = 1 << (length & 31);
  rows[parts.length] = last;
  const reachable = (row: Uint32Array, at: number): boolean =>
    ((row[at >>> 5] ?? 0) & (1 << (at & 31))) !== 0;
  const shifted = (row: Uint32Array, word: number, width: number): number =>
    ((row[word] ?? 0) >>> width) | ((row[word + 1] ?? 0) << (32 - width));
  const digitMasks: Uint32Array[] = [];
  const oneDigit = new Uint32Array(words);
  const literalMasks = new Map<string, Uint32Array>();
  for (const value of literals) literalMasks.set(value, new Uint32Array(words));
  for (let i = 0; i < length; i++) {
    const code = input.charCodeAt(i);
    if (code >= 48 && code <= 57) oneDigit[i >>> 5] = (oneDigit[i >>> 5] ?? 0) | (1 << (i & 31));
    const literalMask = literalMasks.get(input[i] ?? "");
    if (literalMask) literalMask[i >>> 5] = (literalMask[i >>> 5] ?? 0) | (1 << (i & 31));
  }
  digitMasks[1] = oneDigit;
  const maxWidth = Math.max(1, ...parts.map((part) => part.max));
  for (let width = 2; width <= maxWidth; width++) {
    const mask = new Uint32Array(words);
    const previous = digitMasks[width - 1];
    if (!previous) continue;
    for (let word = 0; word < words; word++)
      mask[word] = (previous[word] ?? 0) & shifted(oneDigit, word, width - 1);
    digitMasks[width] = mask;
  }

  for (let p = parts.length - 1; p >= 0; p--) {
    const part = parts[p];
    const next = rows[p + 1];
    if (!part || !next) continue;
    const row = new Uint32Array(words);
    const firstWord = Math.floor(Math.max(0, length - (maxSuffix[p] ?? 0)) / 32);
    const lastWord = Math.floor(Math.min(length, length - (minSuffix[p] ?? 0)) / 32);
    if (part.kind === "literal") {
      const mask = literalMasks.get(part.value);
      if (!mask) continue;
      for (let word = firstWord; word <= lastWord; word++)
        row[word] = shifted(next, word, 1) & (mask[word] ?? 0);
    } else {
      for (let width = part.min; width <= part.max; width++) {
        const mask = digitMasks[width];
        if (!mask) continue;
        for (let word = firstWord; word <= lastWord; word++)
          row[word] = (row[word] ?? 0) | (shifted(next, word, width) & (mask[word] ?? 0));
      }
    }
    rows[p] = row;
  }
  const first = rows[0];
  if (!first || !reachable(first, 0)) return undefined;

  const values: number[] = [];
  let cursor = 0;
  for (let p = 0; p < parts.length; p++) {
    const part = parts[p];
    if (!part) continue;
    if (part.kind === "literal") {
      cursor++;
      continue;
    }
    const next = rows[p + 1];
    if (!next) return undefined;
    // Try longer captures first, as the original greedy regex did.
    for (let width = part.max; width >= part.min; width--) {
      const mask = digitMasks[width];
      if (cursor + width > length || !mask || !reachable(mask, cursor)) continue;
      if (!reachable(next, cursor + width)) continue;
      values.push(Number(input.slice(cursor, cursor + width)));
      cursor += width;
      break;
    }
  }
  return values;
}

/** Strict parser from text to UTC milliseconds; undefined when the text does not fit the format. */
export type DateParser = (input: string) => number | undefined;

/** Turn a dayjs-style format into a strict parser of UTC milliseconds. */
export function compileDateFormat(format: string): DateParser {
  const order: string[] = [];
  const parts: FormatPart[] = [];
  for (let i = 0; i < format.length;) {
    const token = TOKENS.find(([t]) => format.startsWith(t, i));
    if (token) {
      order.push(token[0]);
      parts.push({ kind: "digits", token: token[0], min: token[1], max: token[2] });
      i += token[0].length;
    } else {
      const ch = format.charAt(i);
      if (/[A-Za-z]/.test(ch)) throw new DiagramInputError(`dateFormat token not supported: ${ch}`);
      parts.push({ kind: "literal", value: ch, min: 1, max: 1 });
      i++;
    }
  }
  const hasYear = order.some((t) => t === "YYYY" || t === "YY" || t === "X" || t === "x");
  if (!hasYear) throw new DiagramInputError("dateFormat needs a year");
  return (input) => {
    const values = matchDateParts(parts, input.trim());
    if (!values) return undefined;
    const part = { Y: 1970, M: 1, D: 1, H: 0, m: 0, s: 0, S: 0 };
    let epoch: number | undefined;
    order.forEach((t, i) => {
      const v = values[i] ?? NaN;
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
