/**
 * Quadrant chart parser. Grammar follows Mermaid's quadrantChart page: optional `title`,
 * `x-axis left [--> right]`, `y-axis bottom [--> top]`, `quadrant-1..4` labels, and data points
 * `label: [x, y]` with 0 <= x, y <= 1. CSS classes (`:::`, `classDef`, `cssClass`), directives,
 * links/callbacks and inline point styling are refused.
 */
import { linesOf } from "../../detect";
import { MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** One plotted point in unit quadrant coordinates `[0, 1] x [0, 1]`. */
export interface QuadrantPoint {
  label: string;
  x: number;
  y: number;
}

/** A parsed quadrant chart with optional title, axis captions, four quadrant names and points. */
export interface QuadrantModel {
  title: string;
  xAxis: { left: string; right: string };
  yAxis: { bottom: string; top: string };
  /** Labels for quadrant-1 (top-right), quadrant-2 (top-left), quadrant-3 (bottom-left), quadrant-4 (bottom-right). */
  quadrants: [string, string, string, string];
  points: QuadrantPoint[];
}

const NUMBER = /^[+-]?(?:\d+(?:\.\d+)?|\.\d+)$/;
const REFUSED = /^(?:classDef|cssClass|style|click|link|callback)\b/i;
const POINT_STYLE = /\b(?:color|radius|stroke-color|stroke-width)\s*:/i;

function unquote(raw: string): string {
  const trimmed = raw.trim();
  if (trimmed.length >= 2 && trimmed.startsWith('"') && trimmed.endsWith('"')) {
    return trimmed.slice(1, -1).replace(/\\(.)/g, "$1").trim();
  }
  return trimmed;
}

function splitAxis(raw: string, axisName: string): [string, string] {
  const value = raw.trim();
  if (!value) throw new DiagramInputError(`Empty ${axisName} label`);
  let inQuotes = false;
  let escaped = false;
  let arrowAt = -1;
  for (let i = 0; i < value.length; i++) {
    const ch = value[i];
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
    if (!inQuotes && value.startsWith("-->", i)) {
      arrowAt = i;
      break;
    }
  }
  if (inQuotes) throw new DiagramInputError(`Unterminated quote in ${axisName}`);
  if (arrowAt < 0) {
    const first = unquote(value);
    if (!first) throw new DiagramInputError(`Empty ${axisName} label`);
    return [first, ""];
  }
  const left = unquote(value.slice(0, arrowAt));
  const right = unquote(value.slice(arrowAt + 3));
  if (!left || !right) {
    throw new DiagramInputError(`${axisName} with \`-->\` needs labels on both sides`);
  }
  return [left, right];
}

function parsePointLine(line: string): QuadrantPoint | undefined {
  let label: string;
  let rest: string;
  if (line.startsWith('"')) {
    let end = -1;
    let escaped = false;
    for (let i = 1; i < line.length; i++) {
      const ch = line[i];
      if (escaped) {
        escaped = false;
        continue;
      }
      if (ch === "\\") {
        escaped = true;
        continue;
      }
      if (ch === '"') {
        end = i;
        break;
      }
    }
    if (end < 0) throw new DiagramInputError("Unterminated quoted point label");
    label = line.slice(1, end).replace(/\\(.)/g, "$1").trim();
    rest = line.slice(end + 1).trim();
  } else {
    if (line.includes(":::")) {
      throw new DiagramInputError("Not drawn: CSS class attachment is not supported");
    }
    const colon = line.search(/:\s*\[/);
    if (colon < 0) return undefined;
    label = line.slice(0, colon).trim();
    rest = line.slice(colon).trim();
  }
  if (rest.startsWith(":::")) {
    throw new DiagramInputError("Not drawn: CSS class attachment is not supported");
  }
  if (!rest.startsWith(":")) {
    throw new DiagramInputError(`Unreadable quadrant point: ${line.slice(0, 40)}`);
  }
  if (!label) throw new DiagramInputError("Point needs a label");
  const afterColon = rest.slice(1).trim();
  const coords = /^\[\s*([^\],]+?)\s*,\s*([^\]]+?)\s*\](.*)$/.exec(afterColon);
  if (!coords) {
    throw new DiagramInputError(`Point coordinates must be [x, y]: ${line.slice(0, 40)}`);
  }
  const trailing = (coords[3] ?? "").replace(/;\s*$/, "").trim();
  if (trailing) {
    if (POINT_STYLE.test(trailing)) {
      throw new DiagramInputError("Not drawn: point styling is not supported");
    }
    throw new DiagramInputError(
      `Unexpected text after point coordinates: ${trailing.slice(0, 30)}`,
    );
  }
  const rawX = (coords[1] ?? "").trim();
  const rawY = (coords[2] ?? "").trim();
  if (!NUMBER.test(rawX) || !NUMBER.test(rawY)) {
    throw new DiagramInputError(`Point coordinates must be numbers: ${line.slice(0, 40)}`);
  }
  const x = Number(rawX);
  const y = Number(rawY);
  if (!Number.isFinite(x) || !Number.isFinite(y) || x < 0 || x > 1 || y < 0 || y > 1) {
    throw new DiagramInputError("Point coordinates must be between 0 and 1");
  }
  return { label, x, y };
}

