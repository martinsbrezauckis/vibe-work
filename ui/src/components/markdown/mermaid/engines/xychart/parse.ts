/**
 * XY chart parser. Grammar follows Mermaid's xyChart page: `xychart[-beta] [vertical|horizontal]`,
 * optional `title`, `x-axis` (categorical `[...]` or numeric `min --> max` or title-only), `y-axis`
 * (numeric `min --> max` or title-only), and `bar`/`line` series with optional legend names and
 * per-point `"label"` strings (drawn on lines, accepted/ignored on bars). Directives and `:::` refused.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** Chart direction: vertical (default) or horizontal. */
export type XyOrientation = "vertical" | "horizontal";

/** X-axis configuration: categorical labels, explicit numeric range, or auto-generated slots. */
export type XyXAxis =
  | { kind: "categorical"; title: string; categories: string[] }
  | { kind: "numeric"; title: string; min: number; max: number }
  | { kind: "auto"; title: string };

/** Y-axis configuration: explicit numeric range or auto-computed from data. */
export type XyYAxis =
  { kind: "numeric"; title: string; min: number; max: number } | { kind: "auto"; title: string };

/** One plotted value with an optional per-point label (rendered on `line` series). */
export interface XyPoint {
  value: number;
  label?: string;
}

/** One bar or line series in source order. */
export interface XySeries {
  kind: "bar" | "line";
  name?: string;
  points: XyPoint[];
}

/** A parsed XY chart ready for vertical or horizontal rendering. */
export interface XyChartModel {
  orientation: XyOrientation;
  title: string;
  xAxis: XyXAxis;
  yAxis: XyYAxis;
  series: XySeries[];
}

const NUMBER = /^[+-]?(?:\d+(?:\.\d+)?|\.\d+)$/;
const REFUSED = /^(?:classDef|cssClass|style|click|link|callback)\b/i;

function readQuoted(raw: string, start: number): { value: string; next: number } {
  let escaped = false;
  for (let i = start + 1; i < raw.length; i++) {
    const ch = raw[i];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (ch === "\\") {
      escaped = true;
      continue;
    }
    if (ch === '"') {
      return {
        value: raw
          .slice(start + 1, i)
          .replace(/\\(.)/g, "$1")
          .trim(),
        next: i + 1,
      };
    }
  }
  throw new DiagramInputError("Unterminated quoted string in xychart");
}

function parseOptionalText(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return "";
  if (trimmed.startsWith('"')) {
    const { value, next } = readQuoted(trimmed, 0);
    if (trimmed.slice(next).trim() !== "") {
      throw new DiagramInputError(`Unexpected trailing text after quote: ${trimmed.slice(next)}`);
    }
    return value;
  }
  return trimmed;
}

function findOutsideQuotes(raw: string, needle: string): number {
  let inQuotes = false;
  let escaped = false;
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (ch === "\\" && inQuotes) {
      escaped = true;
      continue;
    }
    if (!inQuotes && raw.startsWith(needle, i)) return i;
    if (ch === '"') {
      inQuotes = !inQuotes;
    }
  }
  if (inQuotes) throw new DiagramInputError("Unterminated quoted string in xychart");
  return -1;
}

function splitBracketItems(raw: string): string[] {
  const items: string[] = [];
  let inQuotes = false;
  let escaped = false;
  let start = 0;
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (ch === "\\" && inQuotes) {
      escaped = true;
      continue;
    }
    if (ch === '"') {
      inQuotes = !inQuotes;
      continue;
    }
    if (!inQuotes && ch === ",") {
      const item = raw.slice(start, i).trim();
      if (!item) throw new DiagramInputError("Empty item before `,` in xychart list");
      items.push(item);
      start = i + 1;
    }
  }
  if (inQuotes) throw new DiagramInputError("Unterminated quoted item in xychart list");
  const tail = raw.slice(start).trim();
  if (!tail) throw new DiagramInputError("Empty or trailing `,` in xychart list");
  items.push(tail);
  return items;
}

