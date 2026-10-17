/**
 * The Changes tab's pure model: labels, what a revert would delete, the side-by-side pairing of
 * diff lines, and the mapping from the core's hunk lines to the `DiffLine` (so hunks render with the
 * transcript's diff rows, never a fork of them).
 * Depends on: protocol/core types, transcript/model/diff (`DiffLine`). Called by: the store, the views,
 * the `ui/changes` route.
 * Invariants: pure functions over the core's view; nothing here decides what the core accepts (the core
 * re-checks every target).
 */
import type {
  ChangesView,
  GitStatusView,
  DiffLine as CoreLine,
  FileChange,
  Hunk,
  TurnLabel,
} from "../../protocol/core";
import type { DiffLine } from "../transcript/model/diff";

/** True when the session has something for the Changes tab to show (else the tab is not offered). */
export function hasChanges(view: ChangesView | undefined): boolean {
  return view !== undefined && view.mode !== "none";
}

/** The history offer waits for Git discovery and disappears for any found repository. */
export function historyOfferShown(
  view: ChangesView | undefined,
  status: GitStatusView | undefined,
  selected: string | undefined,
): boolean {
  return (
    view?.offerHistory === true &&
    status !== undefined &&
    selected === undefined &&
    status.repos.length === 0
  );
}

/** the line shape for one core line. */
export function toDiffLine(line: CoreLine): DiffLine {
  return { kind: line.kind, oldNo: line.oldNo, newNo: line.newNo, text: line.text };
}

/** `+a −r` (or the size for a large file). */
export function statText(file: FileChange): string {
  if (file.content === "largeUntracked" && file.size !== null)
    return `${(file.size / (1024 * 1024)).toFixed(1)} MB`;
  if (file.content === "binary" || file.content === "submodule") return "";
  return `+${String(file.added)} −${String(file.removed)}`;
}

/** The short state line under a file in the list. */
export function fileNote(file: FileChange): string {
  switch (file.content) {
    case "binary":
      return file.status === "added" ? "new binary file" : "binary file";
    case "tooLarge":
      return file.hunkCount > 0
        ? `too large to show here: ${String(file.added + file.removed)} changed lines in ${String(file.hunkCount)} ${file.hunkCount === 1 ? "hunk" : "hunks"}`
        : "too large to compare line by line";
    case "submodule":
      return "submodule: review it in its own repository";
    case "largeUntracked":
      return "large new file, not reviewed line by line";
    case "text":
      break;
  }
  if (file.status === "added") return "new file";
  if (file.status === "deleted") return "deleted";
  if (file.status === "renamed") return `moved from ${file.oldPath ?? "?"}`;
  if (file.newMode !== null && file.hunks.length === 0) return `mode ${file.newMode}`;
  const n = file.hunks.length;
  return `${String(n)} ${n === 1 ? "hunk" : "hunks"}`;
}

/** "turn 3", or "during turn 3" when the file changed while the turn ran (e.g. through a command). */
export function turnText(label: TurnLabel | null): string | null {
  if (label === null) return null;
  return label.source === "effect"
    ? `turn ${String(label.number)}`
    : `during turn ${String(label.number)}`;
}

/** Per-hunk actions apply to text files that exist on both sides. */
export function hunkActions(file: FileChange): boolean {
  return file.content === "text" && (file.status === "modified" || file.status === "renamed");
}

/**
 * True when the user saw the whole change line by line. "Keep all" accepts only these (the core enforces
 * it); keeping any other file one by one asks first.
 */
export function shownInFull(file: FileChange): boolean {
  return file.content === "text";
}

/** File-level actions exist for everything but submodules. */
export function fileActions(file: FileChange): boolean {
  return file.content !== "submodule";
}

/** The path a revert of `file` deletes (a file created in the session, the new path of a move). A
 * case-only rename on a case-insensitive file system (`caseOnly`) is one file under two names: it is
 * renamed back and nothing is deleted. */
export function deletedByRevert(file: FileChange): string | null {
  if (file.status === "added") return file.path;
  return file.status === "renamed" && !file.caseOnly ? file.path : null;
}

/** Every path "Revert all" deletes. */
export function bulkDeletes(view: ChangesView): string[] {
  return view.files
    .filter((f) => f.content !== "submodule")
    .map(deletedByRevert)
    .filter((p): p is string => p !== null);
}

/** "lines 12–18" of a hunk (current side, or the baseline side for a pure removal). */
export function hunkRange(hunk: Hunk): string {
  const start = hunk.newLines > 0 ? hunk.newStart : hunk.oldStart;
  const count = hunk.newLines > 0 ? hunk.newLines : hunk.oldLines;
  return count <= 1
    ? `line ${String(start)}`
    : `lines ${String(start)}–${String(start + count - 1)}`;
}

/** One row of the side-by-side view: left (baseline) and right (current), `null` for an empty cell. */
export interface SplitRow {
  left: DiffLine | null;
  right: DiffLine | null;
}

/**
 * Pairs lines for the side-by-side view (matching context on both sides): context on both sides; a run of
 * removed lines pairs with the added lines that follow it, row by row.
 */
export function splitRows(lines: readonly DiffLine[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line === undefined) break;
    if (line.kind === "ctx" || line.kind === "gap") {
      rows.push({ left: line, right: line });
      i += 1;
      continue;
    }
    const dels: DiffLine[] = [];
    const adds: DiffLine[] = [];
    while (lines[i]?.kind === "del") dels.push(lines[i++] as DiffLine);
    while (lines[i]?.kind === "add") adds.push(lines[i++] as DiffLine);
    for (let j = 0; j < Math.max(dels.length, adds.length); j += 1) {
      rows.push({ left: dels[j] ?? null, right: adds[j] ?? null });
    }
  }
  return rows;
}

/** The file to show: the selected one if it is still changed, else the first. */
export function shownFile(view: ChangesView, selected: string | null): FileChange | null {
  return view.files.find((f) => f.path === selected) ?? view.files[0] ?? null;
}

/** Empty review wording after a confirmed local action. */
export function completionText(completion: "history" | "commit" | undefined): string | null {
  return completion === "history"
    ? "Version history is on. The first version includes every file, also the changes Vibe just made."
    : completion === "commit"
      ? "Everything is saved. New changes appear here."
      : null;
}
