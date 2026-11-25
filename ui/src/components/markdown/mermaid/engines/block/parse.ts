/**
 * Block diagram parser. Grammar follows Mermaid's block diagram page: `columns N`, composite
 * `block[:id[:span]] ... end`, `space[:span]`, all 13 node shapes with `:span`, directional block
 * arrows `<["label"]>(dir)`, multi-item lines, inline `%%` comments, and edges with optional
 * labels. Styling, `class`, `:::`, links and callbacks are refused.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** Supported leaf block shapes. */
export type BlockShape =
  | "rect"
  | "round"
  | "stadium"
  | "subroutine"
  | "cylinder"
  | "circle"
  | "doublecircle"
  | "asymmetric"
  | "rhombus"
  | "hexagon"
  | "parallelogram-r"
  | "parallelogram-l"
  | "trapezoid"
  | "trapezoid-alt";

/** Allowed arrow directions inside `<[label]>(...)`. */
export type ArrowDir = "left" | "right" | "up" | "down" | "x" | "y";

/** One placed item inside a block container. */
export type BlockItem =
  | { kind: "node"; id: string; label: string; shape: BlockShape; span: number }
  | { kind: "arrow"; id: string; label: string; dirs: ArrowDir[]; span: number }
  | { kind: "space"; span: number }
  | BlockGroup;

/** A root or nested composite block container. */
export interface BlockGroup {
  kind: "group";
  id?: string | undefined;
  span: number;
  columns?: number | undefined;
  items: BlockItem[];
}

/** A connector edge between two declared blocks. */
export interface BlockEdge {
  from: string;
  to: string;
  label: string;
  dashed: boolean;
  fromArrow: boolean;
  toArrow: boolean;
}

/** A parsed block diagram. */
export interface BlockModel {
  root: BlockGroup;
  edges: BlockEdge[];
}

/** Maximum grid columns a block diagram may allocate or traverse. */
export const MAX_BLOCK_COLUMNS = 480;
const REFUSED = /^(?:style|classDef|class|click|link|linkStyle|callback)\b/i;
const VALID_DIRS = new Set<ArrowDir>(["left", "right", "up", "down", "x", "y"]);

function decodeLabel(raw: string): string {
  const trimmed = raw.trim();
  let text = trimmed;
  if (trimmed.startsWith('"')) {
    if (trimmed.length < 2 || !trimmed.endsWith('"')) {
      throw new DiagramInputError("Unterminated block label quote");
    }
    text = trimmed.slice(1, -1).replace(/\\(.)/g, "$1");
  }
  return text.replace(/&nbsp;/gi, " ");
}

function stripInlineComment(raw: string): string {
  let inQuotes = false;
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i];
    if (ch === "\\") {
      i++;
      continue;
    }
    if (ch === '"') inQuotes = !inQuotes;
    else if (!inQuotes && ch === "%" && raw[i + 1] === "%") return raw.slice(0, i).trim();
  }
  return raw.trim();
}

function hasUnquotedTripleColon(line: string): boolean {
  let inQuotes = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === "\\") {
      i++;
      continue;
    }
    if (ch === '"') inQuotes = !inQuotes;
    else if (!inQuotes && line.startsWith(":::", i)) return true;
  }
  return false;
}

/** Scan delimited text starting right after `open`, respecting quotes, up to `close`. */
function readUntil(
  line: string,
  start: number,
  close: string,
  allowQuoteOnly = false,
): { content: string; next: number } {
  let inQuotes = false;
  let i = start;
  while (i < line.length) {
    const ch = line[i];
    if (ch === "\\") {
      i += 2;
      continue;
    }
    if (ch === '"') {
      inQuotes = !inQuotes;
      i++;
      continue;
    }
    if (!inQuotes && line.startsWith(close, i)) {
      return { content: line.slice(start, i), next: i + close.length };
    }
    i++;
  }
  if (allowQuoteOnly) throw new DiagramInputError(`Unclosed shape delimiter ${close}`);
  throw new DiagramInputError(`Unclosed block delimiter ${close}`);
}

