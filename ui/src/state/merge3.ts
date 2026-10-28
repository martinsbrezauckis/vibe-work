/**
 * A three-way line merge for an open editor: the text the editor started from (`base`), the user's unsaved text
 * (`mine`) and the newer text on disk (`theirs`, usually Vibe's edit). Changes to different lines are combined;
 * changes to the same lines are conflicts, resolved only when a side is preferred.
 * Depends on: lineDiff. Called by: fileEdits (disk reconciliation and the Keep yours / Take Vibe's choices).
 * Invariants: pure; without a preference a conflict leaves `mine` unchanged (`text === mine`), so nothing the user
 * typed is ever replaced without their choice; line numbers are 1-based.
 */
import { joinLines, lineDiff, splitLines } from "./lineDiff";

/** One side's change: base lines `[start, end)` replaced by `lines`. */
interface Hunk {
  start: number;
  end: number;
  lines: string[];
  side: "mine" | "theirs";
}

/** What a merge gives. */
export interface Merge {
  /** The merged text (`mine` itself while there are unresolved conflicts). */
  text: string;
  /** Lines of `mine` that both sides changed differently (empty when the merge is clean). */
  conflicts: number[];
  /** Lines of `text` that came from `theirs` (where a deletion happened: the line after it). */
  theirs: number[];
}

function hunksOf(base: string, other: string, side: Hunk["side"]): Hunk[] {
  const out: Hunk[] = [];
  let at = 0;
  let current: Hunk | null = null;
  for (const line of lineDiff(base, other)) {
    if (line.kind === "same") {
      if (current !== null) out.push(current);
      current = null;
      at += 1;
      continue;
    }
    current ??= { start: at, end: at, lines: [], side };
    if (line.kind === "del") {
      at += 1;
      current.end = at;
    } else current.lines.push(line.text);
  }
  if (current !== null) out.push(current);
  return out;
}

/** `base[start, end)` with the hunks of one side applied. */
function region(base: readonly string[], start: number, end: number, hunks: readonly Hunk[]) {
  const out: string[] = [];
  let at = start;
  for (const hunk of hunks) {
    out.push(...base.slice(at, hunk.start), ...hunk.lines);
    at = hunk.end;
  }
  out.push(...base.slice(at, end));
  return out;
}

/** Real lines only: the no-newline marker is not a line, and a deletion at the end marks the last line. */
function clamp(marks: readonly number[], text: string): number[] {
  const count = text === "" ? 0 : text.replace(/\n$/, "").split("\n").length;
  const kept = [...new Set(marks.map((m) => Math.min(m, count)))].filter((m) => m >= 1);
  return kept.sort((a, b) => a - b);
}

/** Merges `mine` and `theirs`, both edited from `base`; `prefer` settles the conflicts with that side. */
export function merge3(
  base: string,
  mine: string,
  theirs: string,
  prefer?: "mine" | "theirs",
): Merge {
  const lines = splitLines(base);
  const all = [...hunksOf(base, mine, "mine"), ...hunksOf(base, theirs, "theirs")].sort(
    (a, b) => a.start - b.start || a.end - b.end,
  );
  const out: string[] = [];
  const fromTheirs: number[] = [];
  const conflicts: number[] = [];
  let at = 0;
  let mineAt = 0;
  for (let i = 0; i < all.length;) {
    const first = all[i];
    if (first === undefined) break;
    const cluster = [first];
    let end = first.end;
    i += 1;
    // A hunk joins when it overlaps, or starts at the same place (two insertions there have no order).
    for (
      let next = all[i];
      next !== undefined && (next.start < end || next.start === first.start);
      next = all[i]
    ) {
      cluster.push(next);
      end = Math.max(end, next.end);
      i += 1;
    }
    const start = first.start;
    out.push(...lines.slice(at, start));
    mineAt += start - at;
    const ours = region(
      lines,
      start,
      end,
      cluster.filter((h) => h.side === "mine"),
    );
    const other = region(
      lines,
      start,
      end,
      cluster.filter((h) => h.side === "theirs"),
    );
    const sides = new Set(cluster.map((h) => h.side));
    const same = ours.length === other.length && ours.every((l, k) => l === other[k]);
    let chosen: "mine" | "theirs";
    if (sides.size === 1) chosen = first.side;
    else if (same) chosen = "theirs";
    else if (prefer !== undefined) chosen = prefer;
    else {
      conflicts.push(...(ours.length === 0 ? [mineAt + 1] : ours.map((_, k) => mineAt + k + 1)));
      chosen = "mine";
    }
    const taken = chosen === "mine" ? ours : other;
    if (chosen === "theirs")
      fromTheirs.push(
        ...(taken.length === 0 ? [out.length + 1] : taken.map((_, k) => out.length + k + 1)),
      );
    out.push(...taken);
    mineAt += ours.length;
    at = end;
  }
  out.push(...lines.slice(at));
  if (conflicts.length > 0) return { text: mine, conflicts: clamp(conflicts, mine), theirs: [] };
  const text = joinLines(out);
  return { text, conflicts: [], theirs: clamp(fromTheirs, text) };
}
