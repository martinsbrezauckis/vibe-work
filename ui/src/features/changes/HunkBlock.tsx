/**
 * One hunk of a changed file: header ("Hunk 2 · lines 12–18 · turn 3") with Keep / Revert, and its lines
 * (`HunkLines`: unified with the `DiffLineRow`, or side by side, with comment buttons).
 * Depends on: model.ts, store.ts (actions), transcript rows (`DiffLineRow`). Called by: FileReview.
 * Invariants: buttons only call the store; while the session is busy they are disabled (one action at a
 * time, so a second click never acts on a diff that is about to change).
 */
import { type ReactElement, useState } from "react";

import { Icon } from "../../components/Icon";

import type { FileChange, Hunk } from "../../protocol/core";
import { HunkLines } from "./HunkLines";
import { hunkActions, hunkRange } from "./model";
import type { Changes } from "./store";

/** One hunk with its actions. */
export function HunkBlock(props: {
  changes: Changes;
  sessionId: string;
  file: FileChange;
  hunk: Hunk;
  index: number;
  layout: "split" | "unified";
  busy: boolean;
  /** No keep/revert buttons. */
  readOnly?: boolean;
}): ReactElement {
  const { changes, sessionId, file, hunk, index, layout, busy } = props;
  const label = `Hunk ${String(index + 1)} · ${hunkRange(hunk)}`;
  // (feel item 14): a kept or reverted hunk folds shut (200 ms) while the store applies it; if it is still
  // here when the action ends (it failed, or the diff did not change), it opens again.
  const [leaving, setLeaving] = useState(false);
  const act = (run: () => Promise<unknown>) => {
    setLeaving(true);
    void run().finally(() => {
      setLeaving(false);
    });
  };
  return (
    <div
      className={`hunk${leaving ? " leaving" : ""}`}
      data-hunk-id={hunk.id}
      role="region"
      aria-label={label}
    >
      <div className="hunk-h">
        <span>{label}</span>
        <span className="sp" />
        {hunkActions(file) && props.readOnly !== true && (
          <span className="hunk-acts">
            <button
              type="button"
              className="mini"
              disabled={busy}
              data-action="keep-hunk"
              aria-label={`Keep ${label}`}
              onClick={() => {
                act(() => changes.keepHunk(sessionId, file, hunk));
              }}
            >
              <Icon name="check" size={14} />
              Keep
            </button>
            <button
              type="button"
              className="mini"
              disabled={busy}
              data-action="revert-hunk"
              aria-label={`Revert ${label}`}
              onClick={() => {
                act(() => changes.revertHunk(sessionId, file, hunk));
              }}
            >
              <Icon name="undo-2" size={14} />
              Revert
            </button>
          </span>
        )}
      </div>
      <HunkLines sessionId={sessionId} path={file.path} hunk={hunk} layout={layout} />
    </div>
  );
}
