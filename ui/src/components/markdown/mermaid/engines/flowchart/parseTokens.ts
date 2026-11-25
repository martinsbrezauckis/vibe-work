/**
 * Lexical and token-level parsers for Mermaid flowchart statements: entity decoding, node shapes
 * (both classic syntax and `@{ ... }` metadata blocks), metadata sanitisation, edge operators,
 * mid-link labels, pipe labels, edge IDs, and `&` endpoint splitting.
 */
import { StepBudget } from "../../limits";
import { clampLabel } from "../../measure";
import { DiagramInputError } from "../../types";
import type { FlowEdgeStyle, FlowEndpointMark, FlowShape } from "./types";

const PROTO_KEYS = new Set(["__proto__", "constructor", "prototype"]);
const NODE_ID_CHARS = /^[A-Za-z0-9_.\u0080-\uFFFF-]+$/;

/** Map from documented Mermaid shape names and aliases to canonical FlowShape kinds. */
export const SHAPE_ALIASES: Record<string, FlowShape> = {
  rect: "rect",
  rectangle: "rect",
  proc: "rect",
  process: "rect",
  rounded: "rounded",
  round: "rounded",
  event: "rounded",
  stadium: "stadium",
  pill: "stadium",
  terminal: "stadium",
  subproc: "subproc",
  subprocess: "subproc",
  subroutine: "subproc",
  cyl: "cyl",
  cylinder: "cyl",
  database: "cyl",
  db: "cyl",
  circle: "circle",
  circ: "circle",
  odd: "odd",
  diamond: "diamond",
  diam: "diamond",
  decision: "diamond",
  hex: "hex",
  hexagon: "hex",
  prepare: "hex",
  "lean-r": "lean-r",
  "lean-right": "lean-r",
  "lean-l": "lean-l",
  "lean-left": "lean-l",
  datastore: "datastore",
  "h-cyl": "das",
  das: "das",
  "trap-b": "trap-b",
  "trap-bottom": "trap-b",
  "trap-t": "trap-t",
  "trap-top": "trap-t",
  "dbl-circ": "dbl-circ",
  "double-circle": "dbl-circ",
  text: "text",
  "notch-rect": "notch-rect",
  card: "notch-rect",
  "lin-rect": "lin-rect",
  "lined-rect": "lin-rect",
  "sm-circ": "sm-circ",
  start: "sm-circ",
  "framed-circle": "framed-circle",
  "fr-circ": "framed-circle",
  stop: "framed-circle",
  fork: "fork",
  join: "fork",
  hourglass: "hourglass",
  collate: "hourglass",
  comment: "comment",
  "brace-l": "comment",
  "brace-r": "brace-r",
  braces: "braces",
  bolt: "bolt",
  "com-link": "bolt",
  doc: "doc",
  document: "doc",
  delay: "delay",
  "half-rounded-rectangle": "delay",
  "lin-cyl": "lin-cyl",
  "curv-trap": "curv-trap",
  display: "curv-trap",
  "div-rect": "div-rect",
  "div-proc": "div-rect",
  tri: "tri",
  triangle: "tri",
  extract: "tri",
  "win-pane": "win-pane",
  "internal-storage": "win-pane",
  "f-circ": "f-circ",
  junction: "f-circ",
  "lin-doc": "lin-doc",
  "notch-pent": "notch-pent",
  "loop-limit": "notch-pent",
  "flip-tri": "flip-tri",
  "manual-file": "flip-tri",
  "sl-rect": "sl-rect",
  "manual-input": "sl-rect",
  docs: "docs",
  "st-doc": "docs",
  processes: "processes",
  procs: "processes",
  "st-rect": "processes",
  flag: "flag",
  "paper-tape": "flag",
  "bow-rect": "bow-rect",
  "stored-data": "bow-rect",
  "cross-circ": "cross-circ",
  summary: "cross-circ",
  "tag-doc": "tag-doc",
  "tag-rect": "tag-rect",
};

const NAMED_ENTITIES: Record<string, string> = {
  quot: '"',
  amp: "&",
  lt: "<",
  gt: ">",
  apos: "'",
  nbsp: " ",
  copy: "©",
  reg: "®",
  trade: "™",
  mdash: "—",
  ndash: "–",
  hellip: "…",
  hearts: "♥",
  heart: "♥",
};