/** Parse `quadrantChart` source (text or prepared lines); throws DiagramInputError on invalid syntax or bounds. */
export function parseQuadrant(input: string | PreparedSource): QuadrantModel {
  const [header = "", ...body] = linesOf(input, "quadrant");
  if (!/^quadrantChart\s*$/i.test(header.trim())) {
    throw new DiagramInputError("Unreadable quadrantChart header");
  }
  const model: QuadrantModel = {
    title: "",
    xAxis: { left: "", right: "" },
    yAxis: { bottom: "", top: "" },
    quadrants: ["", "", "", ""],
    points: [],
  };
  let hasContent = false;

  for (const raw of body) {
    const line = raw.trim().replace(/;\s*$/, "").trim();
    if (!line) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }
    let match: RegExpExecArray | null;
    if ((match = /^title\s+(.+)$/i.exec(line))) {
      const text = unquote(match[1] ?? "");
      if (!text) throw new DiagramInputError("Empty quadrant title");
      model.title = text;
      hasContent = true;
    } else if (/^title\s*$/i.test(line)) {
      throw new DiagramInputError("Empty quadrant title");
    } else if ((match = /^x-axis\s+(.+)$/i.exec(line))) {
      const [left, right] = splitAxis(match[1] ?? "", "x-axis");
      model.xAxis = { left, right };
      hasContent = true;
    } else if (/^x-axis\s*$/i.test(line)) {
      throw new DiagramInputError("Empty x-axis");
    } else if ((match = /^y-axis\s+(.+)$/i.exec(line))) {
      const [bottom, top] = splitAxis(match[1] ?? "", "y-axis");
      model.yAxis = { bottom, top };
      hasContent = true;
    } else if (/^y-axis\s*$/i.test(line)) {
      throw new DiagramInputError("Empty y-axis");
    } else if ((match = /^quadrant-([1-4])\s+(.+)$/i.exec(line))) {
      const index = Number(match[1]) - 1;
      const label = unquote(match[2] ?? "");
      if (!label) throw new DiagramInputError(`Empty quadrant-${String(index + 1)} label`);
      model.quadrants[index] = label;
      hasContent = true;
    } else if (/^quadrant-/i.test(line)) {
      throw new DiagramInputError(`Invalid quadrant directive: ${line.slice(0, 40)}`);
    } else {
      const point = parsePointLine(line);
      if (!point) throw new DiagramInputError(`Unreadable quadrant line: ${line.slice(0, 40)}`);
      model.points.push(point);
      checkCount(model.points.length, MAX_NODES, "points");
      hasContent = true;
    }
  }
  if (!hasContent) {
    throw new DiagramInputError("A quadrant chart needs axes, quadrants, or points");
  }
  return model;
}
