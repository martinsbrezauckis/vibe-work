/**
 * Folders without git: the files each turn wrote or edited through Vibe's file tools, shown
 * with the transcript's own diff view (`DiffView` over the session's effect entries), "Undo turn"
 * (previewed, then `session/rewind`), and the one-time version-history offer for code folders.
 * Depends on: store.ts, transcript model (`editDiff`, `writeDiff`, entry helpers) and `DiffView`, shell
 * hooks (the session's entries). Called by: ChangesTab.
 * Invariant: the limits of this undo are said where the user decides (here and in the confirmation):
 * whole turns only, later turns go too, and only files Vibe saved before changing them.
 */
import { type ReactElement, useMemo } from "react";

import type { ChangesView } from "../../protocol/core";
import { useCoreState } from "../shell/hooks";
import { editDiff, writeDiff } from "../transcript/model/diff";
import { type Obj, arr, detailOf, entryId, obj } from "../transcript/model/entries";
import { DiffView } from "../transcript/rows/DiffView";
import type { Changes } from "./store";

/** One edit or write, titled with the path relative to the session folder (Vibe reports it absolute). */
function EntryDiff({ entry, path }: { entry: Obj; path: string }): ReactElement | null {
  const diff = detailOf(entry)["kind"] === "file_edit" ? editDiff(entry) : writeDiff(entry);
  return diff === null ? null : (
    <DiffView diff={{ ...diff, path }} id={`changes:${entryId(entry)}`} />
  );
}

/** The turns' changes with Undo. */
export function TurnUndo(props: {
  changes: Changes;
  view: ChangesView;
  busy: boolean;
  foundRepo: boolean;
}): ReactElement {
  const { changes, view, busy, foundRepo } = props;
  const history = useCoreState((s) => s.sessions[view.sessionId]?.state?.["history"]);
  const entries = useMemo(() => {
    const byId = new Map<string, Obj>();
    for (const raw of arr(history)) {
      const entry = obj(raw);
      if (entry !== null) byId.set(entryId(entry), entry);
    }
    return byId;
  }, [history]);
  return (
    <div className="turn-undo">
      {view.offerHistory && !foundRepo && (
        <div className="history-offer" role="note">
          <span>Turn on version history</span>
          <button
            type="button"
            className="mini"
            data-action="enable-history"
            onClick={() => void changes.askHistory(view.sessionId)}
          >
            Turn on version history
          </button>
        </div>
      )}
      {[...view.turns].reverse().map((turn) => (
        <section
          key={turn.turnId}
          className="turn-changes"
          aria-label={`Turn ${String(turn.number)}`}
        >
          <div className="hunk-h">
            <span>
              Turn {turn.number} · {turn.files.length} {turn.files.length === 1 ? "file" : "files"}
            </span>
            <span className="sp" />
            {turn.anchorEntryId !== null && (
              <button
                type="button"
                className="mini"
                disabled={busy}
                data-action="undo-turn"
                onClick={() => void changes.askUndo(view.sessionId, turn.turnId, turn.number)}
              >
                Undo turn
              </button>
            )}
          </div>
          {turn.files.map((file) =>
            file.entryIds.map((id) => {
              const entry = entries.get(id);
              return entry === undefined ? (
                <div key={id} className="review-note">
                  {file.path}
                </div>
              ) : (
                <EntryDiff key={id} entry={entry} path={file.path} />
              );
            }),
          )}
        </section>
      ))}
    </div>
  );
}