const CLASSIC_PATTERNS: [RegExp, FlowShape][] = [
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\(\(\(([\s\S]*)\)\)\)$/, "dbl-circ"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\(\[([\s\S]*)\]\)$/, "stadium"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\(\(([\s\S]*)\)\)$/, "circle"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\[\[([\s\S]*)\]\]$/, "subproc"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\[\(([\s\S]*)\)\]$/, "cyl"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\[\/([\s\S]*)\\\]$/, "trap-b"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\[\\([\s\S]*)\/\]$/, "trap-t"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\[\/([\s\S]*)\/\]$/, "lean-r"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\[\\([\s\S]*)\\\]$/, "lean-l"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\{\{([\s\S]*)\}\}$/, "hex"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\[([\s\S]*)\]$/, "rect"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\(([\s\S]*)\)$/, "rounded"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)\{([\s\S]*)\}$/, "diamond"],
  [/^([A-Za-z0-9_.\u0080-\uFFFF-]+)>([\s\S]*)\]$/, "odd"],
];

/** Result of parsing a single flowchart node token. */
export interface ParsedNodeToken {
  id: string;
  label?: string;
  shape?: FlowShape;
  meta?: Record<string, string>;
}

/** Result of scanning an edge operator inside a statement. */
export interface ParsedOp {
  index: number;
  length: number;
  edgeId?: string;
  label: string;
  style: FlowEdgeStyle;
  startMark: FlowEndpointMark;
  endMark: FlowEndpointMark;
  minlen: number;
}

/** Decode Mermaid `#quot;`, `#9829;`, and `#x2764;` entity references into plain characters. */
export function decodeEntities(text: string): string {
  return text.replace(/#([A-Za-z0-9]+);/g, (full, code: string) => {
    if (/^\d+$/.test(code)) {
      const cp = Number(code);
      return Number.isInteger(cp) && cp > 0 && cp <= 0x10ffff ? String.fromCodePoint(cp) : full;
    }
    if (/^x[0-9a-fA-F]+$/i.test(code)) {
      const cp = Number.parseInt(code.slice(1), 16);
      return Number.isInteger(cp) && cp > 0 && cp <= 0x10ffff ? String.fromCodePoint(cp) : full;
    }
    return NAMED_ENTITIES[code.toLowerCase()] ?? full;
  });
}

/** Unquote, strip safe Markdown wrapping, decode Mermaid entities, and clamp label length. */
export function decodeLabel(raw: string): string {
  let s = raw.trim();
  if (s.startsWith('"') && s.endsWith('"') && s.length >= 2) {
    s = s.slice(1, -1).replace(/\\(.)/g, "$1");
  }
  s = s.trim();
  if (s.startsWith("`") && s.endsWith("`") && s.length >= 2) {
    s = s
      .slice(1, -1)
      .replace(/\*\*([^*]+)\*\*/g, "$1")
      .replace(/__([^_]+)__/g, "$1")
      .replace(/\*([^*]+)\*/g, "$1")
      .replace(/(^|\s)_([^_]+)_(\s|$)/g, "$1$2$3");
  }
  return clampLabel(decodeEntities(s));
}

/** Return true if `text` contains an unquoted `:::` class attachment operator. */
export function hasUnquotedClassAttach(text: string): boolean {
  let inQuote = false;
  for (let i = 0; i < text.length - 2; i++) {
    const ch = text[i] ?? "";
    if (inQuote) {
      if (ch === "\\" && i + 1 < text.length) i++;
      else if (ch === '"') inQuote = false;
      continue;
    }
    if (ch === '"') inQuote = true;
    else if (ch === ":" && text[i + 1] === ":" && text[i + 2] === ":") return true;
  }
  return false;
}

