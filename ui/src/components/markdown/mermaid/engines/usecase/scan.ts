/** Linear token scans for use-case metadata and labelled associations; called by the parser. */
import { DiagramInputError } from "../../types";

/** Split at commas with an even number of quotes to their right, preserving the metadata grammar. */
export function parseSimpleMeta(raw: string): Record<string, string> {
  const inner = raw.trim();
  const parts: string[] = [];
  let end = inner.length;
  let quoted = false;
  // The former lookahead rescanned the entire suffix at every comma. Count its quote parity once.
  for (let i = inner.length - 1; i >= 0; i--) {
    if (inner[i] === '"') quoted = !quoted;
    else if (inner[i] === "," && !quoted) {
      parts.push(inner.slice(i + 1, end));
      end = i;
    }
  }
  parts.push(inner.slice(0, end));
  const out: Record<string, string> = {};
  for (const part of parts.reverse()) {
    const chunk = part.trim();
    if (!chunk) continue;
    const colon = chunk.indexOf(":");
    if (colon <= 0) throw new DiagramInputError(`Invalid metadata entry: ${chunk}`);
    const key = chunk.slice(0, colon).trim();
    let val = chunk.slice(colon + 1).trim();
    if (val.startsWith('"') && val.endsWith('"') && val.length >= 2) val = val.slice(1, -1);
    out[key] = val;
  }
  return out;
}

interface LabelledAssociation {
  from: string;
  edgeId: string | undefined;
  label: string;
  rightOp: string;
  to: string;
}

/** Read the final destination and operator backwards so label whitespace cannot trade regex matches. */
export function readLabelledAssociation(line: string): LabelledAssociation | undefined {
  const head = /^([A-Za-z0-9_]+)\s+(?:([A-Za-z0-9_]+)@)?--/.exec(line);
  if (!head) return undefined;
  const labelStart = head[0].length;
  let i = line.length;
  while (i > labelStart && /[A-Za-z0-9_]/.test(line[i - 1] ?? "")) i--;
  if (i === line.length || !/\s/.test(line[i - 1] ?? "")) return undefined;
  const to = line.slice(i);
  while (i > labelStart && /\s/.test(line[i - 1] ?? "")) i--;
  const opEnd = i;
  if (line[i - 1] === ">") i--;
  const dashEnd = i;
  while (i > labelStart && line[i - 1] === "-") i--;
  if (dashEnd - i < 2) return undefined;
  const label = line.slice(labelStart, i);
  // Two separators and at least one label character, even when the label is entirely whitespace.
  if (label.length < 3 || !/^\s/.test(label) || !/\s$/.test(label)) return undefined;
  return {
    from: head[1] ?? "",
    edgeId: head[2],
    label: label.trim(),
    rightOp: line.slice(i, opEnd),
    to,
  };
}

interface BoundaryHead {
  quotedTitle: string | undefined;
  id: string | undefined;
  squareTitle: string | undefined;
  roundTitle: string | undefined;
  rest: string;
}

/** Preserve the boundary grammar's first closing delimiter whose suffix fits on one line. */
export function readBoundaryHead(line: string): BoundaryHead | undefined {
  const head = /^systemBoundary\s+(?:"([^"]+)"|([A-Za-z0-9_]+))/.exec(line);
  if (!head) return undefined;
  const start = head[0].length;
  let lastBreak = -1;
  for (let i = start; i < line.length; i++) {
    if (/[\r\n\u2028\u2029]/.test(line[i] ?? "")) lastBreak = i;
  }
  const result: BoundaryHead = {
    quotedTitle: head[1],
    id: head[2],
    squareTitle: undefined,
    roundTitle: undefined,
    rest: line.slice(start),
  };
  if (head[2] !== undefined) {
    let open = start;
    while (open < line.length && /\s/.test(line[open] ?? "")) open++;
    const close = line[open] === "[" ? "]" : line[open] === "(" ? ")" : undefined;
    if (close !== undefined) {
      const end = line.indexOf(close, Math.max(open + 2, lastBreak + 1));
      if (end >= 0) {
        if (close === "]") result.squareTitle = line.slice(open + 1, end);
        else result.roundTitle = line.slice(open + 1, end);
        result.rest = line.slice(end + 1);
        return result;
      }
    }
  }
  return lastBreak < start ? result : undefined;
}
