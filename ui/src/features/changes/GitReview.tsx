/**
 * Git folders: the bulk bar (live state, Keep all, Revert all), the changed-file list (each row with its
 * right-click menu, `changesMenus.ts`), and the selected file's review. The Changes layout (file list beside the
 * diff when the panel is wide).
 * Depends on: model.ts, store.ts, FileReview, changesMenus, RowMenu. Called by: ChangesTab.
 */
import { Button } from "../../components/Controls";
import type { ReactElement } from "react";
import { createPortal } from "react-dom";
import { RowMenu, RowMenuButton, useRowMenu } from "../../components/RowMenu";
import { useApp } from "../shell/AppContext";
import { changedFileMenu } from "./changesMenus";

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
  const app = useApp();
  const fileMenu = useRowMenu<string>();
  const menuFile = view.files.find((f) => f.path === fileMenu.at?.target);
  const built =
    menuFile === undefined
      ? null
      : changedFileMenu(
          app,
          changes,
          view.sessionId,
          menuFile,
          view.baselineLost || view.mode === "uncommitted",
        );
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
              <Button
                data-testid="changes.git-review-keep-all"
                type="button"
                appearance="compact"
                disabled={busy}
                data-action="keep-all"
                onClick={() => void changes.keepAll(view.sessionId)}
              >
                Keep all
              </Button>
              <Button
                data-testid="changes.git-review-ask-revert-all"
                type="button"
                shape="outlined"
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
              <li data-item-id={f.path} key={f.path} {...fileMenu.triggers(f.path)}>
                <button
                  data-testid="changes.git-review-select"
                  type="button"
                  aria-current={f.path === file?.path}
                  tabIndex={f.path === file?.path ? 0 : -1}
                  onKeyDown={(event) => {
                    const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
                    if (!keys.includes(event.key)) return;
                    event.preventDefault();
                    const rows = [
                      ...(event.currentTarget
                        .closest("ul")
                        ?.querySelectorAll<HTMLButtonElement>("button[data-path]") ?? []),
                    ];
                    const index = rows.indexOf(event.currentTarget);
                    const next =
                      event.key === "Home"
                        ? 0
                        : event.key === "End"
                          ? rows.length - 1
                          : Math.max(
                              0,
                              Math.min(
                                rows.length - 1,
                                index + (event.key === "ArrowDown" ? 1 : -1),
                              ),
                            );
                    const row = rows[next];
                    if (row?.dataset["path"] === undefined) return;
                    changes.select(view.sessionId, row.dataset["path"]);
                    row.focus();
                  }}
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
                <RowMenuButton
                  testId="changes.file-more"
                  itemId={f.path}
                  label={`Actions for ${f.path}`}
                  expanded={fileMenu.at?.target === f.path}
                  onOpen={(button) => {
                    fileMenu.openBelow(button, f.path);
                  }}
                />
              </li>
            );
          })}
        </ul>
        {fileMenu.at !== null &&
          built !== null &&
          createPortal(
            <RowMenu
              testId="changes.file-menu"
              at={fileMenu.at}
              label={built.label}
              heading={built.heading}
              items={built.items}
              onClose={fileMenu.close}
            />,
            document.body,
          )}
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
