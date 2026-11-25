/**
 * Mindmap diagram parser. Grammar follows Mermaid's mindmap page: indentation hierarchy with a
 * single root, "unclear indentation" resolution (nearest ancestor with smaller indent), seven node
 * shapes, multi-line Markdown strings, `<br>` breaks, and `::icon(...)` metadata. CSS class
 * annotations (`:::`) and directives are refused.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_NODES, StepBudget, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** Supported mindmap node shapes documented by Mermaid. */
export type MindmapShape =
  "default" | "square" | "rounded" | "circle" | "bang" | "cloud" | "hexagon";

/** One node in a mindmap hierarchy. */
export interface MindmapNode {
  id: string;
  label: string;
  shape: MindmapShape;
  depth: number;
  /** Top-level branch index (0..k-1) for branch coloring; -1 for root. */
  branch: number;
  icon?: string | undefined;
  isMarkdown: boolean;
  children: MindmapNode[];
}

/** A parsed mindmap diagram with a single root node. */
export interface MindmapModel {
  root: MindmapNode;
}

const REFUSED = /^(?:class|classDef|cssClass|click|link|callback)\b/i;

function leadingIndent(raw: string): number {
  let indent = 0;
  for (const ch of raw) {
    if (ch === " ") indent += 1;
    else if (ch === "\t") indent += 4;
    else break;
  }
  return indent;
}

function unwrapLabel(raw: string): { label: string; isMarkdown: boolean } {
  const trimmed = raw.trim();
  if (trimmed.startsWith('"`') && trimmed.endsWith('`"') && trimmed.length >= 4) {
    return { label: trimmed.slice(2, -2), isMarkdown: true };
  }
  if (trimmed.startsWith('"') && trimmed.endsWith('"') && trimmed.length >= 2) {
    return { label: trimmed.slice(1, -1), isMarkdown: false };
  }
  return { label: trimmed, isMarkdown: false };
}

function hasUnclosedMarkdown(text: string): boolean {
  const open = text.indexOf('"`');
  if (open < 0) return false;
  const close = text.indexOf('`"', open + 2);
  return close < 0;
}

