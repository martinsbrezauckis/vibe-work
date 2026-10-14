/**
 * One comment under its rows: where it is, its text, and "Send to Vibe" / "Delete". Used inline
 * under the last commented row (unified), below the hunk (side by side) and in the panel (outdated comments,
 * shown with their quoted lines and why they are outdated).
 * Depends on: protocol types. Called by: HunkLines, LineCommentsPanel.
 * Invariants: comment and diff text are rendered as plain text only.
 */
import type { ReactElement } from "react";

import type { LineCommentView } from "../../protocol/core";
import { rangeText } from "./lineCommentModel";

function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" });
}

/** The comment. `quote` shows the stored lines (outdated comments). */
export function LineCommentThread(props: {
  view: LineCommentView;
  quote?: boolean;
  send: (id: string) => void;
  remove: (id: string) => void;
}): ReactElement {
  const { view, send, remove } = props;
  const { comment } = view;
  const outdated = view.state === "outdated";
  return (
    <div
      className="lc-thread"
      role="group"
      aria-label={`Comment on ${comment.path}, ${rangeText(comment.lines)}`}
      data-comment-id={comment.id}
      data-state={view.state}
    >
      <span className="lc-where">
        {comment.path} · {rangeText(comment.lines)} · {when(comment.createdMs)}
        {comment.sentMs !== null && " · sent to Vibe"}
        {outdated && " · outdated"}
      </span>
      {outdated && <span className="lc-reason">{view.reason}.</span>}
      {props.quote === true && (
        <pre className="lc-quote">
          {comment.lines
            .map((l) => `${l.kind === "add" ? "+" : l.kind === "del" ? "-" : " "}${l.text}`)
            .join("\n")}
        </pre>
      )}
      <span className="lc-text">{comment.text}</span>
      <span className="lc-acts">
        <button
          type="button"
          className="mini"
          data-action="send-comment"
          onClick={() => {
            send(comment.id);
          }}
        >
          Send to Vibe
        </button>
        <button
          type="button"
          className="mini"
          data-action="delete-comment"
          onClick={() => {
            remove(comment.id);
          }}
        >
          Delete
        </button>
      </span>
    </div>
  );
}