function parseRangeSuffix(raw: string): { title: string; min: number; max: number } {
  const arrow = findOutsideQuotes(raw, "-->");
  const leftPart = raw.slice(0, arrow).trim();
  const rightPart = raw.slice(arrow + 3).trim();
  if (!leftPart || !rightPart || !NUMBER.test(rightPart)) {
    throw new DiagramInputError(`Invalid numeric axis range: ${raw.slice(0, 40)}`);
  }
  const max = Number(rightPart);
  let title = "";
  let minRaw: string;
  if (leftPart.startsWith('"')) {
    const { value, next } = readQuoted(leftPart, 0);
    title = value;
    minRaw = leftPart.slice(next).trim();
  } else {
    const parts = leftPart.split(/\s+/);
    if (parts.length === 1) {
      minRaw = parts[0] ?? "";
    } else if (parts.length === 2) {
      title = parts[0] ?? "";
      minRaw = parts[1] ?? "";
    } else {
      throw new DiagramInputError(`Unquoted multi-word axis title must use quotes: ${leftPart}`);
    }
  }
  if (!NUMBER.test(minRaw)) {
    throw new DiagramInputError(`Invalid numeric axis minimum: ${leftPart.slice(0, 40)}`);
  }
  const min = Number(minRaw);
  if (!Number.isFinite(min) || !Number.isFinite(max) || min >= max) {
    throw new DiagramInputError("Axis range minimum must be less than maximum");
  }
  return { title, min, max };
}

function parseXAxis(raw: string): XyXAxis {
  const body = raw.trim();
  if (!body) throw new DiagramInputError("Empty x-axis specification");
  if (findOutsideQuotes(body, ":::") >= 0) {
    throw new DiagramInputError("Not drawn: CSS class attachment is not supported");
  }
  const open = findOutsideQuotes(body, "[");
  if (open >= 0) {
    const close = body.lastIndexOf("]");
    if (close <= open || body.slice(close + 1).trim() !== "") {
      throw new DiagramInputError(`Unclosed or trailing x-axis categories: ${body.slice(0, 40)}`);
    }
    const title = parseOptionalText(body.slice(0, open));
    const inner = body.slice(open + 1, close).trim();
    if (!inner) throw new DiagramInputError("Empty x-axis categories");
    const categories = splitBracketItems(inner).map((item) => {
      const text = parseOptionalText(item);
      if (!text) throw new DiagramInputError("Empty x-axis category");
      return text;
    });
    checkCount(categories.length, MAX_NODES, "categories");
    return { kind: "categorical", title, categories };
  }
  if (findOutsideQuotes(body, "-->") >= 0) {
    const range = parseRangeSuffix(body);
    return { kind: "numeric", ...range };
  }
  return { kind: "auto", title: parseOptionalText(body) };
}

function parseYAxis(raw: string): XyYAxis {
  const body = raw.trim();
  if (!body) throw new DiagramInputError("Empty y-axis specification");
  if (findOutsideQuotes(body, ":::") >= 0) {
    throw new DiagramInputError("Not drawn: CSS class attachment is not supported");
  }
  if (findOutsideQuotes(body, "[") >= 0) {
    throw new DiagramInputError("y-axis cannot be categorical");
  }
  if (findOutsideQuotes(body, "-->") >= 0) {
    const range = parseRangeSuffix(body);
    return { kind: "numeric", ...range };
  }
  return { kind: "auto", title: parseOptionalText(body) };
}

