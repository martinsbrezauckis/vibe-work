/**
 * What the agent is told about a quick edit: a short unified-style diff of the user's change, sent
 * as a context chip with the next message (the composer's chips are data, never typed text, never a command).
 * Depends on: state/lineDiff. Called by: FileEditor (after a save).
 * Invariants: pure; at most `MAX_LINES` changed lines with two lines of context around each run; the chip says the
 * user made the change, so the agent does not take it for its own.
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

/** The chip's text for an edit of `display` (a path the user sees). */
export function editNoteText(display: string, before: string, after: string): string {
  const { lines } = editDiff(before, after);
  return `I edited ${display} myself in Vibe Work's file viewer (it is saved on disk). My change:\n\`\`\`diff\n${lines.join("\n")}\n\`\`\``;
}
