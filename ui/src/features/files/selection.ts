/**
 * Browser selections inside numbered file text. Depends on the DOM's Range API; called by file
 * pointing. Gutters are removed from a cloned fragment, leaving the user's selection untouched.
 */
import type { CodeSelection } from "./model/context";

function selectedLine(node: Node): HTMLElement | null {
  return (
    (node instanceof Element ? node : node.parentElement)?.closest<HTMLElement>("[data-line]") ??
    null
  );
}

/** Reads only a selection whose two endpoints belong to this File tab's numbered text view. */
export function readFileSelection(root: HTMLElement): CodeSelection | null {
  const selection = window.getSelection();
  if (selection === null || selection.isCollapsed || selection.rangeCount === 0) return null;
  const range = selection.getRangeAt(0);
  const first = selectedLine(range.startContainer);
  const last = selectedLine(range.endContainer);
  const box = first?.closest(".file-text");
  if (first === null || last === null || box == null || !root.contains(box) || !box.contains(last))
    return null;
  const fragment = range.cloneContents();
  fragment.querySelectorAll(".file-ln").forEach((gutter) => {
    gutter.remove();
  });
  const text = fragment.textContent;
  if (text === "") return null;
  let end = Number(last.dataset["line"]);
  // A browser range ending at the next line's start has not selected that line.
  const finalLine = fragment.querySelector(`[data-line="${String(end)}"]`);
  if (end > Number(first.dataset["line"]) && finalLine?.textContent === "") end--;
  return { start: Number(first.dataset["line"]), end, text };
}
