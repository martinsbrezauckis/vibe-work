/**
 * The selected file: its name with Keep file / Revert file, the layout switch, and its hunks (or why there
 * are none: binary, too large, a submodule, a mode change).
 * Depends on: model.ts, store.ts, HunkBlock. Called by: GitReview.
 */
import type { ReactElement } from "react";

import { Selector } from "../../components/Selector";
import { useStore } from "../../state/createStore";
import type { FileChange } from "../../protocol/core";
import { HunkBlock } from "./HunkBlock";
import { fileActions, fileNote, turnText } from "./model";
import type { Changes } from "./store";

/** The review of one file. */
export function FileReview(props: {
  changes: Changes;
  sessionId: string;
  file: FileChange;
  busy: boolean;
  /** No keep/revert (the session's starting point was lost; changes are against the last commit). */
  readOnly?: boolean;
}): ReactElement {
  const { changes, sessionId, file, busy } = props;
  const readOnly = props.readOnly === true;
  const layout = useStore(changes.store, (s) => s.layout);
  const turn = turnText(file.turn);
  return (
    <div className="file-review">
      <div className="filebar">
        <span className="fname" title={file.path}>
          {file.path}
        </span>
        {fileActions(file) && !readOnly && (
          <>
            <button
              type="button"
              className="mini"
              disabled={busy}
              data-action="keep-file"
              onClick={() => void changes.keepFile(sessionId, file)}
            >
              Keep file
            </button>
            <button
              type="button"
              className="mini"
              disabled={busy}
              data-action="revert-file"
              onClick={() => void changes.revertFile(sessionId, file)}
            >
              Revert file
            </button>
          </>
        )}
      </div>
      <div className="dtool">
        <Selector
          label="Diff layout"
          value={layout}
          options={[
            { value: "split", label: "Side by side" },
            { value: "unified", label: "Unified" },
          ]}
          onChange={(value) => {
            changes.setLayout(value);
          }}
        />
        <span className="fmeta">
          {fileNote(file)}
          {turn !== null ? ` · ${turn}` : ""}
        </span>
      </div>
      <div className="review">
        {file.hunks.length === 0 ? (
          <div className="review-note">{fileNote(file)}: keep or revert the whole file.</div>
        ) : (
          file.hunks.map((hunk, i) => (
            <HunkBlock
              key={hunk.id}
              changes={changes}
              sessionId={sessionId}
              file={file}
              hunk={hunk}
              index={i}
              layout={layout}
              busy={busy}
              readOnly={readOnly}
            />
          ))
        )}
      </div>
      <p className="bnote">
        Revert puts the lines back the way they were when this session started. Keep accepts them
        and leaves the file as it is.
      </p>
    </div>
  );
}
