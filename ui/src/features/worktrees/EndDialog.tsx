/**
 * The end-of-session choice for a session's copy: after archive or delete (or from the startup
 * list), "Keep the copy" or "Remove it", with what removing discards shown first: every item with unsaved
 * work (changed, new, ignored but not build output, nested repositories; scrollable), the build output count,
 * commits no other branch holds (they stay on the kept branch), stashes made on it, and what happens to the
 * branch. A copy that cannot be removed here says why and offers Keep only. Every origin gets the full lists, including Debug API requests.
 * Depends on: state/worktrees (store, keep, remove, dismiss), model.ts, hooks/useModalFocus.
 * Called by: shell/Content.
 * Invariants: nothing is removed without a click on the remove button, whose label says how many items it
 * discards; the native dialog asks again with the names (the core). Names are text, never markup.
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
  const removeLabel =
    files === 0
      ? "Remove it"
      : `Remove it and discard ${String(files)} unsaved item${files === 1 ? "" : "s"}`;
  const title = end.after === "startup" ? "Review this copy" : "Keep this session's copy?";
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
          <button
            data-testid="worktrees.end-dialog-keep"
            type="button"
            className="tb-btn"
            data-dialog-initial-focus="true"
            disabled={end.busy}
            onClick={() => void worktrees.keep()}
          >
            Keep the copy
          </button>
          <Button
            data-testid="worktrees.end-dialog-remove"
            type="button"
            small
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
