/**
 * Git folders: the bulk bar (live state, Keep all, Revert all), the changed-file list, and the selected
 * file's review. The Changes layout (file list beside the diff when the panel is wide).
 * Depends on: model.ts, store.ts, FileReview. Called by: ChangesTab.
 */
import { Button } from "../../components/Controls";
import type { ReactElement } from "react";

import { useStore } from "../../state/createStore";
import type { ChangesView } from "../../protocol/core";
import { FileReview } from "./FileReview";
import { completionText, fileNote, shownFile, shownInFull, statText, turnText } from "./model";
import type { Changes } from "./store";

/** The review of a git session. */
export function GitReview(props: {
  changes: Changes;
  view: ChangesView;
  busy: boolean;
  running: boolean;
}): ReactElement {
  const { changes, view, busy, running } = props;
  const completion = useStore(changes.store, (s) => s.completion[view.sessionId]);
  const selected = useStore(changes.store, (s) => s.selected[view.sessionId] ?? null);
  const file = shownFile(view, selected);
  const live = view.watching || (running && view.mode !== "uncommitted");
  if (view.files.length === 0) {
    return (
      <div className="empty">
        {completion === "history" ? (
          <p className="key-success">{completionText(completion)}</p>
        ) : (
          (completionText(completion) ??
          (view.mode === "uncommitted"
            ? "No uncommitted files in this repository."
            : "No file changes in this session yet."))
        )}
        {live ? <br /> : null}
        {live ? "This list refreshes as files change." : null}
      </div>
    );
  }
  const n = view.files.length;
  const unseen = view.files.filter((f) => !shownInFull(f) && f.content !== "submodule").length;
  // A lost starting point: changes against the last commit, read only (the banner offers a restart).
  const readOnly = view.baselineLost || view.mode === "uncommitted";
  return (
    <div className="chg">
      <div>
        <div className="bulk">
          <span className="live2">
            <span className={`px ${running && live ? "run" : "ok"}`} aria-hidden="true" />
            {running && live
              ? "Live: refreshes as Vibe edits"
              : live
                ? "Updates automatically"
                : "Refreshed"}
          </span>
          {!readOnly && (
            <>
              <button
                data-testid="changes.git-review-keep-all"
                type="button"
                className="mini"
                disabled={busy}
                data-action="keep-all"
                onClick={() => void changes.keepAll(view.sessionId)}
              >
                Keep all
              </button>
              <Button
                data-testid="changes.git-review-ask-revert-all"
                type="button"
                small
                variant="danger"
                disabled={busy}
                data-action="revert-all"
                onClick={() => {
                  changes.askRevertAll(view.sessionId);
                }}
              >
                Revert all
              </Button>
            </>
          )}
        </div>
        <ul className="files" aria-label={`${String(n)} changed ${n === 1 ? "file" : "files"}`}>
          {view.files.map((f) => {
            const turn = turnText(f.turn);
            return (
              <li data-item-id={f.path} key={f.path}>
                <button
                  data-testid="changes.git-review-select"
                  type="button"
                  aria-current={f.path === file?.path}
                  data-path={f.path}
                  onClick={() => {
                    changes.select(view.sessionId, f.path);
                  }}
                >
                  <span>{f.path}</span>
                  <span className="smeta">{statText(f)}</span>
                  <span className="fst">
                    {fileNote(f)}
                    {turn !== null ? ` · ${turn}` : ""}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
        {unseen > 0 && !readOnly && (
          <p className="bnote">
            Keep all keeps the files shown line by line. {unseen}{" "}
            {unseen === 1 ? "file is" : "files are"} not shown that way and stay until you keep them
            one by one.
          </p>
        )}
        {view.notes.map((note) => (
          <p key={note} className="bnote">
            {note}
          </p>
        ))}
        {view.mode === "uncommitted" && (
          <p className="bnote">
            Uncommitted files in {view.folder}, compared with this repository&apos;s last commit.
          </p>
        )}
      </div>
      {file !== null && (
        <FileReview
          changes={changes}
          sessionId={view.sessionId}
          file={file}
          busy={busy}
          readOnly={readOnly}
        />
      )}
    </div>
  );
}
