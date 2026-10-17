/**
 * A remark plugin for chat text: a single newline inside a paragraph becomes a line break.
 * Standard Markdown treats that newline as a soft break and shows a space, so an answer that lists
 * `README.md`, `calc.py` and `src/app.py` on their own lines would read as one line. Chat apps show
 * the lines as written; this plugin does the same for chat messages only (`Markdown` with `lineBreaks`).
 *
 * Depends on: nothing (walks the mdast tree itself). Called by: components/markdown/Markdown.
 * Invariants:
 * - Only `text` nodes are split. Fenced and inline code (`code` / `inlineCode`) keep their text as it is,
 *   and raw HTML nodes are never touched (the renderer drops them anyway).
 * - Spaces or tabs before the newline are dropped with it (they would only trail the line).
 * - Block structure (paragraphs, lists, tables, headings) comes from the parser and is not changed; a
 *   newline that ends a block never reaches a text node.
 */

/** The parts of an mdast node this plugin reads or writes. */
interface MdNode {
  type: string;
  value?: string;
  children?: MdNode[];
}

/** A newline (any line ending) with the spaces or tabs before it. */
const NEWLINE = /[\t ]*(?:\r\n|\n|\r)/;

/** `node` split at each newline into text and `break` nodes (unchanged when it has none). */
function splitText(node: MdNode): MdNode[] {
  const value = node.value ?? "";
  if (!NEWLINE.test(value)) return [node];
  const out: MdNode[] = [];
  value.split(NEWLINE).forEach((part, i) => {
    if (i > 0) out.push({ type: "break" });
    if (part !== "") out.push({ type: "text", value: part });
  });
  return out;
}

function transform(node: MdNode): void {
  const children = node.children;
  if (children === undefined) return;
  const out: MdNode[] = [];
  for (const child of children) {
    if (child.type === "text") out.push(...splitText(child));
    else {
      transform(child);
      out.push(child);
    }
  }
  node.children = out;
}

/** The remark plugin (`remarkPlugins={[remarkLineBreaks]}`). */
export function remarkLineBreaks(): (tree: MdNode) => void {
  return (tree) => {
    transform(tree);
  };
}
