/**
 * Radar chart parser. Grammar follows Mermaid's radar-beta page: `title`, `axis id["Label"]`,
 * `curve id["Label"]{values}` (positional or `axisId: value`), and options (`showLegend`, `min`,
 * `max`, `graticule`, `ticks`). Styling directives and CSS class attachment (`:::`) are refused.
 */
import { linesOf } from "../../detect";
import { MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** One radial axis with stable id and display label. */
export interface RadarAxis {
  id: string;
  label: string;
}

/** One data series plotted across every axis in `axes` order. */
export interface RadarCurve {
  id: string;
  label: string;
  values: number[];
}

/** A parsed radar chart ready for polar drawing. */
export interface RadarModel {
  title: string;
  axes: RadarAxis[];
  curves: RadarCurve[];
  min: number;
  max: number;
  graticule: "circle" | "polygon";
  ticks: number;
  showLegend: boolean;
}

const NUMBER = /^[+-]?(?:\d+(?:\.\d+)?|\.\d+)$/;
const REFUSED = /^(?:classDef|cssClass|style|click|link|callback)\b/i;
const MAX_TICKS = 50;

function unquote(raw: string): string {
  const trimmed = raw.trim();
  if (trimmed.length >= 2 && trimmed.startsWith('"') && trimmed.endsWith('"')) {
    return trimmed.slice(1, -1).replace(/\\(.)/g, "$1").trim();
  }
  return trimmed;
}

/** Split comma-separated items while respecting double quotes and `{...}` / `[...]` groups. */
function splitCommaItems(raw: string): string[] {
  const out: string[] = [];
  let inQuotes = false;
  let escaped = false;
  let braceDepth = 0;
  let bracketDepth = 0;
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
    if (inQuotes) continue;
    if (ch === "{") braceDepth++;
    else if (ch === "}") {
      braceDepth--;
      if (braceDepth < 0) throw new DiagramInputError("Unexpected `}` in radar line");
    } else if (ch === "[") bracketDepth++;
    else if (ch === "]") {
      bracketDepth--;
      if (bracketDepth < 0) throw new DiagramInputError("Unexpected `]` in radar line");
    } else if (ch === "," && braceDepth === 0 && bracketDepth === 0) {
      const part = raw.slice(start, i).trim();
      if (!part) throw new DiagramInputError("Empty item before `,` in radar list");
      out.push(part);
      start = i + 1;
    }
  }
  if (inQuotes) throw new DiagramInputError("Unterminated quoted string in radar diagram");
  if (braceDepth !== 0 || bracketDepth !== 0) {
    throw new DiagramInputError("Unclosed `{` or `[` in radar diagram");
  }
  const tail = raw.slice(start).trim();
  if (!tail) throw new DiagramInputError("Trailing `,` in radar list");
  out.push(tail);
  return out;
}

type RawCurve =
  | { id: string; label: string; kind: "pos"; values: number[] }
  | { id: string; label: string; kind: "kv"; entries: Map<string, number> };

function findOpenBraceOutsideQuotes(raw: string): number {
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
    if (ch === '"') inQuotes = !inQuotes;
    else if (ch === "{" && !inQuotes) return i;
  }
  return -1;
}

