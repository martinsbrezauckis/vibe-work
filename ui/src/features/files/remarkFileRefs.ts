/**
 * A remark plugin that marks file references in parsed Markdown: a path in a text node, an inline code
 * span that is one whole path, or a link whose target is a relative / absolute / `file://` path becomes a
 * `vw-file-ref` element around the ORIGINAL nodes (the text is never
 * rewritten). The element renders as plain text until the core says the path names a
 * file the app may show (`FileRef.tsx`).
 *
 * Depends on: model/refs. Called by: components/markdown/Markdown (when file references are on), tests.
 * Invariants:
 * - Fenced code, HTML nodes (dropped by `skipHtml` anyway) and the text inside links are left alone.
 * - At most `MAX_REFS` references per message (a log pasted as prose cannot fan out into hundreds of lookups).
 * - The element carries only `dataPath` / `dataLine` properties; nothing here can navigate or reach the core.
 */
import { findPathRefs, pathOfCode, pathOfHref, repoFolderOfHref } from "./model/refs";

/** Most references marked in one Markdown source. */
const MAX_REFS = 60;

/** The parts of an mdast node this plugin reads or writes. */
interface MdNode {
  type: string;
  value?: string;
  url?: string;
  children?: MdNode[];
  data?: { hName?: string; hProperties?: Record<string, unknown> };
  position?: unknown;
}

/** The custom element name the Markdown renderer maps to `FileRef`. */
export const FILE_REF_TAG = "vw-file-ref";

function refNode(path: string, line: number | null, children: MdNode[], folder = false): MdNode {
  return {
    type: "fileRef",
    data: {
      hName: FILE_REF_TAG,
      hProperties: {
        dataPath: path,
        ...(line === null ? {} : { dataLine: String(line) }),
        ...(folder ? { dataFolder: "true" } : {}),
      },
    },
    children,
  };
}

/** The plugin's state for one tree. */
interface Budget {
  left: number;
}

function splitText(node: MdNode, budget: Budget): MdNode[] {
  const value = node.value ?? "";
  if (budget.left <= 0) return [node];
  const refs = findPathRefs(value, budget.left);
  if (refs.length === 0) return [node];
  const out: MdNode[] = [];
  let at = 0;
  for (const ref of refs) {
    if (ref.start > at) out.push({ type: "text", value: value.slice(at, ref.start) });
    out.push(
      refNode(ref.path, ref.line, [{ type: "text", value: value.slice(ref.start, ref.end) }]),
    );
    at = ref.end;
    budget.left -= 1;
  }
  if (at < value.length) out.push({ type: "text", value: value.slice(at) });
  return out;
}

function transform(node: MdNode, budget: Budget): void {
  const children = node.children;
  if (children === undefined) return;
  const out: MdNode[] = [];
  for (const child of children) {
    if (child.type === "text") {
      out.push(...splitText(child, budget));
    } else if (child.type === "inlineCode") {
      const folder = budget.left > 0 ? repoFolderOfHref(child.value ?? "") : null;
      const ref = folder === null && budget.left > 0 ? pathOfCode(child.value ?? "") : null;
      if (folder !== null) {
        budget.left -= 1;
        out.push(refNode(folder, null, [child], true));
      } else if (ref === null) out.push(child);
      else {
        budget.left -= 1;
        out.push(refNode(ref.path, ref.line, [child]));
      }
    } else if (child.type === "link") {
      const folder = budget.left > 0 ? repoFolderOfHref(child.url ?? "") : null;
      const ref = folder === null && budget.left > 0 ? pathOfHref(child.url ?? "") : null;
      if (folder !== null) {
        budget.left -= 1;
        out.push(refNode(folder, null, child.children ?? [], true));
      } else if (ref === null) out.push(child);
      else {
        // The link's own text stays as written; the reference replaces the link (no navigation at all).
        budget.left -= 1;
        out.push(refNode(ref.path, ref.line, child.children ?? []));
      }
    } else {
      if (child.type !== "code" && child.type !== "html") transform(child, budget);
      out.push(child);
    }
  }
  node.children = out;
}

/** The remark plugin (`remarkPlugins={[remarkFileRefs]}`). */
export function remarkFileRefs(): (tree: MdNode) => void {
  return (tree) => {
    transform(tree, { left: MAX_REFS });
  };
}