/** Parse comma/newline-separated `key: value` pairs from a `@{ ... }` metadata body safely. */
export function parseMeta(raw: string): Record<string, string> {
  const safe: Record<string, string> = {};
  const parts: string[] = [];
  let start = 0;
  let inQuote = false;
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i] ?? "";
    if (inQuote) {
      if (ch === "\\" && i + 1 < raw.length) i++;
      else if (ch === '"') inQuote = false;
      continue;
    }
    if (ch === '"') inQuote = true;
    else if (ch === "," || ch === "\n") {
      const piece = raw.slice(start, i).trim();
      if (piece) parts.push(piece);
      start = i + 1;
    }
  }
  const tail = raw.slice(start).trim();
  if (tail) parts.push(tail);

  for (const part of parts) {
    const colon = part.indexOf(":");
    if (colon <= 0) throw new DiagramInputError(`Invalid metadata entry: ${part.slice(0, 40)}`);
    const key = part.slice(0, colon).trim();
    if (PROTO_KEYS.has(key)) continue;
    let val = part.slice(colon + 1).trim();
    if (
      ((val.startsWith('"') && val.endsWith('"')) || (val.startsWith("'") && val.endsWith("'"))) &&
      val.length >= 2
    ) {
      val = val.slice(1, -1).replace(/\\(.)/g, "$1");
    }
    Object.defineProperty(safe, key, {
      value: clampLabel(decodeEntities(val)),
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return safe;
}

function extractTrailingMeta(token: string): { head: string; meta?: Record<string, string> } {
  const trimmed = token.trim();
  if (!trimmed.endsWith("}")) return { head: trimmed };
  let inQuote = false;
  let depth = 0;
  for (let i = trimmed.length - 1; i >= 0; i--) {
    const ch = trimmed[i] ?? "";
    if (ch === '"' && trimmed[i - 1] !== "\\") {
      inQuote = !inQuote;
      continue;
    }
    if (inQuote) continue;
    if (ch === "}") depth++;
    else if (ch === "{") {
      depth--;
      if (depth === 0) {
        if (trimmed[i - 1] === "@") {
          const head = trimmed.slice(0, i - 1).trim();
          const meta = parseMeta(trimmed.slice(i + 1, -1));
          return { head, meta };
        }
        return { head: trimmed };
      }
    }
  }
  return { head: trimmed };
}

/** Parse one flowchart node reference, supporting classic shapes and `@{ shape, label }` blocks. */
export function parseNodeToken(raw: string): ParsedNodeToken {
  if (hasUnquotedClassAttach(raw)) {
    throw new DiagramInputError("Class attachment (:::) is not supported");
  }
  const { head, meta } = extractTrailingMeta(raw);
  if (!head) throw new DiagramInputError(`Missing node identifier: ${raw.slice(0, 40)}`);

  if (meta) {
    if (
      "img" in meta ||
      "icon" in meta ||
      meta.shape === "img" ||
      meta.shape === "image" ||
      meta.shape === "icon"
    ) {
      throw new DiagramInputError(
        "Image and icon shapes are not supported in offline passive diagrams",
      );
    }
    if (meta.shape !== undefined && !SHAPE_ALIASES[meta.shape.toLowerCase()]) {
      throw new DiagramInputError(`Unsupported flowchart shape: ${meta.shape}`);
    }
  }

  let res: ParsedNodeToken | undefined;
  for (const [re, shape] of CLASSIC_PATTERNS) {
    const m = re.exec(head);
    if (m?.[1]) {
      const inner = (m[2] ?? "").trim();
      if (!inner) throw new DiagramInputError(`Empty label in node ${m[1]}`);
      res = { id: m[1], label: decodeLabel(inner), shape };
      break;
    }
  }
  if (!res) {
    if (!NODE_ID_CHARS.test(head)) {
      throw new DiagramInputError(`Unreadable flowchart node: ${head.slice(0, 40)}`);
    }
    res = { id: head };
  }
  if (meta) {
    res.meta = meta;
    if (meta.shape) {
      const mapped = SHAPE_ALIASES[meta.shape.toLowerCase()];
      if (mapped) res.shape = mapped;
    }
    if (meta.label !== undefined) res.label = meta.label;
  }
  return res;
}

function endpointFromChar(ch: string | undefined): FlowEndpointMark {
  if (ch === ">") return "arrow";
  if (ch === "o") return "circle";
  if (ch === "x") return "cross";
  return "none";
}

function matchMidTail(
  prefix: "-- " | "-. " | "== ",
  rest: string,
):
  | { tokenLen: number; style: FlowEdgeStyle; endMark: FlowEndpointMark; minlen: number }
  | undefined {
  if (prefix === "-- ") {
    const m = /^(-{2,}>|-{3,}|-{2,}[ox])/.exec(rest);
    if (!m?.[1]) return undefined;
    const tok = m[1];
    const endMark = endpointFromChar(tok[tok.length - 1]);
    const bodyLen = endMark === "none" ? tok.length : tok.length - 1;
    return {
      tokenLen: tok.length,
      style: "solid",
      endMark,
      minlen: Math.max(1, endMark === "none" ? bodyLen - 2 : bodyLen - 1),
    };
  }
  if (prefix === "-. ") {
    const m = /^(\.-+>|\.-+|\.-+[ox])/.exec(rest);
    if (!m?.[1]) return undefined;
    const tok = m[1];
    const endMark = endpointFromChar(tok[tok.length - 1]);
    const dots = tok.replace(/[^.]/g, "").length;
    return { tokenLen: tok.length, style: "dotted", endMark, minlen: Math.max(1, dots) };
  }
  const m = /^(={2,}>|={3,}|={2,}[ox])/.exec(rest);
  if (!m?.[1]) return undefined;
  const tok = m[1];
  const endMark = endpointFromChar(tok[tok.length - 1]);
  const bodyLen = endMark === "none" ? tok.length : tok.length - 1;
  return {
    tokenLen: tok.length,
    style: "thick",
    endMark,
    minlen: Math.max(1, endMark === "none" ? bodyLen - 2 : bodyLen - 1),
  };
}

function matchMidLabeledOp(slice: string): Omit<ParsedOp, "index" | "edgeId"> | undefined {
  const prefix = slice.startsWith("-- ")
    ? "-- "
    : slice.startsWith("-. ")
      ? "-. "
      : slice.startsWith("== ")
        ? "== "
        : undefined;
  if (!prefix) return undefined;

  let inQuote = false;
  for (let i = 3; i < slice.length; i++) {
    const ch = slice[i] ?? "";
    if (inQuote) {
      if (ch === "\\" && i + 1 < slice.length) i++;
      else if (ch === '"') inQuote = false;
      continue;
    }
    if (ch === '"') {
      inQuote = true;
      continue;
    }
    if (ch !== " ") continue;
    const matched = matchMidTail(prefix, slice.slice(i + 1));
    if (matched) {
      return {
        length: i + 1 + matched.tokenLen,
        label: decodeLabel(slice.slice(3, i)),
        style: matched.style,
        startMark: "none",
        endMark: matched.endMark,
        minlen: matched.minlen,
      };
    }
  }
  return undefined;
}

function matchUnlabeledOp(
  line: string,
  pos: number,
  boundaryBefore: boolean,
): Omit<ParsedOp, "index" | "edgeId"> | undefined {
  const slice = line.slice(pos);
  const first = slice[0] ?? "";
  let cursor = 0;
  let startMark: FlowEndpointMark = "none";

  if ((first === "<" || first === "o" || first === "x") && boundaryBefore) {
    const next = slice[1] ?? "";
    if (next === "-" || next === "=") {
      startMark = first === "<" ? "arrow" : first === "o" ? "circle" : "cross";
      cursor = 1;
    } else {
      return undefined;
    }
  }

  const body = slice.slice(cursor);
  let style: FlowEdgeStyle | undefined;
  let tokLen = 0;
  let endMark: FlowEndpointMark = "none";
  let minlen = 1;

  const inv = /^(~{3,})/.exec(body);
  const dot = /^(-\.+)(?:-([>ox])?|([>ox]))/.exec(body);
  const thick = /^(={2,})([>ox])?/.exec(body);
  const solid = /^(-{2,})([>ox])?/.exec(body);

  if (inv?.[1] && startMark === "none") {
    style = "invisible";
    tokLen = inv[1].length;
    minlen = Math.max(1, tokLen - 2);
  } else if (dot?.[1]) {
    style = "dotted";
    endMark = endpointFromChar(dot[2] ?? dot[3]);
    tokLen = dot[0].length;
    minlen = Math.max(1, dot[1].length - 1);
  } else if (thick?.[1]) {
    const eqLen = thick[1].length;
    endMark = endpointFromChar(thick[2]);
    if (eqLen < 3 && startMark === "none" && endMark === "none") return undefined;
    style = "thick";
    tokLen = thick[0].length;
    minlen = Math.max(1, startMark === "none" && endMark === "none" ? eqLen - 2 : eqLen - 1);
  } else if (solid?.[1]) {
    const dashLen = solid[1].length;
    endMark = endpointFromChar(solid[2]);
    if (dashLen < 3 && startMark === "none" && endMark === "none") return undefined;
    style = "solid";
    tokLen = solid[0].length;
    minlen = Math.max(1, startMark === "none" && endMark === "none" ? dashLen - 2 : dashLen - 1);
  }

  if (!style) return undefined;

  let totalLen = cursor + tokLen;
  let label = "";
  const after = slice.slice(totalLen);
  const pipe = /^\s*\|([^|]*)\|/.exec(after);
  if (pipe) {
    label = decodeLabel(pipe[1] ?? "");
    totalLen += pipe[0].length;
  }
  return { length: totalLen, label, style, startMark, endMark, minlen };
}

/** Find the next unbracketed edge operator in `line` at or after `fromIndex`. */
export function findNextEdgeOp(
  line: string,
  fromIndex: number,
  budget: StepBudget,
): ParsedOp | undefined {
  let sq = 0;
  let par = 0;
  let br = 0;
  let inQuote = false;
  let inAsym = false;

  for (let i = fromIndex; i < line.length; i++) {
    budget.tick();
    const ch = line[i] ?? "";
    if (inQuote) {
      if (ch === "\\" && i + 1 < line.length) i++;
      else if (ch === '"') inQuote = false;
      continue;
    }
    if (inAsym) {
      if (ch === '"') inQuote = true;
      else if (ch === "]") inAsym = false;
      continue;
    }
    if (ch === '"') {
      inQuote = true;
      continue;
    }
    if (ch === "[") {
      sq++;
      continue;
    }
    if (ch === "]" && sq > 0) {
      sq--;
      continue;
    }
    if (ch === "(") {
      par++;
      continue;
    }
    if (ch === ")" && par > 0) {
      par--;
      continue;
    }
    if (ch === "{") {
      br++;
      continue;
    }
    if (ch === "}" && br > 0) {
      br--;
      continue;
    }
    if (sq > 0 || par > 0 || br > 0) continue;

    // Asymmetric node `id>text]`: `>` directly follows an identifier character and closes at `]`.
    if (
      ch === ">" &&
      i > 0 &&
      /[A-Za-z0-9_.\u0080-\uFFFF]/.test(line[i - 1] ?? "") &&
      line.indexOf("]", i + 1) > i
    ) {
      inAsym = true;
      continue;
    }

    const boundaryBefore = i === fromIndex || /[\s\])}"]/.test(line[i - 1] ?? "");
    let edgeId: string | undefined;
    let opStart = i;
    if (boundaryBefore) {
      const idPrefix = /^([A-Za-z0-9_.-]+)@(?=[-~=.ox<])/.exec(line.slice(i));
      if (idPrefix?.[1]) {
        edgeId = idPrefix[1];
        opStart = i + idPrefix[0].length;
      }
    }

    const mid = matchMidLabeledOp(line.slice(opStart));
    if (mid) {
      const res: ParsedOp = {
        index: i,
        length: opStart - i + mid.length,
        label: mid.label,
        style: mid.style,
        startMark: mid.startMark,
        endMark: mid.endMark,
        minlen: mid.minlen,
      };
      if (edgeId !== undefined) res.edgeId = edgeId;
      return res;
    }

    const plain = matchUnlabeledOp(line, opStart, boundaryBefore || edgeId !== undefined);
    if (plain) {
      const res: ParsedOp = {
        index: i,
        length: opStart - i + plain.length,
        label: plain.label,
        style: plain.style,
        startMark: plain.startMark,
        endMark: plain.endMark,
        minlen: plain.minlen,
      };
      if (edgeId !== undefined) res.edgeId = edgeId;
      return res;
    }
  }
  return undefined;
}

