/**
 * Swimlane diagram parser. Grammar follows Mermaid's swimlanes page: `swimlane-beta [direction]`,
 * top-level `subgraph` bands (optional `id [Label]`), nested plain `subgraph` boxes, shared-node
 * first-lane ownership (dropping lanes emptied solely by deduplication while retaining authored
 * empty lanes), flowchart node shapes, solid/dotted/thick edges with labels and chaining.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { clampLabel } from "../../measure";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** Flow direction of a swimlane diagram. */
export type SwimlaneDirection = "TB" | "BT" | "LR" | "RL";

/** Supported flowchart-style node shapes inside swimlanes. */
export type SwimlaneShape =
  "rect" | "round" | "stadium" | "diamond" | "circle" | "subroutine" | "cylinder" | "hexagon";

/** A nested box declared inside a swimlane. */
export interface SwimlaneBox {
  id: string;
  label: string;
  nodeIds: string[];
}

/** A top-level swimlane band. */
export interface SwimlaneLane {
  id: string;
  label: string;
  nodeIds: string[];
  nestedBoxes: SwimlaneBox[];
}

/** A node belonging to a single swimlane. */
export interface SwimlaneNode {
  id: string;
  label: string;
  shape: SwimlaneShape;
  laneId: string;
  boxId?: string;
}

/** A directed or undirected connector between two nodes. */
export interface SwimlaneEdge {
  from: string;
  to: string;
  label: string;
  style: "solid" | "dotted" | "thick";
  arrowStart: boolean;
  arrowEnd: boolean;
}

/** A parsed swimlane diagram. */
export interface SwimlaneModel {
  direction: SwimlaneDirection;
  lanes: SwimlaneLane[];
  nodes: SwimlaneNode[];
  edges: SwimlaneEdge[];
}

const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style|linkStyle)\b|:::/i;

function unquote(raw: string): string {
  const t = raw.trim();
  if (t.startsWith('"') && t.endsWith('"') && t.length >= 2) return t.slice(1, -1);
  return t;
}

function parseNodeToken(raw: string): { id: string; label?: string; shape?: SwimlaneShape } {
  const t = raw.trim();
  if (!t || /:::/.test(t)) {
    throw new DiagramInputError(`Invalid swimlane node token: ${raw.slice(0, 30)}`);
  }
  const patterns: [RegExp, SwimlaneShape][] = [
    [/^([A-Za-z0-9_.-]+)\(\[([\s\S]+)\]\)$/, "stadium"],
    [/^([A-Za-z0-9_.-]+)\(\(([\s\S]+)\)\)$/, "circle"],
    [/^([A-Za-z0-9_.-]+)\[\[([\s\S]+)\]\]$/, "subroutine"],
    [/^([A-Za-z0-9_.-]+)\[\(([\s\S]+)\)\]$/, "cylinder"],
    [/^([A-Za-z0-9_.-]+)\{\{([\s\S]+)\}\}$/, "hexagon"],
    [/^([A-Za-z0-9_.-]+)\[([\s\S]+)\]$/, "rect"],
    [/^([A-Za-z0-9_.-]+)\(([\s\S]+)\)$/, "round"],
    [/^([A-Za-z0-9_.-]+)\{([\s\S]+)\}$/, "diamond"],
  ];
  for (const [re, shape] of patterns) {
    const m = re.exec(t);
    if (m?.[1]) {
      return { id: m[1], label: clampLabel(unquote(m[2] ?? "")), shape };
    }
  }
  if (/^[A-Za-z0-9_.-]+$/.test(t)) return { id: t };
  throw new DiagramInputError(`Unreadable swimlane node: ${t.slice(0, 40)}`);
}

interface ParsedOp {
  index: number;
  length: number;
  label: string;
  style: "solid" | "dotted" | "thick";
  arrowStart: boolean;
  arrowEnd: boolean;
}

/** Read a labelled connector up to its first character forbidden in the label. */
function labelledEdgeAt(text: string, index: number): ParsedOp | undefined {
  let marker: string;
  let blocked: string;
  let endings: readonly string[];
  if (text.startsWith("--", index)) {
    marker = "--";
    blocked = "->";
    endings = ["-->", "---"];
  } else if (text.startsWith("-.", index)) {
    marker = "-.";
    blocked = ".";
    endings = [".->", ".-"];
  } else if (text.startsWith("==", index)) {
    marker = "==";
    blocked = "=>";
    endings = ["==>", "==="];
  } else return undefined;

  const firstSpace = index + marker.length;
  if (!/\s/.test(text[firstSpace] ?? "")) return undefined;
  const labelStart = firstSpace + 1;
  for (let end = labelStart; end < text.length; end++) {
    if (!blocked.includes(text[end] ?? "")) continue;
    const endToken = endings.find((token) => text.startsWith(token, end));
    if (!endToken || end - labelStart < 2 || !/\s/.test(text[end - 1] ?? "")) return undefined;
    return {
      index,
      length: end + endToken.length - index,
      label: clampLabel(unquote(text.slice(labelStart, end - 1).trim())),
      style: endToken.startsWith(".") ? "dotted" : endToken.startsWith("=") ? "thick" : "solid",
      arrowStart: false,
      arrowEnd: endToken.endsWith(">"),
    };
  }
  return undefined;
}

