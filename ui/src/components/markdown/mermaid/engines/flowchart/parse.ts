/**
 * Flowchart and graph parser following Mermaid 12.3.0 syntax: headers (`flowchart`, `flowchart-v2`,
 * `flowchart-elk`, `graph`), directions (`TB`/`TD`/`BT`/`LR`/`RL`), classic and `@{ shape: ... }`
 * node shapes, edge styles/arrowheads/labels/IDs/minlen, chaining and `&` grouping, subgraphs
 * (with nested direction and `view: collapsed`), inline comments, entity codes, and strict security
 * refusals for directives, styling, and callbacks.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_EDGES, MAX_NODES, StepBudget, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";
import {
  decodeLabel,
  findNextEdgeOp,
  hasUnquotedClassAttach,
  parseMeta,
  parseNodeToken,
  splitAmpersands,
  type ParsedOp,
} from "./parseTokens";
import type { FlowDirection, FlowEdge, FlowNode, FlowSubgraph, FlowchartModel } from "./types";

const REFUSED_STMT = /^(?:click|style|classDef|linkStyle|callback|link|cssClass)\b|^class\s+/i;
const DIR_RE = /^direction\s+(TB|TD|BT|LR|RL)$/i;
const SUBGRAPH_RE = /^subgraph(?:\s+([\s\S]+))?$/i;

function normDir(raw: string): FlowDirection {
  const up = raw.toUpperCase();
  if (up === "TD" || up === "TB") return "TB";
  if (up === "BT" || up === "LR" || up === "RL") return up;
  throw new DiagramInputError(`Invalid flowchart direction: ${raw}`);
}

function stripInlineComment(line: string): string {
  let inQuote = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i] ?? "";
    if (inQuote) {
      if (ch === "\\" && i + 1 < line.length) i++;
      else if (ch === '"') inQuote = false;
      continue;
    }
    if (ch === '"') {
      inQuote = true;
    } else if (ch === "%" && line[i + 1] === "%") {
      return line.slice(0, i).trimEnd();
    }
  }
  return line;
}

function advanceLineState(
  segment: string,
  startInQuote: boolean,
  startBraceDepth: number,
): { inQuote: boolean; braceDepth: number } {
  let inQuote = startInQuote;
  let braceDepth = startBraceDepth;
  for (let k = 0; k < segment.length; k++) {
    const ch = segment[k] ?? "";
    if (inQuote) {
      if (ch === "\\" && k + 1 < segment.length) k++;
      else if (ch === '"') inQuote = false;
      continue;
    }
    if (ch === '"') {
      inQuote = true;
    } else if (ch === "@" && segment[k + 1] === "{") {
      braceDepth++;
      k++;
    } else if (braceDepth > 0) {
      if (ch === "{") braceDepth++;
      else if (ch === "}") braceDepth--;
    }
  }
  return { inQuote, braceDepth };
}

function mergeLogicalLines(rawLines: readonly string[], budget: StepBudget): string[] {
  const out: string[] = [];
  let i = 0;
  let accDescrOpen = false;
  while (i < rawLines.length) {
    budget.tick();
    const cleaned = stripInlineComment(rawLines[i] ?? "").trim();
    i++;
    if (!cleaned) continue;
    if (accDescrOpen) {
      if (cleaned === "}") accDescrOpen = false;
      continue;
    }
    if (/^accDescr\s*\{$/i.test(cleaned)) {
      accDescrOpen = true;
      continue;
    }

    let cur = cleaned;
    let state = advanceLineState(cur, false, 0);
    while ((state.inQuote || state.braceDepth > 0) && i < rawLines.length) {
      budget.tick();
      const next = stripInlineComment(rawLines[i] ?? "").trim();
      i++;
      cur += `\n${next}`;
      state = advanceLineState(next, state.inQuote, state.braceDepth);
    }
    if (state.inQuote) throw new DiagramInputError("Unterminated quoted string");
    if (state.braceDepth > 0) throw new DiagramInputError("Unterminated `@{ ... }` block");
    out.push(cur);
  }
  if (accDescrOpen) throw new DiagramInputError("Unclosed accDescr block");
  return out;
}

function splitStatements(lines: readonly string[], budget: StepBudget): string[] {
  const stmts: string[] = [];
  for (const line of lines) {
    let start = 0;
    let inQuote = false;
    const stack: string[] = [];
    for (let i = 0; i < line.length; i++) {
      budget.tick();
      const ch = line[i] ?? "";
      if (inQuote) {
        if (ch === "\\" && i + 1 < line.length) i++;
        else if (ch === '"') inQuote = false;
        continue;
      }
      if (ch === '"') {
        inQuote = true;
      } else if (ch === "[" || ch === "(" || ch === "{") {
        stack.push(ch);
      } else if (ch === "]" || ch === ")" || ch === "}") {
        if (stack.length > 0) {
          const want = ch === "]" ? "[" : ch === ")" ? "(" : "{";
          if (stack.pop() !== want) throw new DiagramInputError("Mismatched closing bracket");
        } else if (ch !== "]") {
          throw new DiagramInputError("Unexpected closing delimiter");
        }
      } else if (ch === ";" && stack.length === 0) {
        const chunk = line.slice(start, i).trim();
        if (chunk) stmts.push(chunk);
        start = i + 1;
      }
    }
    if (stack.length > 0) throw new DiagramInputError("Unclosed shape delimiter");
    const tail = line.slice(start).trim();
    if (tail) stmts.push(tail);
  }
  return stmts;
}

function parseSubgraphHeader(
  spec: string,
  nextAutoId: () => string,
): { id: string; title: string } {
  const explicitMatch = /^([A-Za-z0-9_.\u0080-\uFFFF-]+)\s*\[([\s\S]+)\]$/.exec(spec);
  if (explicitMatch?.[1]) {
    return { id: explicitMatch[1], title: decodeLabel(explicitMatch[2] ?? "") };
  }
  if (!spec) {
    return { id: nextAutoId(), title: "" };
  }
  if (spec.startsWith('"') && spec.endsWith('"')) {
    return { id: nextAutoId(), title: decodeLabel(spec) };
  }
  return { id: spec, title: decodeLabel(spec) };
}

/**
 * Parse `flowchart` / `graph` source (text or PreparedSource) into a FlowchartModel.
 * Throws DiagramInputError on syntax errors, unclosed constructs, or refused styling/callbacks,
 * and DiagramLimitError when node, edge, subgraph, or depth bounds are exceeded.
 */