/** Split an edge endpoint group on unbracketed `&` combinators (`A & B`). */
export function splitAmpersands(segment: string, budget: StepBudget): string[] {
  const parts: string[] = [];
  let start = 0;
  let inQuote = false;
  let sq = 0;
  let par = 0;
  let br = 0;
  let inAsym = false;

  for (let i = 0; i < segment.length; i++) {
    budget.tick();
    const ch = segment[i] ?? "";
    if (inQuote) {
      if (ch === "\\" && i + 1 < segment.length) i++;
      else if (ch === '"') inQuote = false;
      continue;
    }
    if (inAsym) {
      if (ch === '"') inQuote = true;
      else if (ch === "]") inAsym = false;
      continue;
    }
    if (ch === '"') inQuote = true;
    else if (ch === "[") sq++;
    else if (ch === "]" && sq > 0) sq--;
    else if (ch === "(") par++;
    else if (ch === ")" && par > 0) par--;
    else if (ch === "{") br++;
    else if (ch === "}" && br > 0) br--;
    else if (
      ch === ">" &&
      i > 0 &&
      /[A-Za-z0-9_.\u0080-\uFFFF]/.test(segment[i - 1] ?? "") &&
      segment.indexOf("]", i + 1) > i
    ) {
      inAsym = true;
    } else if (ch === "&" && sq === 0 && par === 0 && br === 0) {
      const part = segment.slice(start, i).trim();
      if (!part) throw new DiagramInputError("Missing node around `&`");
      parts.push(part);
      start = i + 1;
    }
  }
  const tail = segment.slice(start).trim();
  if (!tail) throw new DiagramInputError("Missing node around `&`");
  parts.push(tail);
  return parts;
}