function findNextEdgeOp(text: string, fromIndex: number): ParsedOp | undefined {
  let depthSquare = 0;
  let depthParen = 0;
  let depthBrace = 0;
  let inQuote = false;

  for (let i = fromIndex; i < text.length; i++) {
    const ch = text[i] ?? "";
    if (inQuote) {
      if (ch === "\\" && i + 1 < text.length) {
        i++;
        continue;
      }
      if (ch === '"') inQuote = false;
      continue;
    }
    if (ch === '"') {
      inQuote = true;
      continue;
    }
    if (ch === "[") depthSquare++;
    else if (ch === "]" && depthSquare > 0) depthSquare--;
    else if (ch === "(") depthParen++;
    else if (ch === ")" && depthParen > 0) depthParen--;
    else if (ch === "{") depthBrace++;
    else if (ch === "}" && depthBrace > 0) depthBrace--;

    if (depthSquare > 0 || depthParen > 0 || depthBrace > 0) continue;

    const slice = text.slice(i);
    const labelled = labelledEdgeAt(text, i);
    if (labelled) return labelled;

    const opMatch = /^(<-->|-\.->|-\.-|==>|===|-->|---)(?:\|([^|]*)\|)?/.exec(slice);
    if (opMatch?.[1]) {
      const tok = opMatch[1];
      return {
        index: i,
        length: opMatch[0].length,
        label: clampLabel(unquote(opMatch[2] ?? "")),
        style: tok.startsWith("-.") ? "dotted" : tok.startsWith("=") ? "thick" : "solid",
        arrowStart: tok.startsWith("<"),
        arrowEnd: tok.endsWith(">"),
      };
    }
  }
  return undefined;
}

function splitEdgeChain(
  line: string,
): { nodes: string[]; ops: Omit<ParsedOp, "index" | "length">[] } | undefined {
  const first = findNextEdgeOp(line, 0);
  if (!first) return undefined;
  const nodes: string[] = [];
  const ops: Omit<ParsedOp, "index" | "length">[] = [];
  let cursor = 0;
  let op: ParsedOp | undefined = first;

  while (op) {
    const lhs = line.slice(cursor, op.index).trim();
    if (!lhs) throw new DiagramInputError(`Missing source node before edge: ${line.slice(0, 40)}`);
    nodes.push(lhs);
    ops.push({
      label: op.label,
      style: op.style,
      arrowStart: op.arrowStart,
      arrowEnd: op.arrowEnd,
    });
    cursor = op.index + op.length;
    op = findNextEdgeOp(line, cursor);
  }

  const tail = line.slice(cursor).trim();
  if (!tail) throw new DiagramInputError(`Dangling edge without target node: ${line.slice(0, 40)}`);
  nodes.push(tail);
  return { nodes, ops };
}