function parseCurveItem(raw: string): RawCurve {
  const open = findOpenBraceOutsideQuotes(raw);
  const close = raw.lastIndexOf("}");
  if (open < 0 || close !== raw.length - 1 || close <= open) {
    throw new DiagramInputError(`Unreadable radar curve: ${raw.slice(0, 40)}`);
  }
  const head = raw.slice(0, open).trim();
  if (head.includes(":::")) {
    throw new DiagramInputError("Not drawn: CSS class attachment is not supported");
  }
  const headMatch = /^([^\s[\]{}",:]+)(?:\["((?:[^"\\]|\\.)*)"\])?$/.exec(head);
  if (!headMatch) {
    throw new DiagramInputError(`Unreadable radar curve header: ${head.slice(0, 40)}`);
  }
  const id = headMatch[1] ?? "";
  const label = headMatch[2] !== undefined ? headMatch[2].replace(/\\(.)/g, "$1").trim() || id : id;
  const body = raw.slice(open + 1, close).trim();
  if (!body) throw new DiagramInputError(`Curve ${id} has no values`);
  const items = splitCommaItems(body);
  const isKv = items[0]?.includes(":") ?? false;
  if (isKv) {
    const entries = new Map<string, number>();
    for (const item of items) {
      const colon = item.indexOf(":");
      if (colon < 0) throw new DiagramInputError(`Cannot mix positional and keyed values in ${id}`);
      const key = item.slice(0, colon).trim();
      const valRaw = item.slice(colon + 1).trim();
      if (!key || !NUMBER.test(valRaw)) {
        throw new DiagramInputError(`Invalid keyed value in curve ${id}: ${item.slice(0, 30)}`);
      }
      if (entries.has(key)) {
        throw new DiagramInputError(`Duplicate axis ${key} in curve ${id}`);
      }
      entries.set(key, Number(valRaw));
    }
    return { id, label, kind: "kv", entries };
  }
  const values: number[] = [];
  for (const item of items) {
    if (item.includes(":")) {
      throw new DiagramInputError(`Cannot mix positional and keyed values in ${id}`);
    }
    if (!NUMBER.test(item)) {
      throw new DiagramInputError(`Invalid numeric value in curve ${id}: ${item.slice(0, 30)}`);
    }
    values.push(Number(item));
  }
  return { id, label, kind: "pos", values };
}

/** Parse `radar-beta` / `radar` source; throws DiagramInputError or DiagramLimitError on invalid/oversized input. */
export function parseRadar(input: string | PreparedSource): RadarModel {
  const [header = "", ...body] = linesOf(input, "radar");
  if (!/^radar(?:-beta)?\s*$/i.test(header.trim())) {
    throw new DiagramInputError("Unreadable radar header");
  }
  let title = "";
  const axes: RadarAxis[] = [];
  const axisIds = new Set<string>();
  const rawCurves: RawCurve[] = [];
  const curveIds = new Set<string>();
  let userMin: number | undefined;
  let userMax: number | undefined;
  let graticule: "circle" | "polygon" = "circle";
  let ticks = 5;
  let showLegend = true;

  for (const raw of body) {
    const line = raw.trim().replace(/;\s*$/, "").trim();
    if (!line) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }
    let match: RegExpExecArray | null;
    if ((match = /^title\s+(.+)$/i.exec(line))) {
      title = unquote(match[1] ?? "");
      if (!title) throw new DiagramInputError("Empty radar title");
    } else if (/^title\s*$/i.test(line)) {
      throw new DiagramInputError("Empty radar title");
    } else if ((match = /^axis\s+(.+)$/i.exec(line))) {
      for (const item of splitCommaItems(match[1] ?? "")) {
        if (item.includes(":::")) {
          throw new DiagramInputError("Not drawn: CSS class attachment is not supported");
        }
        const m = /^([^\s[\]{}",:]+)(?:\["((?:[^"\\]|\\.)*)"\])?$/.exec(item);
        if (!m) throw new DiagramInputError(`Unreadable radar axis: ${item.slice(0, 40)}`);
        const id = m[1] ?? "";
        if (axisIds.has(id)) throw new DiagramInputError(`Duplicate radar axis id: ${id}`);
        axisIds.add(id);
        const label = m[2] !== undefined ? m[2].replace(/\\(.)/g, "$1").trim() || id : id;
        axes.push({ id, label });
        checkCount(axes.length, MAX_NODES, "axes");
      }
    } else if ((match = /^curve\s+(.+)$/i.exec(line))) {
      for (const item of splitCommaItems(match[1] ?? "")) {
        const parsed = parseCurveItem(item);
        if (curveIds.has(parsed.id)) {
          throw new DiagramInputError(`Duplicate radar curve id: ${parsed.id}`);
        }
        curveIds.add(parsed.id);
        rawCurves.push(parsed);
        checkCount(rawCurves.length, MAX_NODES, "curves");
      }
    } else if ((match = /^min\s+(\S+)$/i.exec(line))) {
      const val = match[1] ?? "";
      if (!NUMBER.test(val)) throw new DiagramInputError(`Invalid radar min: ${val}`);
      userMin = Number(val);
    } else if ((match = /^max\s+(\S+)$/i.exec(line))) {
      const val = match[1] ?? "";
      if (!NUMBER.test(val)) throw new DiagramInputError(`Invalid radar max: ${val}`);
      userMax = Number(val);
    } else if ((match = /^graticule\s+(\S+)$/i.exec(line))) {
      const kind = (match[1] ?? "").toLowerCase();
      if (kind !== "circle" && kind !== "polygon") {
        throw new DiagramInputError(`Unknown graticule type: ${kind}`);
      }
      graticule = kind;
    } else if ((match = /^ticks\s+(\S+)$/i.exec(line))) {
      const val = match[1] ?? "";
      if (!/^\d+$/.test(val)) throw new DiagramInputError(`Radar ticks must be an integer: ${val}`);
      const count = Number(val);
      if (count < 1) throw new DiagramInputError("Radar ticks must be at least 1");
      if (count > MAX_TICKS) {
        throw new DiagramLimitError(`Diagram too large: more than ${String(MAX_TICKS)} ticks`);
      }
      ticks = count;
    } else if ((match = /^showLegend\s+(\S+)$/i.exec(line))) {
      const flag = (match[1] ?? "").toLowerCase();
      if (flag !== "true" && flag !== "false") {
        throw new DiagramInputError(`showLegend must be true or false: ${flag}`);
      }
      showLegend = flag === "true";
    } else {
      throw new DiagramInputError(`Unreadable radar line: ${line.slice(0, 40)}`);
    }
  }

  if (axes.length === 0) throw new DiagramInputError("A radar chart needs axes");
  if (rawCurves.length === 0) throw new DiagramInputError("A radar chart needs curves");

  const min = userMin ?? 0;
  const curves: RadarCurve[] = rawCurves.map((raw) => {
    if (raw.kind === "pos") {
      if (raw.values.length !== axes.length) {
        throw new DiagramInputError(
          `Curve ${raw.id} has ${String(raw.values.length)} values for ${String(axes.length)} axes`,
        );
      }
      return { id: raw.id, label: raw.label, values: raw.values };
    }
    for (const key of raw.entries.keys()) {
      if (!axisIds.has(key)) {
        throw new DiagramInputError(`Curve ${raw.id} references unknown axis: ${key}`);
      }
    }
    const values = axes.map((ax) => raw.entries.get(ax.id) ?? min);
    return { id: raw.id, label: raw.label, values };
  });

  const maxData = Math.max(min, ...curves.flatMap((c) => c.values));
  const max = userMax !== undefined ? userMax : maxData > min ? maxData : min + 1;
  if (!Number.isFinite(min) || !Number.isFinite(max) || max <= min) {
    throw new DiagramInputError("Radar max must be greater than min");
  }
  return { title, axes, curves, min, max, graticule, ticks, showLegend };
}
