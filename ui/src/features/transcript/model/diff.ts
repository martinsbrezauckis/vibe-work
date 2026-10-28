/**
 * Inline diffs without git ("Facts"): a completed `file_edit` carries
 * `state.output.occurrences[] = {startLine, oldText, newText}` and a completed `file_write` carries
 * `state.output = {filePath, content, fileExisted, previousContent}` (confirmed in the protocol
 * captures). This module turns either into numbered diff lines.
 * Depends on: `diff` 9.0.0 (`diffLines`, BSD-3-Clause), entries.ts. Called by: the step body and the
 * approval card (a pending edit shows the change it asks for).
 * Invariants:
 * - Both sides are compared with a final newline added, so a missing newline at the end of
 *   `oldText` never turns the last line into a remove/add pair.
 * - A diff too large to compute quickly (edit distance over `MAX_EDIT`) degrades to "old block
 *   removed, new block added" instead of freezing the window.
 * - The path comes from the output when the effect completed, else from `detail.input` (a failed
 *   write has no `filePath` in its output).
 */
import { diffLines } from "diff";

import { type Obj, arr, inputOf, num, obj, outputOf, statusOf, str } from "./entries";

/** One diff line. Numbers are file line numbers (old side, new side), `null` where none applies. */
export interface DiffLine {
  kind: "ctx" | "add" | "del" | "gap";
  oldNo: number | null;
  newNo: number | null;
  text: string;
}

/** A diff for one file. */
export interface FileDiff {
  path: string;
  lines: DiffLine[];
  added: number;
  removed: number;
  /** The write created the file. */
  newFile: boolean;
}

/** First changed file line for navigation, with a first-line fallback for an empty diff. */
export function firstChangedLine(diff: FileDiff): number {
  const line = diff.lines.find((line) => line.kind === "add" || line.kind === "del");
  return Math.max(1, line?.newNo ?? line?.oldNo ?? 1);
}

/** Largest edit distance computed line by line before degrading (keeps a huge rewrite cheap). */
const MAX_EDIT = 4000;

function splitLines(block: string): string[] {
  const body = block.endsWith("\n") ? block.slice(0, -1) : block;
  return body === "" && block === "" ? [] : body.split("\n");
}

/**
 * Line diff of `oldText` -> `newText`, numbered from `oldStart` / `newStart` (1-based).
 * Returns the lines and the next line numbers after the block.
 */
export function lineDiff(
  oldText: string,
  newText: string,
  oldStart = 1,
  newStart = 1,
): { lines: DiffLine[]; oldNext: number; newNext: number } {
  const withEol = (text: string) => {
    const s = text.replace(/\r\n/g, "\n");
    return s === "" || s.endsWith("\n") ? s : `${s}\n`;
  };
  const changes = diffLines(withEol(oldText), withEol(newText), {
    maxEditLength: MAX_EDIT,
    timeout: 200,
  }) ?? [
    { value: withEol(oldText), added: false, removed: true, count: 0 },
    { value: withEol(newText), added: true, removed: false, count: 0 },
  ];
  const lines: DiffLine[] = [];
  let oldNo = oldStart;
  let newNo = newStart;
  for (const change of changes) {
    for (const text of splitLines(change.value)) {
      if (change.added) lines.push({ kind: "add", oldNo: null, newNo: newNo++, text });
      else if (change.removed) lines.push({ kind: "del", oldNo: oldNo++, newNo: null, text });
      else lines.push({ kind: "ctx", oldNo: oldNo++, newNo: newNo++, text });
    }
  }
  return { lines, oldNext: oldNo, newNext: newNo };
}

function count(lines: readonly DiffLine[], kind: DiffLine["kind"]): number {
  return lines.reduce((n, line) => n + (line.kind === kind ? 1 : 0), 0);
}

function finish(path: string, lines: DiffLine[], newFile: boolean): FileDiff {
  return { path, lines, added: count(lines, "add"), removed: count(lines, "del"), newFile };
}

