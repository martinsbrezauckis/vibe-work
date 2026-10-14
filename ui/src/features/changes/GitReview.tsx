/**
 * Git folders: the bulk bar (live state, Keep all, Revert all), the changed-file list, and the selected
 * file's review. The Changes layout (file list beside the diff when the panel is wide).
 * Depends on: model.ts, store.ts, FileReview. Called by: ChangesTab.
 */
import type { ReactElement } from "react";

import { useStore } from "../../state/createStore";
import type { ChangesView } from "../../protocol/core";
import { FileReview } from "./FileReview";
import { fileNote, shownFile, shownInFull, statText, turnText } from "./model";
import type { Changes } from "./store";

/** The review of a git session. */
export function GitReview(props: {
  changes: Changes;
  view: ChangesView;
  busy: boolean;
  running: boolean;
}): ReactElement {
  const { changes, view, busy, running } = props;
  const selected = useStore(changes.store, (s) => s.selected[view.sessionId] ?? null);
  const file = shownFile(view, selected);
  const live = view.watching || running;
  if (view.files.length === 0) {
    return (
      <div className="empty">
        No file changes in this session yet.
        {live ? <br /> : null}
        {live ? "This list refreshes as files change." : null}
      </div>
    );
  }
  const n = view.files.length;
  const unseen = view.files.filter((f) => !shownInFull(f) && f.content !== "submodule").length;
  // A lost starting point: changes against the last commit, read only (the banner offers a restart).
  const readOnly = view.baselineLost;
  return (
    <div className="chg">
      <div>
        <div className="bulk">
          <span className="live2">
            <span className={`px ${running ? "run" : "ok"}`} aria-hidden="true" />
            {running ? "Live: refreshes as Vibe edits" : view.watching ? "Up to date" : "Refreshed"}
          </span>
          {!readOnly && (
            <>
              <button
                type="button"
                className="mini"
                disabled={busy}
                data-action="keep-all"
                onClick={() => void changes.keepAll(view.sessionId)}
              >
                Keep all
              </button>
              <button
                type="button"
                className="mini"
                disabled={busy}
                data-action="revert-all"
                onClick={() => {
                  changes.askRevertAll(view.sessionId);
                }}
              >
                Revert all
              </button>
            </>
          )}
        </div>
        <ul className="files" aria-label={`${String(n)} changed ${n === 1 ? "file" : "files"}`}>
          {view.files.map((f) => {
            const turn = turnText(f.turn);
            return (
              <li key={f.path}>
                <button
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
        {unseen > 0 && (
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
