/**
 * Diff line comments, the pure part: which rows a comment covers, how a drag or shift-click
 * becomes a range, and the context chip a comment turns into when it is sent to Vibe.
 * Depends on: protocol/core types, pages/model (`Chip`). Called by: lineComments.ts (controller), the row and
 * thread components, commentRoutes.ts, UI tests.
 * Invariants:
 * - A chip is data: its text quotes the diff and the comment, and starts with our own sentence, never with the
 *   user's text, so a comment such as `!rm -rf` or `/compact` can never be read as a command (the composer sends
 *   chips as resource blocks).
 * - The quoted lines come from the core's current diff when the comment is still attached, else from the lines
 *   stored with the comment (and the chip says they may have changed).
 */
import type { DiffLine, LineCommentView } from "../../protocol/core";
import type { Chip } from "../pages/model";

/** Most lines one comment can cover (the core refuses more). */
export const MAX_COMMENT_LINES = 40;

/** A range of rows inside one hunk, `from <= to`. */
export interface Range {
  from: number;
  to: number;
}

/** The range between the row the user started at and the row they extend to (either order), clipped. */
export function rangeBetween(anchor: number, to: number, hunkLines: number): Range {
  const last = Math.max(0, hunkLines - 1);
  const [a, b] = anchor <= to ? [anchor, to] : [to, anchor];
  const from = Math.min(a, last);
  return { from, to: Math.min(b, last, from + MAX_COMMENT_LINES - 1) };
}

/** True when `index` of hunk `hunkId` is covered by an attached comment of `path`. */
export function commentsAt(
  comments: readonly LineCommentView[],
  path: string,
  hunkId: string,
  index: number,
): LineCommentView[] {
  return comments.filter(
    (c) =>
      c.state === "attached" &&
      c.comment.path === path &&
      c.hunkId === hunkId &&
      index >= c.from &&
      index <= c.to,
  );
}

/** Attached comments of `hunkId` whose last row is `index` (their thread opens under it). */
export function threadsEndingAt(
  comments: readonly LineCommentView[],
  path: string,
  hunkId: string,
  index: number,
): LineCommentView[] {
  return comments.filter(
    (c) =>
      c.state === "attached" && c.comment.path === path && c.hunkId === hunkId && c.to === index,
  );
}

/** The unified-diff prefix of a line. */
const sign = (line: DiffLine): string =>
  line.kind === "add" ? "+" : line.kind === "del" ? "-" : " ";

/** "line 12", "lines 12–15", or "removed line 7" when no quoted line exists in the current file. */
export function rangeText(lines: readonly DiffLine[]): string {
  const now = lines.map((l) => l.newNo).filter((n): n is number => n !== null);
  if (now.length > 0) {
    const [first, last] = [now[0] as number, now[now.length - 1] as number];
    return first === last ? `line ${String(first)}` : `lines ${String(first)}–${String(last)}`;
  }
  const old = lines.map((l) => l.oldNo).filter((n): n is number => n !== null);
  const [first, last] = [old[0] ?? 0, old[old.length - 1] ?? 0];
  return first === last
    ? `removed line ${String(first)}`
    : `removed lines ${String(first)}–${String(last)}`;
}

/** The first line number shown in a label (`path:12`). */
export function labelLine(lines: readonly DiffLine[]): string {
  const first = lines[0];
  if (first === undefined) return "";
  return String(first.newNo ?? first.oldNo ?? "");
}

/** A code fence longer than any run of backticks in `text`, so quoted code cannot close it early. */
function fence(text: string): string {
  const longest = Math.max(0, ...(text.match(/`+/g) ?? []).map((run) => run.length));
  return "`".repeat(Math.max(3, longest + 1));
}

/**
 * The chip a comment becomes. `current` are the quoted lines in the diff as it is now (attached comments); with
 * `null` the lines stored with the comment are quoted and the chip says they may have changed.
 */
export function commentChip(view: LineCommentView, current: readonly DiffLine[] | null): Chip {
  const { comment } = view;
  const lines = current ?? comment.lines;
  const quoted = lines.map((l) => `${sign(l)}${l.text}`).join("\n");
  const fenced = fence(quoted);
  const out = [
    `Comment on ${comment.path}, ${rangeText(lines)} (a change made in this session; "+" added, "-" removed):`,
    `${fenced}diff`,
    quoted,
    fenced,
  ];
  if (current === null) {
    out.push(
      "Note: these lines have changed or left the diff since the comment was written, so they may not match the file now.",
    );
  }
  out.push(`Comment: ${comment.text}`);
  return {
    label: `Diff comment · ${comment.path}:${labelLine(lines)}`,
    source: "diff",
    text: out.join("\n"),
  };
}
