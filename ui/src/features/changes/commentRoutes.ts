/**
 * The diff-comment actions of the `ui/changes` route: an external agent lists, adds, removes and
 * sends line comments the way a user does. Called by changesRoutes.ts, which owns the route.
 * Actions: `comment {path, line, endLine?, side?, text, send?}` (current file numbering; `side: "old"` for
 * removed lines), `removeComment {id}`, `sendComments {ids?}` (default: every unsent attached comment).
 * Depends on: lineComments.ts, store.ts (the diff the tab shows), state/uiControl. Called by: changesRoutes.ts.
 * Invariants: bodies are validated (`invalidParams`); while a route request is served every core call carries
 * the Debug API origin (a label: no native confirmation), and chips go into the composer through its
 * Debug API facade (marked, sent with that origin); nothing here can run a command.
 */
import { UiControlError } from "../../state/uiControl";
import type { LineComments } from "./lineComments";
import { rangeText } from "./lineCommentModel";
import type { Changes } from "./store";

/** The comments as the route reports them. */
export function commentsState(comments: LineComments, sessionId: string | null) {
  const list = sessionId === null ? [] : (comments.store.get().comments[sessionId] ?? []);
  return list.map((c) => ({
    id: c.comment.id,
    path: c.comment.path,
    lines: rangeText(c.comment.lines),
    text: c.comment.text,
    state: c.state,
    reason: c.state === "outdated" ? c.reason : null,
    sent: c.comment.sentMs !== null,
  }));
}

function row(input: Record<string, unknown>, name: string): number | null {
  const value = input[name];
  if (value === undefined) return null;
  if (typeof value !== "number" || !Number.isInteger(value) || value < 1)
    throw new UiControlError("invalidParams", `${name} must be a positive integer`);
  return value;
}

/** Runs one comment action; returns nothing (the route answers with its state). */
export async function commentAction(
  action: string,
  input: Record<string, unknown>,
  ctx: { changes: Changes; comments: LineComments; sessionId: string },
): Promise<void> {
  const { changes, comments, sessionId } = ctx;
  switch (action) {
    case "comment": {
      const { path, text } = input;
      if (typeof path !== "string" || path === "")
        throw new UiControlError("invalidParams", "path must be a non-empty string");
      if (typeof text !== "string" || text.trim() === "")
        throw new UiControlError("invalidParams", "text must be a non-empty string");
      const line = row(input, "line");
      if (line === null) throw new UiControlError("invalidParams", "line is required");
      const end = row(input, "endLine") ?? line;
      const side = input["side"] === "old" ? "old" : "new";
      const file = changes.store.get().views[sessionId]?.files.find((f) => f.path === path);
      if (file === undefined)
        throw new UiControlError("invalidParams", "that file is not in the changes");
      const number = (l: { oldNo: number | null; newNo: number | null }) =>
        side === "old" ? l.oldNo : l.newNo;
      for (const hunk of file.hunks) {
        const from = hunk.lines.findIndex((l) => number(l) === line);
        const to = hunk.lines.findIndex((l) => number(l) === end);
        if (from < 0 || to < 0) continue;
        comments.begin(sessionId, path, hunk.id, from, hunk.lines.length, false);
        comments.begin(sessionId, path, hunk.id, to, hunk.lines.length, true);
        const id = await comments.save(text, input["send"] === true, "debug-api");
        if (id === null)
          throw new UiControlError(
            "invalidParams",
            comments.store.get().error[sessionId] ?? "the comment was not saved",
          );
        return;
      }
      throw new UiControlError(
        "invalidParams",
        "no such line (or the lines are in different hunks)",
      );
    }
    case "removeComment": {
      const id = input["id"];
      if (typeof id !== "string" || id === "")
        throw new UiControlError("invalidParams", "id must be a non-empty string");
      if (!(await comments.remove(sessionId, id)))
        throw new UiControlError(
          "invalidParams",
          comments.store.get().error[sessionId] ?? "the comment was not removed",
        );
      return;
    }
    case "sendComments": {
      const given = input["ids"];
      if (
        given !== undefined &&
        !(Array.isArray(given) && given.every((x) => typeof x === "string"))
      )
        throw new UiControlError("invalidParams", "ids must be a list of comment ids");
      const unsent = (comments.store.get().comments[sessionId] ?? [])
        .filter((c) => c.comment.sentMs === null && c.state === "attached")
        .map((c) => c.comment.id);
      await comments.send(sessionId, Array.isArray(given) ? given : unsent, "debug-api");
      return;
    }
    default:
      throw new UiControlError("invalidParams", `unknown comment action ${action}`);
  }
}