interface ParsedToken {
  item: Exclude<BlockItem, BlockGroup>;
  next: number;
}

function readSpan(line: string, pos: number): { span: number; next: number } {
  if (line[pos] !== ":") return { span: 1, next: pos };
  const m = /^:(\d+)/.exec(line.slice(pos));
  if (!m)
    throw new DiagramInputError(`Invalid block column span near ${line.slice(pos, pos + 16)}`);
  const span = Number(m[1]);
  if (!Number.isSafeInteger(span) || span < 1) {
    throw new DiagramInputError("Block column span must be at least 1");
  }
  checkCount(span, MAX_BLOCK_COLUMNS, "block columns");
  return { span, next: pos + m[0].length };
}

function parseBlockToken(line: string, startPos: number): ParsedToken {
  let pos = startPos;
  while (pos < line.length && /\s/.test(line[pos] ?? "")) pos++;
  const idMatch = /^[A-Za-z_](?:[A-Za-z0-9_.]|-(?![->]))*/.exec(line.slice(pos));
  if (!idMatch) {
    throw new DiagramInputError(`Unreadable block item near ${line.slice(pos, pos + 24)}`);
  }
  const id = idMatch[0];
  pos += id.length;

  if (id === "space" && (pos >= line.length || /[\s:]/.test(line[pos] ?? ""))) {
    const { span, next } = readSpan(line, pos);
    return { item: { kind: "space", span }, next };
  }

  // Block arrow: id<["label"]>(dir1[, dir2])[:span]
  if (line.startsWith("<[", pos)) {
    const inner = readUntil(line, pos + 2, "]>", true);
    pos = inner.next;
    if (line[pos] !== "(") throw new DiagramInputError(`Block arrow ${id} missing (direction)`);
    const dirPart = readUntil(line, pos + 1, ")", true);
    pos = dirPart.next;
    const dirs = dirPart.content
      .split(",")
      .map((d) => d.trim().toLowerCase() as ArrowDir)
      .filter(Boolean);
    if (dirs.length === 0 || dirs.some((d) => !VALID_DIRS.has(d))) {
      throw new DiagramInputError(`Invalid block arrow direction: ${dirPart.content}`);
    }
    const { span, next } = readSpan(line, pos);
    return { item: { kind: "arrow", id, label: decodeLabel(inner.content), dirs, span }, next };
  }

  let shape: BlockShape = "rect";
  let label = id;
  const openRules: [string, string, BlockShape][] = [
    ["(((", ")))", "doublecircle"],
    ["((", "))", "circle"],
    ["([", "])", "stadium"],
    ["[[", "]]", "subroutine"],
    ["[(", ")]", "cylinder"],
    ["{{", "}}", "hexagon"],
    ["[/", "/]", "parallelogram-r"],
    ["[/", "\\]", "trapezoid"],
    ["[\\", "\\]", "parallelogram-l"],
    ["[\\", "/]", "trapezoid-alt"],
    ["[", "]", "rect"],
    ["(", ")", "round"],
    ["{", "}", "rhombus"],
    [">", "]", "asymmetric"],
  ];

  // For [/ and [\, peek ahead for exact matching closing bracket sequence (/] vs \]).
  if (line.startsWith("[/", pos) || line.startsWith("[\\", pos)) {
    const openTok = line.slice(pos, pos + 2);
    const bodyStart = pos + 2;
    let inQuotes = false;
    let matchedClose: string | undefined;
    let closeAt = -1;
    for (let i = bodyStart; i < line.length; i++) {
      const ch = line[i];
      if (ch === "\\") {
        if (!inQuotes && line[i + 1] === "]") {
          matchedClose = "\\]";
          closeAt = i;
          break;
        }
        i++;
        continue;
      }
      if (ch === '"') inQuotes = !inQuotes;
      else if (!inQuotes && ch === "/" && line[i + 1] === "]") {
        matchedClose = "/]";
        closeAt = i;
        break;
      }
    }
    if (closeAt < 0 || !matchedClose) {
      throw new DiagramInputError(`Unclosed shape delimiter ${openTok}`);
    }
    shape =
      openTok === "[/"
        ? matchedClose === "/]"
          ? "parallelogram-r"
          : "trapezoid"
        : matchedClose === "\\]"
          ? "parallelogram-l"
          : "trapezoid-alt";
    label = decodeLabel(line.slice(bodyStart, closeAt));
    pos = closeAt + 2;
  } else {
    for (const [open, close, kind] of openRules) {
      if (open === "[/" || open === "[\\") continue;
      if (line.startsWith(open, pos)) {
        const res = readUntil(line, pos + open.length, close, true);
        shape = kind;
        label = decodeLabel(res.content);
        pos = res.next;
        break;
      }
    }
  }

  const { span, next } = readSpan(line, pos);
  return { item: { kind: "node", id, label, shape, span }, next };
}

