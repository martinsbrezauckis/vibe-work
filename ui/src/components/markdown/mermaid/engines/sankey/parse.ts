/**
 * Sankey diagram parser. Grammar follows Mermaid's sankey page: `sankey` / `sankey-beta`,
 * followed by 3-column RFC 4180 CSV rows (`source,target,value`) with quoted fields and `""`
 * quote escaping. Rejects wrong column counts, unclosed quotes, non-positive values, self-loops,
 * and cycles.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_NODES, StepBudget, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** One flow node with its topological column `layer` and incoming/outgoing totals. */
export interface SankeyNode {
  id: string;
  index: number;
  layer: number;
  inFlow: number;
  outFlow: number;
  value: number;
}

/** One directed flow link between node indices. */
export interface SankeyLink {
  source: number;
  target: number;
  value: number;
}

/** A parsed acyclic Sankey flow graph. */
export interface SankeyModel {
  nodes: SankeyNode[];
  links: SankeyLink[];
  maxLayer: number;
}

const NUMBER = /^(?:\d+(?:\.\d+)?|\.\d+)$/;
const REFUSED = /^(?:classDef|cssClass|style|click|link|callback)\b/i;

function parseCsvRow(line: string): [string, string, string] {
  const fields: string[] = [];
  let i = 0;
  while (i <= line.length) {
    if (line[i] === '"') {
      i++;
      let value = "";
      let closed = false;
      while (i < line.length) {
        const ch = line[i];
        if (ch === '"') {
          if (line[i + 1] === '"') {
            value += '"';
            i += 2;
          } else {
            closed = true;
            i++;
            break;
          }
        } else {
          value += ch ?? "";
          i++;
        }
      }
      if (!closed) throw new DiagramInputError("Unterminated quoted field in sankey row");
      while (i < line.length && line[i] === " ") i++;
      if (i < line.length && line[i] !== ",") {
        throw new DiagramInputError(
          `Unexpected character after closing quote: ${line.slice(0, 40)}`,
        );
      }
      fields.push(value.trim());
      if (i === line.length) break;
      i++;
    } else {
      const comma = line.indexOf(",", i);
      const end = comma < 0 ? line.length : comma;
      const rawField = line.slice(i, end);
      if (rawField.includes('"')) {
        throw new DiagramInputError(`Unescaped quote in unquoted field: ${line.slice(0, 40)}`);
      }
      fields.push(rawField.trim());
      if (comma < 0) break;
      i = comma + 1;
      if (i === line.length) {
        fields.push("");
        break;
      }
    }
  }
  if (fields.length !== 3) {
    throw new DiagramInputError(
      `Sankey rows must have 3 CSV columns (found ${String(fields.length)}): ${line.slice(0, 40)}`,
    );
  }
  return [fields[0] ?? "", fields[1] ?? "", fields[2] ?? ""];
}

/** Parse `sankey` / `sankey-beta` source; throws DiagramInputError or DiagramLimitError on invalid/cyclic input. */
export function parseSankey(input: string | PreparedSource): SankeyModel {
  const [header = "", ...body] = linesOf(input, "sankey");
  if (!/^sankey(?:-beta)?\s*$/i.test(header.trim())) {
    throw new DiagramInputError("Unreadable sankey header");
  }

  const nodes: SankeyNode[] = [];
  const nodeIndex = new Map<string, number>();
  const links: SankeyLink[] = [];

  const getNode = (id: string): number => {
    const existing = nodeIndex.get(id);
    if (existing !== undefined) return existing;
    const index = nodes.length;
    nodes.push({ id, index, layer: 0, inFlow: 0, outFlow: 0, value: 0 });
    checkCount(nodes.length, MAX_NODES, "nodes");
    nodeIndex.set(id, index);
    return index;
  };

  for (const raw of body) {
    const line = raw.trim();
    if (!line) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }
    const [srcId, dstId, rawValue] = parseCsvRow(line);
    if (!srcId || !dstId) {
      throw new DiagramInputError(
        `Sankey row needs non-empty source and target: ${line.slice(0, 40)}`,
      );
    }
    if (srcId === dstId) {
      throw new DiagramInputError(`Sankey self-loop is not allowed: ${srcId}`);
    }
    if (!NUMBER.test(rawValue)) {
      throw new DiagramInputError(`Invalid sankey flow value: ${rawValue.slice(0, 30)}`);
    }
    const value = Number(rawValue);
    if (!Number.isFinite(value) || !(value > 0)) {
      throw new DiagramInputError(`Sankey flow value must be positive: ${rawValue}`);
    }
    const source = getNode(srcId);
    const target = getNode(dstId);
    links.push({ source, target, value });
    checkCount(links.length, MAX_EDGES, "links");
    const srcNode = nodes[source];
    const dstNode = nodes[target];
    if (srcNode) srcNode.outFlow += value;
    if (dstNode) dstNode.inFlow += value;
  }

  if (links.length === 0) {
    throw new DiagramInputError("A sankey diagram needs flow links");
  }

  // Kahn topological sort to validate DAG and compute longest-path column layers.
  const budget = new StepBudget();
  const inDegree = new Array<number>(nodes.length).fill(0);
  const outgoing: number[][] = Array.from({ length: nodes.length }, () => []);
  for (const link of links) {
    inDegree[link.target] = (inDegree[link.target] ?? 0) + 1;
    outgoing[link.source]?.push(link.target);
  }
  const queue: number[] = [];
  for (let i = 0; i < nodes.length; i++) {
    if ((inDegree[i] ?? 0) === 0) queue.push(i);
  }
  let visited = 0;
  let head = 0;
  while (head < queue.length) {
    budget.tick();
    const u = queue[head++] as number;
    visited++;
    const uLayer = nodes[u]?.layer ?? 0;
    for (const v of outgoing[u] ?? []) {
      const vNode = nodes[v];
      if (vNode && uLayer + 1 > vNode.layer) vNode.layer = uLayer + 1;
      const nextDeg = (inDegree[v] ?? 1) - 1;
      inDegree[v] = nextDeg;
      if (nextDeg === 0) queue.push(v);
    }
  }
  if (visited < nodes.length) {
    throw new DiagramInputError("Sankey diagram cannot contain cycles");
  }

  let maxLayer = 0;
  for (const node of nodes) {
    node.value = Math.max(node.inFlow, node.outFlow);
    if (node.layer > maxLayer) maxLayer = node.layer;
  }
  // Justify alignment: sink nodes with no outgoing flow sit in the rightmost column.
  for (const node of nodes) {
    if (node.outFlow === 0) node.layer = maxLayer;
  }

  return { nodes, links, maxLayer };
}
