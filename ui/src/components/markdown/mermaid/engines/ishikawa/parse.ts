/**
 * Ishikawa (fishbone) diagram parser. Grammar follows Mermaid's ishikawa page: first body line is
 * the problem/effect at the head; subsequent indented lines form nested cause categories branching
 * off the spine. CSS class attachment (`:::`) and directives are refused.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** One cause node in an Ishikawa fishbone tree. */
export interface IshikawaCause {
  label: string;
  children: IshikawaCause[];
}

/** A parsed Ishikawa diagram: the head effect and top-level cause categories. */
export interface IshikawaModel {
  effect: string;
  categories: IshikawaCause[];
}

const REFUSED = /^(?:class|classDef|cssClass|click|link|callback)\b|:::/i;

function leadingIndent(raw: string): number {
  let indent = 0;
  for (const ch of raw) {
    if (ch === " ") indent += 1;
    else if (ch === "\t") indent += 4;
    else break;
  }
  return indent;
}

/**
 * Parse `ishikawa-beta` source (text or prepared lines); throws DiagramInputError on empty/malformed
 * input or DiagramLimitError when node or depth limits are exceeded.
 */
export function parseIshikawa(input: string | PreparedSource): IshikawaModel {
  const [, ...body] = linesOf(input, "ishikawa");
  const firstRaw = body[0];
  if (firstRaw === undefined) {
    throw new DiagramInputError("An ishikawa diagram needs an effect and at least one cause");
  }
  const effect = firstRaw.trim();
  if (!effect || REFUSED.test(effect)) {
    throw new DiagramInputError(`Unreadable ishikawa effect: ${effect.slice(0, 40)}`);
  }

  let nodeCount = 1;
  checkCount(nodeCount, MAX_NODES, "nodes");

  const categories: IshikawaCause[] = [];
  const stack: { node: IshikawaCause; indent: number }[] = [];

  for (const raw of body.slice(1)) {
    const label = raw.trim();
    if (!label) continue;
    if (REFUSED.test(label)) {
      throw new DiagramInputError(`Not drawn: ${label.slice(0, 40)}`);
    }
    const indent = leadingIndent(raw);
    while (stack.length > 0 && (stack[stack.length - 1]?.indent ?? 0) >= indent) {
      stack.pop();
    }
    const depth = stack.length + 1;
    if (depth > MAX_DEPTH) {
      throw new DiagramLimitError(
        `Diagram too large: causes nested more than ${String(MAX_DEPTH)} deep`,
      );
    }
    nodeCount += 1;
    checkCount(nodeCount, MAX_NODES, "nodes");
    const cause: IshikawaCause = { label, children: [] };
    const parent = stack[stack.length - 1];
    if (parent) parent.node.children.push(cause);
    else categories.push(cause);
    stack.push({ node: cause, indent });
  }

  if (categories.length === 0) {
    throw new DiagramInputError("An ishikawa diagram needs at least one cause");
  }
  return { effect, categories };
}