/** Find a depth-0 edge connector if this line represents a link between two blocks. */
function tryParseEdgeLine(line: string):
  | {
      leftToken: ParsedToken;
      edge: Omit<BlockEdge, "from" | "to">;
      rightToken: ParsedToken;
    }
  | undefined {
  // Parse left token first; if there is an edge operator immediately after it, treat line as edge.
  let left: ParsedToken;
  try {
    left = parseBlockToken(line, 0);
  } catch {
    return undefined;
  }
  const rest = line.slice(left.next).trim();
  if (rest === "") return undefined;

  let m = /^--\s*(?:"([^"]*)"|([^>-]+?))\s*(---|-->)\s*(.+)$/.exec(rest);
  if (m) {
    const label = decodeLabel(m[1] !== undefined ? `"${m[1]}"` : (m[2] ?? ""));
    const toArrow = m[3] === "-->";
    const right = parseBlockToken(m[4] ?? "", 0);
    if ((m[4] ?? "").slice(right.next).trim() !== "") return undefined;
    return {
      leftToken: left,
      edge: { label, dashed: false, fromArrow: false, toArrow },
      rightToken: right,
    };
  }

  m = /^(<)?(---|--|-\.-|===|==)(>)?(?:\|([^|]*)\|)?\s*(.+)$/.exec(rest);
  if (m) {
    const fromArrow = Boolean(m[1]);
    const mid = m[2] ?? "--";
    const toArrow = Boolean(m[3]);
    if (mid === "--" && !fromArrow && !toArrow) return undefined;
    if (mid === "==" && !fromArrow && !toArrow) return undefined;
    const label = decodeLabel(m[4] ?? "");
    const tail = m[5] ?? "";
    const right = parseBlockToken(tail, 0);
    if (tail.slice(right.next).trim() !== "") return undefined;
    return {
      leftToken: left,
      edge: { label, dashed: mid === "-.-", fromArrow, toArrow },
      rightToken: right,
    };
  }

  return undefined;
}