export function parseFlowchart(input: string | PreparedSource): FlowchartModel {
  const budget = new StepBudget();
  const rawLines = linesOf(input, "flowchart");
  const logical = mergeLogicalLines(rawLines, budget);
  const statements = splitStatements(logical, budget);

  const header = statements[0];
  if (!header) throw new DiagramInputError("Empty flowchart");
  const headMatch = /^(?:flowchart(?:-v2|-elk)?|graph)(?:\s+([A-Za-z]+))?$/i.exec(header.trim());
  if (!headMatch) throw new DiagramInputError(`Invalid flowchart header: ${header.slice(0, 40)}`);

  const direction = headMatch[1] ? normDir(headMatch[1]) : "TB";
  const model: FlowchartModel = { direction, nodes: [], edges: [], subgraphs: [] };
  const nodeById = new Map<string, FlowNode>();
  const subgraphById = new Map<string, FlowSubgraph>();
  const edgeById = new Map<string, FlowEdge>();
  const stack: FlowSubgraph[] = [];
  let autoSgSeq = 0;

  const activeSubgraph = (): FlowSubgraph | undefined => stack[stack.length - 1];

  const resolveEndpoint = (rawToken: string): string => {
    const trimmed = rawToken.trim();
    const existingSg = subgraphById.get(trimmed);
    if (existingSg) return existingSg.id;

    const parsed = parseNodeToken(trimmed);
    if (subgraphById.has(parsed.id)) {
      throw new DiagramInputError(`Cannot redefine subgraph ${parsed.id} as a node`);
    }
    let node = nodeById.get(parsed.id);
    const sg = activeSubgraph();
    if (!node) {
      node = {
        id: parsed.id,
        label: parsed.label ?? parsed.id,
        hasExplicitLabel: parsed.label !== undefined,
        shape: parsed.shape ?? "rect",
        metadata: {},
      };
      if (sg) {
        node.subgraphId = sg.id;
        sg.nodeIds.push(node.id);
      }
      nodeById.set(node.id, node);
      model.nodes.push(node);
      checkCount(model.nodes.length, MAX_NODES, "nodes");
    } else {
      if (sg && node.subgraphId === undefined) {
        node.subgraphId = sg.id;
        sg.nodeIds.push(node.id);
      }
      if (parsed.label !== undefined) {
        node.label = parsed.label;
        node.hasExplicitLabel = true;
      }
      if (parsed.shape !== undefined) node.shape = parsed.shape;
    }
    if (parsed.meta) Object.assign(node.metadata, parsed.meta);
    return node.id;
  };

  for (const stmt of statements.slice(1)) {
    budget.tick();
    if (/^(?:accTitle|accDescr)\b/i.test(stmt)) continue;
    if (REFUSED_STMT.test(stmt)) {
      throw new DiagramInputError(`Not drawn: ${stmt.split(/\s/)[0] ?? ""}`);
    }
    if (hasUnquotedClassAttach(stmt)) {
      throw new DiagramInputError("Class attachment (:::) is not supported");
    }

    const dirMatch = DIR_RE.exec(stmt);
    if (dirMatch?.[1]) {
      const d = normDir(dirMatch[1]);
      const sg = activeSubgraph();
      if (sg) sg.direction = d;
      else model.direction = d;
      continue;
    }

    const sgMatch = SUBGRAPH_RE.exec(stmt);
    if (sgMatch) {
      if (stack.length >= MAX_DEPTH) {
        throw new DiagramLimitError(
          `Diagram too large: subgraphs nested more than ${String(MAX_DEPTH)} deep`,
        );
      }
      const { id, title } = parseSubgraphHeader((sgMatch[1] ?? "").trim(), () => {
        autoSgSeq++;
        return `__sg_${String(autoSgSeq)}`;
      });
      if (subgraphById.has(id)) {
        throw new DiagramInputError(`Duplicate subgraph identifier: ${id}`);
      }
      const parent = activeSubgraph();
      const existingNode = nodeById.get(id);
      if (existingNode) {
        const explicit =
          existingNode.hasExplicitLabel ||
          existingNode.shape !== "rect" ||
          Object.keys(existingNode.metadata).length > 0;
        if (explicit) {
          throw new DiagramInputError(`Identifier ${id} is already used by a node`);
        }
        // Promote an earlier forward reference (`Start --> one` before `subgraph one`) to this subgraph.
        nodeById.delete(id);
        const idx = model.nodes.indexOf(existingNode);
        if (idx >= 0) model.nodes.splice(idx, 1);
        if (existingNode.subgraphId !== undefined) {
          const prevSg = subgraphById.get(existingNode.subgraphId);
          if (prevSg) {
            const ni = prevSg.nodeIds.indexOf(id);
            if (ni >= 0) prevSg.nodeIds.splice(ni, 1);
          }
        }
      }
      const sg: FlowSubgraph = {
        id,
        title,
        collapsed: false,
        nodeIds: [],
        childSubgraphIds: [],
      };
      if (parent) {
        sg.parentId = parent.id;
        parent.childSubgraphIds.push(sg.id);
      }
      subgraphById.set(sg.id, sg);
      model.subgraphs.push(sg);
      checkCount(model.subgraphs.length, MAX_NODES, "subgraphs");
      stack.push(sg);
      continue;
    }

    if (stmt === "end") {
      if (stack.length === 0) throw new DiagramInputError("`end` without matching `subgraph`");
      stack.pop();
      continue;
    }

    const firstOp = findNextEdgeOp(stmt, 0, budget);
    if (firstOp) {
      const groups: string[][] = [];
      const ops: ParsedOp[] = [];
      let cursor = 0;
      let op: ParsedOp | undefined = firstOp;
      while (op) {
        const lhs = stmt.slice(cursor, op.index).trim();
        if (!lhs)
          throw new DiagramInputError(`Missing source node before edge: ${stmt.slice(0, 40)}`);
        groups.push(splitAmpersands(lhs, budget).map(resolveEndpoint));
        ops.push(op);
        cursor = op.index + op.length;
        op = findNextEdgeOp(stmt, cursor, budget);
      }
      const tail = stmt.slice(cursor).trim();
      if (!tail) throw new DiagramInputError(`Dangling edge without target: ${stmt.slice(0, 40)}`);
      groups.push(splitAmpersands(tail, budget).map(resolveEndpoint));

      for (let idx = 0; idx < ops.length; idx++) {
        const edgeOp = ops[idx];
        const fromList = groups[idx];
        const toList = groups[idx + 1];
        if (!edgeOp || !fromList || !toList) continue;
        for (const from of fromList) {
          for (const to of toList) {
            const edge: FlowEdge = {
              from,
              to,
              label: edgeOp.label,
              style: edgeOp.style,
              startMark: edgeOp.startMark,
              endMark: edgeOp.endMark,
              minlen: edgeOp.minlen,
            };
            if (edgeOp.edgeId !== undefined) {
              edge.id = edgeOp.edgeId;
              edgeById.set(edgeOp.edgeId, edge);
            }
            model.edges.push(edge);
            checkCount(model.edges.length, MAX_EDGES, "edges");
          }
        }
      }
      continue;
    }

    // Standalone metadata statement (`id@{ ... }`) targeting a subgraph, edge, or node.
    const metaOnly = /^([A-Za-z0-9_.\u0080-\uFFFF-]+)@\{([\s\S]*)\}$/.exec(stmt);
    if (metaOnly?.[1]) {
      const targetId = metaOnly[1];
      const meta = parseMeta(metaOnly[2] ?? "");
      const sg = subgraphById.get(targetId);
      if (sg) {
        if (meta.view === "collapsed") sg.collapsed = true;
        else if (meta.view === "expanded") sg.collapsed = false;
        if (meta.label !== undefined) sg.title = meta.label;
        continue;
      }
      if (edgeById.has(targetId)) continue;
      resolveEndpoint(stmt);
      continue;
    }

    resolveEndpoint(stmt);
  }

  if (stack.length > 0) throw new DiagramInputError("Unclosed `subgraph`");
  if (model.nodes.length === 0 && model.subgraphs.length === 0) {
    throw new DiagramInputError("A flowchart needs at least one node or subgraph");
  }
  return model;
}