/** Parse `swimlane-beta` source (text or prepared lines); throws DiagramInputError on invalid syntax. */
export function parseSwimlane(input: string | PreparedSource): SwimlaneModel {
  const [header, ...body] = linesOf(input, "swimlane");
  const headerMatch = /^swimlane-beta(?:\s+(TB|TD|BT|LR|RL))?\s*$/i.exec((header ?? "").trim());
  if (!headerMatch) {
    throw new DiagramInputError(`Invalid swimlane header: ${(header ?? "").slice(0, 40)}`);
  }
  const rawDir = (headerMatch[1] ?? "TB").toUpperCase();
  const direction = (rawDir === "TD" ? "TB" : rawDir) as SwimlaneDirection;

  const lanes: (SwimlaneLane & { hadSharedLoss: boolean })[] = [];
  const laneById = new Map<string, SwimlaneLane & { hadSharedLoss: boolean }>();
  const nodeById = new Map<string, SwimlaneNode>();
  const edges: SwimlaneEdge[] = [];
  const stack: { kind: "lane" | "box"; id: string; box?: SwimlaneBox }[] = [];
  let accDescrOpen = false;
  let implicitLane: (SwimlaneLane & { hadSharedLoss: boolean }) | undefined;

  const ensureImplicitLane = (): SwimlaneLane & { hadSharedLoss: boolean } => {
    if (!implicitLane) {
      implicitLane = {
        id: "__default",
        label: "Flow",
        nodeIds: [],
        nestedBoxes: [],
        hadSharedLoss: false,
      };
      lanes.push(implicitLane);
      checkCount(lanes.length, MAX_NODES, "swimlanes");
    }
    return implicitLane;
  };

  const currentDeclaredLane = (): (SwimlaneLane & { hadSharedLoss: boolean }) | undefined => {
    const first = stack[0];
    return first?.kind === "lane" ? laneById.get(first.id) : undefined;
  };

  const activeLane = (): SwimlaneLane & { hadSharedLoss: boolean } =>
    currentDeclaredLane() ?? ensureImplicitLane();

  const activeBox = (): SwimlaneBox | undefined => {
    for (let i = stack.length - 1; i >= 0; i--) {
      const entry = stack[i];
      if (entry?.kind === "box") return entry.box;
    }
    return undefined;
  };

  const registerNode = (token: string): string => {
    const parsed = parseNodeToken(token);
    const existing = nodeById.get(parsed.id);
    if (!existing) {
      const lane = activeLane();
      const box = activeBox();
      const node: SwimlaneNode = {
        id: parsed.id,
        label: parsed.label ?? parsed.id,
        shape: parsed.shape ?? "rect",
        laneId: lane.id,
      };
      if (box) {
        node.boxId = box.id;
        box.nodeIds.push(node.id);
      }
      nodeById.set(node.id, node);
      lane.nodeIds.push(node.id);
      checkCount(nodeById.size, MAX_NODES, "nodes");
      return node.id;
    }
    const declaredLane = currentDeclaredLane();
    if (declaredLane && existing.laneId !== declaredLane.id) {
      declaredLane.hadSharedLoss = true;
    }
    if (parsed.label !== undefined && existing.label === existing.id) {
      existing.label = parsed.label;
    }
    if (parsed.shape !== undefined && existing.shape === "rect") {
      existing.shape = parsed.shape;
    }
    return existing.id;
  };

  for (const raw of body) {
    const line = raw.trim().replace(/;$/, "").trim();
    if (line === "") continue;

    if (accDescrOpen) {
      if (line === "}") accDescrOpen = false;
      continue;
    }
    if (/^accDescr\s*\{$/i.test(line)) {
      accDescrOpen = true;
      continue;
    }
    if (/^(?:accTitle|accDescr)\b/i.test(line)) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }

    const subMatch = /^subgraph\s+(.+)$/i.exec(line);
    if (subMatch) {
      if (stack.length >= MAX_DEPTH) {
        throw new DiagramLimitError(
          `Diagram too large: subgraphs nested more than ${String(MAX_DEPTH)} deep`,
        );
      }
      const spec = (subMatch[1] ?? "").trim();
      const idLabel = /^([A-Za-z0-9_.-]+)\s*\[([\s\S]+)\]$/.exec(spec);
      const id = idLabel?.[1] ?? spec;
      const label = clampLabel(unquote(idLabel?.[2] ?? spec));
      if (!id) throw new DiagramInputError("Subgraph missing identifier");

      if (stack.length === 0) {
        if (laneById.has(id)) {
          throw new DiagramInputError(`Duplicate swimlane: ${id}`);
        }
        const lane = {
          id,
          label,
          nodeIds: [],
          nestedBoxes: [],
          hadSharedLoss: false,
        };
        laneById.set(id, lane);
        lanes.push(lane);
        checkCount(lanes.length, MAX_NODES, "swimlanes");
        stack.push({ kind: "lane", id });
      } else {
        const parentLane = activeLane();
        const box: SwimlaneBox = { id, label, nodeIds: [] };
        parentLane.nestedBoxes.push(box);
        stack.push({ kind: "box", id, box });
      }
      continue;
    }

    if (/^end$/i.test(line)) {
      if (stack.length === 0) {
        throw new DiagramInputError("`end` without matching `subgraph`");
      }
      stack.pop();
      continue;
    }

    const chain = splitEdgeChain(line);
    if (chain) {
      const ids = chain.nodes.map(registerNode);
      chain.ops.forEach((op, i) => {
        const from = ids[i] ?? "";
        const to = ids[i + 1] ?? "";
        edges.push({ from, to, ...op });
        checkCount(edges.length, MAX_EDGES, "edges");
      });
      continue;
    }

    registerNode(line);
  }

  if (accDescrOpen) throw new DiagramInputError("Unclosed accDescr block");
  if (stack.length > 0) throw new DiagramInputError("Unclosed `subgraph` in swimlane diagram");
  if (lanes.length === 0) {
    throw new DiagramInputError("A swimlane diagram needs at least one lane or node");
  }

  const incomingTargets = new Set(edges.map((e) => e.to));
  const keptLanes = lanes.filter(
    (lane) => !(lane.nodeIds.length === 0 && lane.hadSharedLoss && !incomingTargets.has(lane.id)),
  );
  if (keptLanes.length === 0) {
    throw new DiagramInputError("A swimlane diagram needs at least one visible lane");
  }

  return {
    direction,
    lanes: keptLanes.map(({ id, label, nodeIds, nestedBoxes }) => ({
      id,
      label,
      nodeIds,
      nestedBoxes,
    })),
    nodes: [...nodeById.values()],
    edges,
  };
}
