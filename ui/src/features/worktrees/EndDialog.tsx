/**
 * Explicit copy removal: shows unsaved work, branch outcome and blockers before the native confirmation.
 * Depends on state/worktrees, model and modal focus; mounted by shell/Shell, outside the chat column, because the
 * Changes tab asks it from the side panel, which a maximized panel leaves as the only visible area. Cancel changes nothing.
 */
import { PathLine } from "../../components/PathLine";
import { Button } from "../../components/Controls";
import { type ReactElement, useRef } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";
import { useStore } from "../../state/createStore";
import { NO_WORKTREES, type Worktrees } from "../../state/worktrees";
import { useApp } from "../shell/AppContext";
import { branchLine, buildLine, endSummary, kindText } from "./model";

function Body({ worktrees }: { worktrees: Worktrees }): ReactElement | null {
  const end = useStore(worktrees.store, (s) => s.end);
  if (end === null) return null;
  const inspection = end.inspection;
  if (inspection === null) return <p role="status">Checking the copy…</p>;
  const build = buildLine(inspection);
  return (
    <>
      {inspection.path !== null && <PathLine className="wt-path" path={inspection.path} />}
      {endSummary(inspection).map((line) => (
        <p key={line}>{line}</p>
      ))}
      {inspection.files.length > 0 && (
        <ul className="mono wt-files" data-worktree="files">
          {inspection.files.map((f) => (
            <li key={f.path}>
              {f.path} <span className="wt-status">({kindText(f.kind)})</span>
            </li>
          ))}
        </ul>
      )}
      {build !== "" && <p>{build}</p>}
      {inspection.unmergedTotal > 0 && (
        <p>
          {String(inspection.unmergedTotal)} commit{inspection.unmergedTotal === 1 ? "" : "s"} on it{" "}
          are in no other branch; they stay on the branch:
        </p>
      )}
      {inspection.unmerged.length > 0 && (
        <ul className="mono wt-files" data-worktree="commits">
          {inspection.unmerged.map((c) => (
            <li key={c}>{c}</li>
          ))}
        </ul>
      )}
      {inspection.stashCount > 0 && (
        <p>
          {String(inspection.stashCount)} stash entr
          {inspection.stashCount === 1 ? "y was" : "ies were"} made on its branch; they stay in the
          repository&apos;s stash list.
        </p>
      )}
      <p>{branchLine(inspection)}</p>
      {inspection.blocked !== null && (
        <p className="wt-blocked" role="note">
          {inspection.blocked}
        </p>
      )}
    </>
  );
}

/** The dialog, when a choice is open. */
export function EndDialog(): ReactElement | null {
  const { worktrees } = useApp();
  const end = useStore(worktrees?.store ?? NO_WORKTREES, (s) => s.end);
  const dialog = useRef<HTMLDivElement>(null);
  useModalFocus(end !== null, dialog, () => {
    worktrees?.dismiss();
  });
  if (worktrees === null || end === null) return null;
  const inspection = end.inspection;
  const files = inspection?.fileCount ?? 0;
  const canRemove = inspection !== null && inspection.blocked === null && !end.busy;
  const verb = "Throw it away";
  const removeLabel =
    files === 0
      ? verb
      : `${verb} and discard ${String(files)} unsaved item${files === 1 ? "" : "s"}`;
  const title = "Throw away this copy?";
  return (
    <>
      <div className="scrim" />
      <div
        data-testid="worktrees.end-dialog-modal"
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        ref={dialog}
        tabIndex={-1}
        data-worktree="end"
      >
        <header>
          <h2>{title}</h2>
        </header>
        <section>
          <Body worktrees={worktrees} />
        </section>
        <footer className="modal-actions">
          <Button
            data-testid="worktrees.end-dialog-cancel"
            type="button"

            data-dialog-initial-focus="true"
            disabled={end.busy}
            onClick={() => {
              worktrees.dismiss();
            }}
          >
            Cancel
          </Button>
          <Button
            data-testid="worktrees.end-dialog-remove"
            type="button"
            shape="outlined"
            variant="danger"
            data-action="remove"
            disabled={!canRemove}
            onClick={() => void worktrees.remove()}
          >
            {removeLabel}
          </Button>
        </footer>
      </div>
    </>
  );
}
