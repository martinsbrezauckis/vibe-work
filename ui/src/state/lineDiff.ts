/**
 * A line diff for the "review your changes" step of the instruction editors: what the file holds
 * now against the text about to be saved.
 * Depends on: nothing. Called by: instructions (counts), features/settings/DiffPreview.
 * Invariants: pure; the result, read top to bottom with `del` lines taken from `before` and `add` lines
 * from `after`, rebuilds both texts exactly (tests; `joinLines` is the inverse of `splitLines`). Long inputs never blow up: past a work limit the
 * middle part is shown as "all removed, then all added" (still correct, just less fine).
 */

/** Whether a line is in both texts, only in the old one, or only in the new one. */
export type DiffKind = "same" | "add" | "del";

/** One line of the diff (without its newline). */
export interface DiffLine {
  kind: DiffKind;
  text: string;
}

/** Most table cells the exact comparison may fill (about 16 MB of 32-bit counters). */
const WORK_LIMIT = 4_000_000;

/** The marker line (as in `git diff`) for a text whose last line has no newline. */
export const NO_NEWLINE = "\\ No newline at end of file";

/**
 * The lines of `text`; the empty text has none. A final newline ends the last line instead of starting
 * an empty one, and a last line without one gets the `NO_NEWLINE` marker, so a change of only the final
 * newline is a visible change.
 */
export function splitLines(text: string): string[] {
  if (text === "") return [];
  const parts = text.split("\n");
  if (parts[parts.length - 1] === "") return parts.slice(0, -1);
  return [...parts, NO_NEWLINE];
}

/** The text that `splitLines` split into `lines`. */
export function joinLines(lines: readonly string[]): string {
  if (lines.length === 0) return "";
  if (lines[lines.length - 1] === NO_NEWLINE) return lines.slice(0, -1).join("\n");
  return `${lines.join("\n")}\n`;
}

/** Diffs two texts line by line. */
export function lineDiff(before: string, after: string): DiffLine[] {
  const a = splitLines(before);
  const b = splitLines(after);
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start += 1;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA -= 1;
    endB -= 1;
  }
  const out: DiffLine[] = a.slice(0, start).map((text) => ({ kind: "same", text }));
  out.push(...middle(a.slice(start, endA), b.slice(start, endB)));
  out.push(...a.slice(endA).map((text): DiffLine => ({ kind: "same", text })));
  return out;
}

/**
 * Counts of added and removed lines. The `NO_NEWLINE` markers are not lines of the file, so they are
 * not counted; `finalNewlineChanged` says when only the end of the file differs.
 */
export function diffCounts(lines: readonly DiffLine[]): {
  added: number;
  removed: number;
  finalNewlineChanged: boolean;
} {
  let added = 0;
  let removed = 0;
  let marker = false;
  for (const line of lines) {
    if (line.kind === "same") continue;
    if (line.text === NO_NEWLINE) marker = true;
    else if (line.kind === "add") added += 1;
    else removed += 1;
  }
  return { added, removed, finalNewlineChanged: marker };
}

function middle(a: string[], b: string[]): DiffLine[] {
  if (a.length === 0) return b.map((text) => ({ kind: "add", text }));
  if (b.length === 0) return a.map((text) => ({ kind: "del", text }));
  if (a.length * b.length > WORK_LIMIT) {
    return [
      ...a.map((text): DiffLine => ({ kind: "del", text })),
      ...b.map((text): DiffLine => ({ kind: "add", text })),
    ];
  }
  // Longest common subsequence table, filled from the end so the walk below runs forwards.
  const width = b.length + 1;
  const table = new Uint32Array((a.length + 1) * width);
  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) {
      table[i * width + j] =
        a[i] === b[j]
          ? (table[(i + 1) * width + j + 1] ?? 0) + 1
          : Math.max(table[(i + 1) * width + j] ?? 0, table[i * width + j + 1] ?? 0);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ kind: "same", text: a[i] ?? "" });
      i += 1;
      j += 1;
    } else if ((table[(i + 1) * width + j] ?? 0) >= (table[i * width + j + 1] ?? 0)) {
      out.push({ kind: "del", text: a[i] ?? "" });
      i += 1;
    } else {
      out.push({ kind: "add", text: b[j] ?? "" });
      j += 1;
    }
  }
  for (; i < a.length; i += 1) out.push({ kind: "del", text: a[i] ?? "" });
  for (; j < b.length; j += 1) out.push({ kind: "add", text: b[j] ?? "" });
  return out;
}
