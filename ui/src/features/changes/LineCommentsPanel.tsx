/**
 * The comments bar of the Changes tab: how many comments the session has, "Send N to Vibe" for the
 * unsent ones (one chip each, so one message), and the outdated comments (their lines changed or left the diff)
 * listed with their quoted lines, never dropped. Renders nothing while there are no comments.
 * Depends on: lineComments.ts via context, LineCommentThread. Called by: ChangesTab.
 * Invariants: sending only inserts chips into the composer; the user sends the message.
 */
import { type ReactElement, useEffect } from "react";

import { useStore } from "../../state/createStore";
import { useLineComments } from "./LineCommentsContext";
import { LineCommentThread } from "./LineCommentThread";

const NONE: never[] = [];

/** The bar for `sessionId`. */
export function LineCommentsPanel({ sessionId }: { sessionId: string }): ReactElement | null {
  const ctl = useLineComments();
  if (ctl === null) return null;
  return <Panel ctl={ctl} sessionId={sessionId} />;
}

function Panel({
  ctl,
  sessionId,
}: {
  ctl: NonNullable<ReturnType<typeof useLineComments>>;
  sessionId: string;
}): ReactElement | null {
  const all = useStore(ctl.store, (s) => s.comments[sessionId] ?? NONE);
  const error = useStore(ctl.store, (s) => s.error[sessionId] ?? null);
  useEffect(() => {
    void ctl.load(sessionId);
  }, [ctl, sessionId]);
  if (all.length === 0 && error === null) return null;
  const unsent = all.filter((c) => c.comment.sentMs === null);
  const outdated = all.filter((c) => c.state === "outdated");
  return (
    <section className="lc-panel" aria-label="Comments on the changes">
      <div className="lc-bar">
        <span>
          {all.length} comment{all.length === 1 ? "" : "s"}
          {outdated.length > 0 ? `, ${String(outdated.length)} outdated` : ""}
        </span>
        <button
          type="button"
          className="mini"
          data-action="send-all-comments"
          disabled={unsent.length === 0}
          onClick={() => {
            void ctl.send(
              sessionId,
              unsent.map((c) => c.comment.id),
            );
          }}
        >
          Send {unsent.length} to Vibe
        </button>
        <span className="lc-note">Kept in Vibe Work, not in the repo.</span>
      </div>
      {error !== null && (
        <p className="changes-error" role="status">
          {error}
        </p>
      )}
      {outdated.length > 0 && (
        <details className="lc-outdated" open>
          <summary>Outdated comments</summary>
          {outdated.map((c) => (
            <LineCommentThread
              key={c.comment.id}
              view={c}
              quote
              send={(id) => {
                void ctl.send(sessionId, [id]);
              }}
              remove={(id) => {
                void ctl.remove(sessionId, id);
              }}
            />
          ))}
        </details>
      )}
    </section>
  );
}
