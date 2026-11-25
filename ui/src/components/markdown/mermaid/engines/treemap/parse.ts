/**
 * Treemap diagram parser. Grammar follows Mermaid's treemap page: indentation hierarchy of quoted
 * `"Section"` parents and `"Leaf": value` numeric leaves. CSS class styling (`:::`), negative or
 * non-finite values, zero-total trees, empty sections, and leaves with children are refused.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_NODES, StepBudget, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** One node in a treemap hierarchy (section container or valued leaf). */
export interface TreemapNode {
  label: string;
  value: number;
  isLeaf: boolean;
  depth: number;
  /** Top-level root index (0..k-1) for branch palette coloring. */
  branch: number;
  children: TreemapNode[];
}

/** A parsed treemap diagram with root sections/leaves and positive total value. */
export interface TreemapModel {
  roots: TreemapNode[];
  total: number;
}

const REFUSED = /^(?:class|classDef|cssClass|click|link|callback)\b/i;
const NUMBER = /^\d{1,12}(?:\.\d{1,12})?$/;

function leadingIndent(raw: string): number {
  let indent = 0;
  for (const ch of raw) {
    if (ch === " ") indent += 1;
    else if (ch === "\t") indent += 4;
    else break;
  }
  return indent;
}

function parseQuotedPrefix(line: string): { label: string; rest: string } {
  if (!line.startsWith('"')) {
    throw new DiagramInputError(
      `Treemap nodes must start with a quoted label: ${line.slice(0, 40)}`,
    );
  }
  let label = "";
  let i = 1;
  while (i < line.length) {
    const ch = line[i] ?? "";
    if (ch === "\\" && i + 1 < line.length) {
      label += line[i + 1] ?? "";
      i += 2;
      continue;
    }
    if (ch === '"') {
      return { label, rest: line.slice(i + 1).trim() };
    }
    label += ch;
    i += 1;
  }
  throw new DiagramInputError("Unterminated quoted label in treemap");
}

function aggregate(node: TreemapNode, budget: StepBudget): number {
  budget.tick();
  if (node.isLeaf) return node.value;
  if (node.children.length === 0) {
    throw new DiagramInputError(`Treemap section "${node.label}" has no leaf values`);
  }
  let sum = 0;
  for (const child of node.children) {
    sum += aggregate(child, budget);
  }
  node.value = sum;
  return sum;
}

/**
 * Parse `treemap-beta` / `treemap` source (text or prepared lines); throws DiagramInputError on
 * invalid syntax, negative/zero-sum values, or `:::` styling, or DiagramLimitError on bounds.
 */
export function parseTreemap(input: string | PreparedSource): TreemapModel {
  const [, ...body] = linesOf(input, "treemap");
  if (body.length === 0) {
    throw new DiagramInputError("A treemap diagram needs sections or leaf values");
  }

  const budget = new StepBudget();
  const roots: TreemapNode[] = [];
  const stack: { node: TreemapNode; indent: number }[] = [];
  let count = 0;

  for (const raw of body) {
    budget.tick();
    const trimmed = raw.trim();
    if (!trimmed) continue;
    if (REFUSED.test(trimmed)) {
      throw new DiagramInputError(`Not drawn: ${trimmed.slice(0, 40)}`);
    }

    const { label, rest } = parseQuotedPrefix(trimmed);
    if (!label.trim()) throw new DiagramInputError("Treemap node label cannot be empty");
    if (rest.includes(":::")) {
      throw new DiagramInputError("CSS class annotations (:::) are not supported in treemap");
    }

    let isLeaf = false;
    let value = 0;
    if (rest.length > 0) {
      if (!rest.startsWith(":")) {
        throw new DiagramInputError(`Unexpected treemap line syntax: ${rest.slice(0, 30)}`);
      }
      const numText = rest.slice(1).trim();
      if (!NUMBER.test(numText)) {
        throw new DiagramInputError(`Invalid treemap value: ${numText.slice(0, 30)}`);
      }
      value = Number(numText);
      if (!Number.isFinite(value) || value < 0) {
        throw new DiagramInputError(`Treemap value must be finite and non-negative: ${numText}`);
      }
      isLeaf = true;
    }

    const indent = leadingIndent(raw);
    while (stack.length > 0 && (stack[stack.length - 1]?.indent ?? 0) >= indent) {
      stack.pop();
    }
    const depth = stack.length + 1;
    if (depth > MAX_DEPTH) {
      throw new DiagramLimitError(
        `Diagram too large: treemap nested more than ${String(MAX_DEPTH)} deep`,
      );
    }
    count += 1;
    checkCount(count, MAX_NODES, "nodes");

    const parent = stack[stack.length - 1]?.node;
    if (parent?.isLeaf) {
      throw new DiagramInputError(`Leaf "${parent.label}" cannot contain child nodes`);
    }
    const branch = parent ? parent.branch : roots.length;
    const node: TreemapNode = {
      label,
      value,
      isLeaf,
      depth,
      branch,
      children: [],
    };
    if (parent) parent.children.push(node);
    else roots.push(node);
    stack.push({ node, indent });
  }

  let total = 0;
  for (const root of roots) {
    total += aggregate(root, budget);
  }
  if (roots.length === 0 || !(total > 0)) {
    throw new DiagramInputError("A treemap diagram needs a positive total value");
  }
  return { roots, total };
}