function parseSeries(kind: "bar" | "line", raw: string): XySeries {
  const body = raw.trim();
  if (findOutsideQuotes(body, ":::") >= 0) {
    throw new DiagramInputError("Not drawn: CSS class attachment is not supported");
  }
  const open = findOutsideQuotes(body, "[");
  if (open < 0) throw new DiagramInputError(`${kind} series needs [values]`);
  const close = body.lastIndexOf("]");
  if (close <= open || body.slice(close + 1).trim() !== "") {
    throw new DiagramInputError(`Unclosed or trailing text in ${kind} series`);
  }
  const nameRaw = body.slice(0, open).trim();
  const name = nameRaw ? parseOptionalText(nameRaw) : undefined;
  if (nameRaw && !name) throw new DiagramInputError(`Empty ${kind} series name`);
  const inner = body.slice(open + 1, close).trim();
  if (!inner) throw new DiagramInputError(`${kind} series cannot be empty`);

  const points: XyPoint[] = splitBracketItems(inner).map((item) => {
    const trimmed = item.trim();
    const quoteAt = findOutsideQuotes(trimmed, '"');
    let numPart = trimmed;
    let ptLabel: string | undefined;
    if (quoteAt >= 0) {
      numPart = trimmed.slice(0, quoteAt).trim();
      const { value: parsedLabel, next } = readQuoted(trimmed, quoteAt);
      if (trimmed.slice(next).trim() !== "") {
        throw new DiagramInputError(`Unexpected text after point label: ${trimmed.slice(0, 30)}`);
      }
      if (kind === "line" && parsedLabel) ptLabel = parsedLabel;
    }
    if (!NUMBER.test(numPart)) {
      throw new DiagramInputError(`Invalid number in ${kind} series: ${item.slice(0, 30)}`);
    }
    const value = Number(numPart);
    if (!Number.isFinite(value)) {
      throw new DiagramInputError(`Non-finite value in ${kind} series`);
    }
    return ptLabel !== undefined ? { value, label: ptLabel } : { value };
  });

  const out: XySeries = { kind, points };
  if (name !== undefined) out.name = name;
  return out;
}

/** Parse `xychart` / `xychart-beta` source; throws DiagramInputError or DiagramLimitError on problems. */
export function parseXyChart(input: string | PreparedSource): XyChartModel {
  const [header = "", ...body] = linesOf(input, "xychart");
  const head = /^xychart(?:-beta)?(?:\s+(vertical|horizontal))?\s*$/i.exec(header.trim());
  if (!head) throw new DiagramInputError(`Unreadable xychart header: ${header.slice(0, 40)}`);
  const orientation: XyOrientation =
    (head[1]?.toLowerCase() as XyOrientation | undefined) ?? "vertical";

  let title = "";
  let xAxis: XyXAxis = { kind: "auto", title: "" };
  let yAxis: XyYAxis = { kind: "auto", title: "" };
  const series: XySeries[] = [];
  let totalPoints = 0;

  for (const raw of body) {
    const line = raw.trim().replace(/;\s*$/, "").trim();
    if (!line) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }
    let match: RegExpExecArray | null;
    if ((match = /^title\s+(.+)$/i.exec(line))) {
      title = parseOptionalText(match[1] ?? "");
      if (!title) throw new DiagramInputError("Empty xychart title");
    } else if (/^title\s*$/i.test(line)) {
      throw new DiagramInputError("Empty xychart title");
    } else if ((match = /^x-axis\s+(.+)$/i.exec(line))) {
      xAxis = parseXAxis(match[1] ?? "");
    } else if (/^x-axis\s*$/i.test(line)) {
      throw new DiagramInputError("Empty x-axis");
    } else if ((match = /^y-axis\s+(.+)$/i.exec(line))) {
      yAxis = parseYAxis(match[1] ?? "");
    } else if (/^y-axis\s*$/i.test(line)) {
      throw new DiagramInputError("Empty y-axis");
    } else if ((match = /^(bar|line)\s+(.+)$/i.exec(line))) {
      const kind = (match[1] ?? "").toLowerCase() as "bar" | "line";
      const parsed = parseSeries(kind, match[2] ?? "");
      series.push(parsed);
      checkCount(series.length, MAX_NODES, "series");
      totalPoints += parsed.points.length;
      checkCount(totalPoints, MAX_EDGES, "data points");
    } else {
      throw new DiagramInputError(`Unreadable xychart line: ${line.slice(0, 40)}`);
    }
  }

  if (series.length === 0) {
    throw new DiagramInputError("An xychart needs at least one bar or line series");
  }
  const maxSeriesLen = Math.max(...series.map((s) => s.points.length));
  checkCount(maxSeriesLen, MAX_NODES, "data slots");
  if (xAxis.kind === "categorical" && maxSeriesLen > xAxis.categories.length) {
    throw new DiagramInputError("Series has more data points than x-axis categories");
  }
  return { orientation, title, xAxis, yAxis, series };
}