/** Each `{old, new}` pair as a hunk, separated by gap lines; the new side shifts by earlier hunks. */
function hunks(
  pairs: readonly { start: number | null; newStart?: number | null; old: string; new: string }[],
): DiffLine[] {
  const lines: DiffLine[] = [];
  let shift = 0;
  pairs.forEach((pair, i) => {
    if (i > 0) lines.push({ kind: "gap", oldNo: null, newNo: null, text: "" });
    const start = pair.start ?? 1;
    const block = lineDiff(pair.old, pair.new, start, pair.newStart ?? start + shift);
    lines.push(...block.lines);
    shift = block.newNext - block.oldNext;
  });
  return lines;
}

/** Rebuild the after-text only when every reported hunk matches the exact pre-edit text. */
function afterEdit(previous: string, occurrences: readonly Obj[]): string | null {
  const offsets = [0];
  for (let i = 0; i < previous.length; i += 1) if (previous[i] === "\n") offsets.push(i + 1);
  const edits = occurrences
    .map((o) => ({
      line: num(o["startLine"]),
      old: str(o["oldText"]),
      next: str(o["newText"]),
    }))
    .sort((a, b) => (b.line ?? 0) - (a.line ?? 0));
  let after = previous;
  let boundary = previous.length;
  for (const edit of edits) {
    if (
      edit.line === null ||
      !Number.isInteger(edit.line) ||
      edit.old === null ||
      edit.next === null
    )
      return null;
    const start = offsets[edit.line - 1];
    if (
      start === undefined ||
      start + edit.old.length > boundary ||
      previous.slice(start, start + edit.old.length) !== edit.old
    )
      return null;
    after = after.slice(0, start) + edit.next + after.slice(start + edit.old.length);
    boundary = start;
  }
  return after;
}

/**
 * The diff of a `file_edit` effect: from the completed output's `occurrences` (exact line numbers),
 * else from the requested change in `detail.input` (single or batch form; numbered from 1, since the
 * position is not known before the edit runs). `null` when neither is present.
 */
export function editDiff(entry: Obj): FileDiff | null {
  const input = inputOf(entry);
  const output = statusOf(entry) === "completed" ? outputOf(entry) : null;
  const path = str(output?.["file"]) ?? str(input?.["filePath"]) ?? "";
  const occurrences = arr(output?.["occurrences"])
    .map(obj)
    .filter((o): o is Obj => o !== null);
  if (occurrences.length > 0) {
    const previous = str(output?.["previousContent"]);
    const after = previous === null ? null : afterEdit(previous, occurrences);
    if (previous !== null && after !== null)
      return finish(path, lineDiff(previous, after).lines, false);
    const pairs = occurrences.map((o) => ({
      start: num(o["startLine"]),
      newStart: num(o["newStartLine"]),
      old: str(o["oldText"]) ?? "",
      new: str(o["newText"]) ?? "",
    }));
    return finish(path, hunks(pairs), false);
  }
  if (input === null) return null;
  const changes = arr(input["changes"])
    .map(obj)
    .filter((o): o is Obj => o !== null);
  const pairs = (changes.length > 0 ? changes : [input]).map((c) => ({
    start: null,
    old: str(c["oldString"]) ?? "",
    new: str(c["newString"]) ?? "",
  }));
  return finish(path, hunks(pairs), false);
}

/**
 * The diff of a `file_write` effect: `previousContent` -> `content` when Vibe reports the old text,
 * else every line of the new content added (a new file, or a write still waiting for approval).
 */
export function writeDiff(entry: Obj): FileDiff | null {
  const input = inputOf(entry);
  const output = statusOf(entry) === "completed" ? outputOf(entry) : null;
  const path = str(output?.["filePath"]) ?? str(input?.["filePath"]) ?? "";
  const content = str(output?.["content"]) ?? str(input?.["content"]);
  if (content === null) return null;
  const previous = str(output?.["previousContent"]);
  const existed = output?.["fileExisted"] === true;
  if (previous !== null) return finish(path, lineDiff(previous, content).lines, false);
  return finish(path, lineDiff("", content).lines, !existed);
}