function checkUnclosedShape(text: string): void {
  if (/^[^\s[\](){}]*\(\(/.test(text) && !text.includes("))")) {
    throw new DiagramInputError("Unclosed circle shape `((` in mindmap");
  }
  if (/^[^\s[\](){}]*\)\)/.test(text) && !text.includes("((")) {
    throw new DiagramInputError("Unclosed bang shape `))` in mindmap");
  }
  if (/^[^\s[\](){}]*\{\{/.test(text) && !text.includes("}}")) {
    throw new DiagramInputError("Unclosed hexagon shape `{{` in mindmap");
  }
  if (/^[^\s[\](){}]*\[/.test(text) && !text.includes("]")) {
    throw new DiagramInputError("Unclosed square shape `[` in mindmap");
  }
  if (/^[^\s[\](){}]+\([^)]*$/.test(text)) {
    throw new DiagramInputError("Unclosed rounded shape `(` in mindmap");
  }
}

const SHAPE_MATCHERS: [MindmapShape, RegExp][] = [
  ["circle", /^([^\s[\](){}]*)\(\(([\s\S]*)\)\)$/],
  ["bang", /^([^\s[\](){}]*)\)\)([\s\S]*)\(\($/],
  ["hexagon", /^([^\s[\](){}]*)\{\{([\s\S]*)\}\}$/],
  ["square", /^([^\s[\](){}]*)\[([\s\S]*)\]$/],
  ["rounded", /^([^\s[\](){}]*)\(([\s\S]*)\)$/],
  ["cloud", /^([^\s[\](){}]*)\)([\s\S]*)\($/],
];

function parseShape(
  rawText: string,
  fallbackId: string,
): { id: string; label: string; shape: MindmapShape; isMarkdown: boolean } {
  checkUnclosedShape(rawText);
  for (const [shape, pattern] of SHAPE_MATCHERS) {
    const match = pattern.exec(rawText);
    if (!match) continue;
    const id = match[1] ? match[1] : fallbackId;
    const { label, isMarkdown } = unwrapLabel(match[2] ?? "");
    if (!label.trim()) throw new DiagramInputError("Empty mindmap node label");
    return { id, label, shape, isMarkdown };
  }
  const { label, isMarkdown } = unwrapLabel(rawText);
  if (!label.trim()) throw new DiagramInputError("Empty mindmap node label");
  return { id: fallbackId, label, shape: "default", isMarkdown };
}

/**
 * Parse `mindmap` source (text or prepared lines); throws DiagramInputError on malformed lines,
 * multiple roots, or refused styling (`:::`), or DiagramLimitError when bounds are exceeded.
 */
export function parseMindmap(input: string | PreparedSource): MindmapModel {
  const [, ...body] = linesOf(input, "mindmap");
  if (body.length === 0) {
    throw new DiagramInputError("A mindmap diagram needs a root node");
  }

  const budget = new StepBudget();
  const joinedLines: { indent: number; text: string }[] = [];

  for (let i = 0; i < body.length; i++) {
    budget.tick();
    const raw = body[i] ?? "";
    const indent = leadingIndent(raw);
    let text = raw.trimStart();
    while (hasUnclosedMarkdown(text)) {
      i += 1;
      if (i >= body.length) {
        throw new DiagramInputError("Unterminated markdown string in mindmap");
      }
      budget.tick();
      text = `${text}\n${body[i] ?? ""}`;
    }
    joinedLines.push({ indent, text });
  }

  const stack: { node: MindmapNode; indent: number }[] = [];
  let root: MindmapNode | undefined;
  let lastNode: MindmapNode | undefined;
  let count = 0;

  for (const entry of joinedLines) {
    budget.tick();
    let line = entry.text.trim();
    if (!line) continue;

    if (line.startsWith("::icon(")) {
      const close = line.indexOf(")", 7);
      if (close < 0) throw new DiagramInputError("Unclosed ::icon(...) in mindmap");
      if (!lastNode) throw new DiagramInputError("::icon(...) must follow a mindmap node");
      lastNode.icon = line.slice(7, close).trim();
      const trailing = line.slice(close + 1).trim();
      if (trailing) throw new DiagramInputError(`Unexpected text after ::icon(): ${trailing}`);
      continue;
    }

    if (
      REFUSED.test(line) ||
      (!hasUnclosedMarkdown(line) && !line.startsWith('"`') && line.includes(":::"))
    ) {
      throw new DiagramInputError("CSS class annotations (:::) are not supported in mindmap");
    }

    let inlineIcon: string | undefined;
    const inlineIconMatch = /\s+::icon\(([^)]*)\)\s*$/.exec(line);
    if (inlineIconMatch) {
      inlineIcon = (inlineIconMatch[1] ?? "").trim();
      line = line.slice(0, inlineIconMatch.index).trim();
    }

    count += 1;
    checkCount(count, MAX_NODES, "nodes");

    if (!root) {
      const parsed = parseShape(line, `n${String(count)}`);
      root = {
        ...parsed,
        depth: 0,
        branch: -1,
        icon: inlineIcon,
        children: [],
      };
      stack.push({ node: root, indent: entry.indent });
      lastNode = root;
      continue;
    }

    const rootIndent = stack[0]?.indent ?? 0;
    if (entry.indent <= rootIndent) {
      throw new DiagramInputError("A mindmap can only have one root node");
    }
    while (stack.length > 1 && (stack[stack.length - 1]?.indent ?? 0) >= entry.indent) {
      stack.pop();
    }
    const depth = stack.length;
    if (depth > MAX_DEPTH) {
      throw new DiagramLimitError(
        `Diagram too large: mindmap nested more than ${String(MAX_DEPTH)} deep`,
      );
    }
    const parent = stack[stack.length - 1]?.node;
    if (!parent) throw new DiagramInputError("A mindmap can only have one root node");

    const branch = depth === 1 ? root.children.length : parent.branch;
    const parsed = parseShape(line, `n${String(count)}`);
    const node: MindmapNode = {
      ...parsed,
      depth,
      branch,
      icon: inlineIcon,
      children: [],
    };
    parent.children.push(node);
    stack.push({ node, indent: entry.indent });
    lastNode = node;
  }

  if (!root) {
    throw new DiagramInputError("A mindmap diagram needs a root node");
  }
  return { root };
}
