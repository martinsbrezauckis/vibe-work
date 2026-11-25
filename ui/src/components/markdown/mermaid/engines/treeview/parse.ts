/**
 * TreeView diagram parser. Supports both indentation-based outlines and box-drawing trees
 * (`├──`, `└──`, `│`, `┣━━`, `┗━━`, `┃`), bare and quoted labels, trailing `/` directories,
 * `icon(...)`, `:::highlight`, and `##` inline descriptions. Arbitrary `:::class` styling is refused.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_NODES, StepBudget, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** One node in a TreeView directory hierarchy. */
export interface TreeViewNode {
  label: string;
  isDir: boolean;
  highlight: boolean;
  /** Undefined = default (hidden), "none" = explicitly hidden, otherwise icon name (`folder`, `file`, `pack:name`). */
  icon?: string | undefined;
  description?: string | undefined;
  depth: number;
  children: TreeViewNode[];
}

/** A parsed TreeView diagram consisting of one or more top-level root nodes. */
export interface TreeViewModel {
  roots: TreeViewNode[];
}

const BOX_LINE = /^([ │┃\t]*)(├──|└──|┣━━|┗━━|├─|└─|┣━|┗━)\s*(.*)$/;
const HAS_BOX_CHAR = /[├└┣┗│┃]/;

function expandTabs(raw: string): string {
  return raw.replace(/\t/g, "    ");
}

function leadingSpaces(raw: string): number {
  let indent = 0;
  for (const ch of raw) {
    if (ch === " ") indent += 1;
    else if (ch === "\t") indent += 4;
    else break;
  }
  return indent;
}

function parseQuotedLabel(content: string): { label: string; rest: string } {
  let label = "";
  let i = 1;
  while (i < content.length) {
    const ch = content[i] ?? "";
    if (ch === "\\" && i + 1 < content.length) {
      label += content[i + 1] ?? "";
      i += 2;
      continue;
    }
    if (ch === '"') {
      let rest = content.slice(i + 1);
      if (rest.startsWith("/")) {
        label += "/";
        rest = rest.slice(1);
      }
      return { label, rest };
    }
    label += ch;
    i += 1;
  }
  throw new DiagramInputError("Unterminated quoted label in treeView");
}

function parseAnnotations(
  initialRest: string,
  budget: StepBudget,
): { highlight: boolean; icon?: string | undefined; description?: string | undefined } {
  let rest = initialRest;
  let highlight = false;
  let icon: string | undefined;
  let description: string | undefined;

  while (rest.trim().length > 0) {
    budget.tick();
    rest = rest.trimStart();
    if (rest.startsWith("##")) {
      description = rest.slice(2).trim();
      break;
    }
    if (rest.startsWith(":::")) {
      const match = /^:::(\S+)(.*)$/.exec(rest);
      const cls = match?.[1] ?? "";
      if (!cls) throw new DiagramInputError("Missing class name after ::: in treeView");
      if (cls !== "highlight") {
        throw new DiagramInputError(
          `Unsupported treeView class ":::${cls}": only :::highlight is supported`,
        );
      }
      highlight = true;
      rest = match?.[2] ?? "";
      continue;
    }
    if (rest.startsWith("icon(")) {
      const closeIdx = rest.indexOf(")", 5);
      if (closeIdx < 0) throw new DiagramInputError("Unclosed icon(...) annotation in treeView");
      const inner = rest.slice(5, closeIdx).trim();
      icon = inner === "" || inner.toLowerCase() === "none" ? "none" : inner;
      rest = rest.slice(closeIdx + 1);
      continue;
    }
    throw new DiagramInputError(`Unexpected annotation in treeView: ${rest.slice(0, 30)}`);
  }

  return { highlight, icon, description };
}

function parseNodeContent(content: string, depth: number, budget: StepBudget): TreeViewNode {
  const trimmed = content.trimStart();
  if (!trimmed) throw new DiagramInputError("Empty node label in treeView");

  let label: string;
  let rest: string;
  if (trimmed.startsWith('"')) {
    ({ label, rest } = parseQuotedLabel(trimmed));
  } else {
    const marker = /(?:^|\s)(?:icon\(|##)|:::/.exec(trimmed);
    const splitAt = marker ? marker.index : trimmed.length;
    label = trimmed.slice(0, splitAt).trim();
    rest = trimmed.slice(splitAt);
  }
  if (!label) throw new DiagramInputError("Empty node label in treeView");
  const { highlight, icon, description } = parseAnnotations(rest, budget);
  return {
    label,
    isDir: label.endsWith("/"),
    highlight,
    icon,
    description,
    depth,
    children: [],
  };
}

/**
 * Parse `treeView-beta` source (text or prepared lines); throws DiagramInputError on syntax errors
 * or DiagramLimitError when node or depth limits are exceeded.
 */
export function parseTreeview(input: string | PreparedSource): TreeViewModel {
  const [, ...body] = linesOf(input, "treeview");
  if (body.length === 0) {
    throw new DiagramInputError("A treeView diagram needs at least one node");
  }

  const budget = new StepBudget();
  const isBoxMode = body.some((line) => HAS_BOX_CHAR.test(line));
  const roots: TreeViewNode[] = [];
  const stack: { node: TreeViewNode; col: number }[] = [];
  let count = 0;

  body.forEach((raw, index) => {
    budget.tick();
    const boxMatch = BOX_LINE.exec(raw);
    let col: number;
    let content: string;
    if (boxMatch) {
      col = Array.from(expandTabs(boxMatch[1] ?? "")).length;
      content = boxMatch[3] ?? "";
    } else if (isBoxMode) {
      if (HAS_BOX_CHAR.test(raw) || index > 0) {
        throw new DiagramInputError(
          `Unreadable box-drawing treeView line: ${raw.trim().slice(0, 40)}`,
        );
      }
      col = -1;
      content = raw.trim();
    } else {
      col = leadingSpaces(raw);
      content = raw.trim();
    }

    while (stack.length > 0 && (stack[stack.length - 1]?.col ?? 0) >= col) {
      stack.pop();
    }
    const depth = stack.length;
    if (depth + 1 > MAX_DEPTH) {
      throw new DiagramLimitError(
        `Diagram too large: treeView nested more than ${String(MAX_DEPTH)} deep`,
      );
    }
    count += 1;
    checkCount(count, MAX_NODES, "nodes");

    const node = parseNodeContent(content, depth, budget);
    const parent = stack[stack.length - 1];
    if (parent) parent.node.children.push(node);
    else roots.push(node);
    stack.push({ node, col });
  });

  if (roots.length === 0) {
    throw new DiagramInputError("A treeView diagram needs at least one node");
  }
  return { roots };
}
