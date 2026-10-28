/**
 * Folders without git: the files each turn wrote or edited through Vibe's file tools, shown
 * with the transcript's own diff view (`DiffView` over the session's effect entries), "Undo turn"
 * (previewed, then `session/rewind`), and the one-time version-history offer for code folders.
 * Depends on: store.ts, transcript model (`editDiff`, `writeDiff`, entry helpers) and `DiffView`, shell
 * hooks (the session and its children's entries). Called by: ChangesTab.
 * Invariant: the limits of this undo are said where the user decides (here and in the confirmation):
 * whole turns only, later turns go too, and only files Vibe saved before changing them.
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useMemo } from "react";

import type { ChangesView } from "../../protocol/core";
import { useCoreState } from "../shell/hooks";
import { editDiff, writeDiff, firstChangedLine } from "../transcript/model/diff";
import { type Obj, arr, detailOf, entryId, obj } from "../transcript/model/entries";
import { DiffView } from "../transcript/rows/DiffView";
import type { Changes } from "./store";

/** One edit or write, titled with the path relative to the session folder (Vibe reports it absolute). */
function EntryDiff({
  entry,
  path,
  sessionId,
}: {
  entry: Obj;
  path: string;
  sessionId: string;
}): ReactElement | null {
  const diff = detailOf(entry)["kind"] === "file_edit" ? editDiff(entry) : writeDiff(entry);
  return diff === null ? null : (
    <div data-review-path={path} data-review-line={firstChangedLine(diff)}>
      <DiffView diff={{ ...diff, path }} id={`changes:${sessionId}:${entryId(entry)}`} />
    </div>
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
  const sessions = useCoreState((s) => s.sessions);
  const entries = useMemo(() => {
    const bySession = new Map<string, Map<string, Obj>>();
    const ids = new Set([
      view.sessionId,
      ...view.turns.map((turn) => turn.subagent?.sessionId ?? view.sessionId),
    ]);
    for (const id of ids) {
      const byId = new Map<string, Obj>();
      for (const raw of arr(sessions[id]?.state?.["history"])) {
        const entry = obj(raw);
        if (entry !== null) byId.set(entryId(entry), entry);
      }
      bySession.set(id, byId);
    }
    return bySession;
  }, [sessions, view.sessionId, view.turns]);
  return (
    <div className="turn-undo">
      {view.offerHistory && !foundRepo && (
        <div className="history-offer" role="note">
          <span>Keep versions of this folder so you can save work and go back.</span>
          <Button
            data-testid="changes.turn-undo-ask-history"
            type="button"
            appearance="compact"
            data-action="enable-history"
            onClick={() => void changes.askHistory(view.sessionId)}
          >
            Turn on version history
          </Button>
        </div>
      )}
      {[...view.turns].reverse().map((turn) => {
        const source = turn.subagent?.sessionId ?? view.sessionId;
        const label = `${turn.subagent === undefined ? "" : `${turn.subagent.name} · `}Turn ${String(turn.number)}`;
        return (
          <section
            data-item-id={`${source}:${turn.turnId}`}
            key={`${source}:${turn.turnId}`}
            className="turn-changes"
            aria-label={label}
          >
            <div className="hunk-h">
              <span>
                {label} · {turn.files.length} {turn.files.length === 1 ? "file" : "files"}
              </span>
              <span className="sp" />
              {turn.subagent === undefined && turn.anchorEntryId !== null && (
                <Button
                  data-testid="changes.turn-undo-ask-undo"
                  type="button"
                  appearance="compact"
                  disabled={busy}
                  data-action="undo-turn"
                  onClick={() => void changes.askUndo(view.sessionId, turn.turnId, turn.number)}
                >
                  Undo turn
                </Button>
              )}
            </div>
            {turn.files.map((file) =>
              file.entryIds.map((id) => {
                const entry = entries.get(source)?.get(id);
                return entry === undefined ? (
                  <div key={id} className="review-note">
                    {file.path}
                  </div>
                ) : (
                  <EntryDiff key={id} entry={entry} path={file.path} sessionId={source} />
                );
              }),
            )}
          </section>
        );
      })}
    </div>
  );
}
