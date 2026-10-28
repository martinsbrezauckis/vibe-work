/**
 * A short unified-style diff of a quick edit and how many lines it changed (a save reports the count).
 * Depends on: state/lineDiff. Called by: saveEdit.
 * Invariants: pure; at most `MAX_LINES` changed lines with two lines of context around each run.
 */
import { lineDiff } from "../../../state/lineDiff";

/** Most diff lines in the note. */
const MAX_LINES = 60;
/** Unchanged lines kept around a change. */
const CONTEXT = 2;

/**
 * The diff lines (`+`, `-`, ` `) of `before` -> `after`, cut to the changes and their context, and how many lines
 * changed (a replaced line counts once).
 */
export function editDiff(before: string, after: string): { lines: string[]; changed: number } {
  const diff = lineDiff(before, after);
  const keep = diff.map((_line, i) =>
    diff.slice(Math.max(0, i - CONTEXT), i + CONTEXT + 1).some((near) => near.kind !== "same"),
  );
  const lines: string[] = [];
  let added = 0;
  let removed = 0;
  let gap = false;
  diff.forEach((line, i) => {
    if (!keep[i]) {
      gap = lines.length > 0;
      return;
    }
    if (gap) lines.push("…");
    gap = false;
    if (line.kind === "add") added += 1;
    else if (line.kind === "del") removed += 1;
    lines.push(`${line.kind === "add" ? "+" : line.kind === "del" ? "-" : " "}${line.text}`);
  });
  // A replaced line is one removed and one added: count it once.
  const changed = Math.max(added, removed);
  if (lines.length > MAX_LINES) {
    const left = lines.length - MAX_LINES;
    return { lines: [...lines.slice(0, MAX_LINES), `… ${String(left)} more lines`], changed };
  }
  return { lines, changed };
}