/** Parse `block` or `block-beta` source; throws DiagramInputError or DiagramLimitError on problems. */
export function parseBlock(input: string | PreparedSource): BlockModel {
  const [, ...body] = linesOf(input, "block");
  const root: BlockGroup = { kind: "group", span: 1, items: [] };
  const stack: BlockGroup[] = [root];
  const edges: BlockEdge[] = [];
  const declared = new Map<string, BlockItem>();
  let nodeCount = 0;
  let spaceSlots = 0;

  const registerItem = (item: BlockItem): void => {
    if (item.kind === "space") {
      spaceSlots += item.span;
      checkCount(spaceSlots, MAX_BLOCK_COLUMNS, "space columns");
      return;
    }
    nodeCount++;
    checkCount(nodeCount, MAX_NODES, "blocks");
    if (item.id !== undefined) {
      if (declared.has(item.id)) {
        throw new DiagramInputError(`Duplicate block id: ${item.id}`);
      }
      declared.set(item.id, item);
    }
  };

  for (const raw of body) {
    const line = stripInlineComment(raw);
    if (line === "") continue;
    if (REFUSED.test(line) || hasUnquotedTripleColon(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }
    if (/^acc(?:Title|Descr)\b/i.test(line)) continue;

    const current = stack[stack.length - 1] as BlockGroup;

    const colMatch = /^columns\s+(auto|\d+)$/i.exec(line);
    if (colMatch) {
      const arg = (colMatch[1] ?? "").toLowerCase();
      if (arg === "auto") {
        current.columns = undefined;
      } else {
        const count = Number(arg);
        if (!Number.isSafeInteger(count) || count < 1) {
          throw new DiagramInputError("columns count must be at least 1");
        }
        checkCount(count, MAX_NODES, "columns");
        current.columns = count;
      }
      continue;
    }

    const blockOpen = /^block(?::([A-Za-z_][A-Za-z0-9_.-]*)(?::(\d+))?)?$/i.exec(line);
    if (blockOpen) {
      if (stack.length > MAX_DEPTH) {
        throw new DiagramLimitError(
          `Diagram too large: blocks nested more than ${String(MAX_DEPTH)} deep`,
        );
      }
      const span = blockOpen[2] ? Number(blockOpen[2]) : 1;
      if (!Number.isSafeInteger(span) || span < 1) {
        throw new DiagramInputError("Composite block span must be at least 1");
      }
      checkCount(span, MAX_BLOCK_COLUMNS, "block columns");
      const group: BlockGroup = { kind: "group", id: blockOpen[1], span, items: [] };
      registerItem(group);
      current.items.push(group);
      stack.push(group);
      continue;
    }

    if (/^end$/i.test(line)) {
      if (stack.length === 1) throw new DiagramInputError("`end` without an open block");
      stack.pop();
      continue;
    }

    const edgeParsed = tryParseEdgeLine(line);
    if (edgeParsed) {
      const { leftToken, edge, rightToken } = edgeParsed;
      if (leftToken.item.kind === "space" || rightToken.item.kind === "space") {
        throw new DiagramInputError("Cannot connect an edge to a space block");
      }
      const fromId = leftToken.item.id;
      const toId = rightToken.item.id;
      const fromExisting = declared.get(fromId);
      const toExisting = declared.get(toId);
      if (!fromExisting) throw new DiagramInputError(`Unknown block in edge: ${fromId}`);
      if (!toExisting) throw new DiagramInputError(`Unknown block in edge: ${toId}`);
      if (
        leftToken.item.kind === "node" &&
        fromExisting.kind === "node" &&
        leftToken.item.label !== fromId
      ) {
        fromExisting.label = leftToken.item.label;
        fromExisting.shape = leftToken.item.shape;
      }
      if (
        rightToken.item.kind === "node" &&
        toExisting.kind === "node" &&
        rightToken.item.label !== toId
      ) {
        toExisting.label = rightToken.item.label;
        toExisting.shape = rightToken.item.shape;
      }
      edges.push({ from: fromId, to: toId, ...edge });
      checkCount(edges.length, MAX_EDGES, "edges");
      continue;
    }

    // Multiple block items on one line.
    let pos = 0;
    while (pos < line.length) {
      while (pos < line.length && /\s/.test(line[pos] ?? "")) pos++;
      if (pos >= line.length) break;
      const parsed = parseBlockToken(line, pos);
      registerItem(parsed.item);
      current.items.push(parsed.item);
      pos = parsed.next;
    }
  }

  if (stack.length > 1) throw new DiagramInputError("Unclosed block");
  if (nodeCount === 0) throw new DiagramInputError("A block diagram needs blocks");
  return { root, edges };
}
