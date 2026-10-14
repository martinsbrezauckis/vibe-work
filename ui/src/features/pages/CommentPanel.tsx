/**
 * Comments on the shown page (PROJECT_PAGES "Comments to agent"): the form after a section pick (the
 * section label comes from the page FILE through the core), the page's comments by section, and the detached ones
 * (their section is gone from the page: listed, never dropped). "Send to Vibe" puts the comment into the composer as
 * a context chip citing page and section; the user sends the message.
 * Depends on: protocol types. Called by: PagesTab.
 * Invariants: renders only; comment and page text are plain text; comments live in Vibe Work's data, never in the
 * repository (the panel says so).
 */
import { type ReactElement, useState } from "react";

import type { Comment, PickedSection } from "../../protocol/core";

function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" });
}

function Row({
  comment,
  detached,
  send,
  remove,
}: {
  comment: Comment;
  detached: boolean;
  send: (c: Comment) => void;
  remove: (c: Comment) => void;
}): ReactElement {
  return (
    <li className="pages-comment" data-comment-id={comment.id} data-section={comment.section}>
      <span className="pages-comment-where">
        {detached
          ? `was on “${comment.sectionLabel}”`
          : `§ ${comment.sectionLabel || comment.section}`}{" "}
        · {when(comment.createdMs)}
        {comment.sentMs !== null && " · sent to Vibe"}
      </span>
      <span className="pages-comment-text">{comment.text}</span>
      <span className="pages-comment-acts">
        <button
          type="button"
          className="pages-btn"
          onClick={() => {
            send(comment);
          }}
        >
          Send to Vibe
        </button>
        <button
          type="button"
          className="pages-btn"
          onClick={() => {
            remove(comment);
          }}
        >
          Delete
        </button>
      </span>
    </li>
  );
}

/** The panel. `draft` is the section the user just picked (opens the form). */
export function CommentPanel({
  attached,
  detached,
  draft,
  save,
  cancel,
  send,
  remove,
}: {
  attached: readonly Comment[];
  detached: readonly Comment[];
  draft: PickedSection | null;
  save: (text: string, andSend: boolean) => void;
  cancel: () => void;
  send: (c: Comment) => void;
  remove: (c: Comment) => void;
}): ReactElement {
  const [text, setText] = useState("");
  const total = attached.length + detached.length;
  return (
    <section className="pages-comments" aria-label="Comments on this page">
      {draft !== null && (
        <form
          className="pages-cbox"
          onSubmit={(event) => {
            event.preventDefault();
          }}
        >
          <label htmlFor="pages-comment-text">
            Comment on “{draft.label || draft.section}”
            {draft.quote !== null && (
              <span className="pages-quote"> on “{draft.quote.slice(0, 120)}”</span>
            )}
          </label>
          <textarea
            id="pages-comment-text"
            value={text}
            placeholder="Vibe gets it with the page and section."
            onChange={(event) => {
              setText(event.target.value);
            }}
          />
          <div className="pages-cbox-row">
            <button
              type="button"
              className="pages-btn go"
              disabled={text.trim() === ""}
              onClick={() => {
                save(text, true);
                setText("");
              }}
            >
              Send to Vibe
            </button>
            <button
              type="button"
              className="pages-btn"
              disabled={text.trim() === ""}
              onClick={() => {
                save(text, false);
                setText("");
              }}
            >
              Save
            </button>
            <button
              type="button"
              className="pages-btn"
              onClick={() => {
                setText("");
                cancel();
              }}
            >
              Cancel
            </button>
            <span className="pages-note">Saved in Vibe Work, not in the repo.</span>
          </div>
        </form>
      )}
      {total > 0 && (
        <details open={draft === null}>
          <summary>
            {total} comment{total === 1 ? "" : "s"} on this page
          </summary>
          <ul className="pages-comment-list">
            {attached.map((c) => (
              <Row key={c.id} comment={c} detached={false} send={send} remove={remove} />
            ))}
          </ul>
          {detached.length > 0 && (
            <div className="pages-detached">
              <b>
                {detached.length} detached comment{detached.length === 1 ? "" : "s"}
              </b>
              : the section they were on is gone from the page, so they are listed here instead of
              dropped.
              <ul className="pages-comment-list">
                {detached.map((c) => (
                  <Row key={c.id} comment={c} detached send={send} remove={remove} />
                ))}
              </ul>
            </div>
          )}
        </details>
      )}
    </section>
  );
}
