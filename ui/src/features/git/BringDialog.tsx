/**
 * "Bring into <base>": the dry run's answer for a session copy without a forge. A
 * fast-forward or a merge commit says which folder changes; conflicts list the files, change nothing, and offer
 * "Ask Vibe to update the copy from <base>" (a prepared message in the composer, never sent).
 * Depends on: state/git, GitModal. Called by: GitDialogs.
 * Invariants: the press goes to the core's `bringIn` with the commits the dry run saw; the core asks the native
 * confirmation and refuses when either branch moved.
 */
import type { ReactElement } from "react";

import type { BringDialog as Dialog, Git } from "../../state/git";
import { GitModal } from "./GitModal";

/** The open Bring dialog. */
export function BringDialog({
  git,
  dialog,
  busy,
  error,
}: {
  git: Git;
  dialog: Dialog;
  busy: boolean;
  error: string | null;
}): ReactElement {
  const { plan, sentence } = dialog.plan;
  const conflicts = plan.kind === "conflicts";
  const blocked = plan.kind === "upToDate" || plan.checkedOutElsewhere;
  return (
    <GitModal
      title={`Bring ${plan.branch} into ${plan.base}`}
      kind="bring"
      onClose={() => {
        git.close();
      }}
      actions={
        <>
          <button
            type="button"
            className="tb-btn"
            onClick={() => {
              git.close();
            }}
          >
            {conflicts || blocked ? "Close" : "Cancel"}
          </button>
          {conflicts ? (
            <button
              type="button"
              className="btn-primary"
              disabled={busy}
              data-action="bring-ask-vibe"
              onClick={() => {
                void git.prepare(dialog.sessionId, "updateFromBase").then(() => {
                  git.close();
                });
              }}
            >
              Ask Vibe to update the copy from {plan.base}
            </button>
          ) : (
            !blocked && (
              <button
                type="button"
                className="btn-primary"
                disabled={busy}
                data-action="bring-in"
                onClick={() => void git.bringIn()}
              >
                Bring into {plan.base}
              </button>
            )
          )}
        </>
      }
    >
      <p data-git-field="sentence">{sentence}</p>
      {conflicts && (
        <ul className="git-files mono">
          {plan.conflicts.slice(0, 30).map((p) => (
            <li key={p}>{p}</li>
          ))}
        </ul>
      )}
      {error !== null && (
        <p className="git-warn" role="alert">
          {error}
        </p>
      )}
    </GitModal>
  );
}
