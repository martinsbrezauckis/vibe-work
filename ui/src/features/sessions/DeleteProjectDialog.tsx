/**
 * The window's confirmation before a project is deleted: names the project and how many sessions move
 * to "Not in a project", and says that no session is deleted (the core only re-files them). The Debug API never sees
 * this dialog: its delete asks the native one in the core.
 * Depends on: hooks/useModalFocus, useGroups. Called by: Shell (open while `groups.deleting` names a project).
 * Mounted outside the session column so the Projects page can ask while that column is hidden.
 * Invariants: Cancel has the initial focus (Enter does not delete by accident); Escape and the scrim cancel.
 */
import { Button } from "../../components/Controls";
import { useRef } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";
import { useGroupActions, useGroups } from "./useGroups";

/** The dialog for the project in `groups.deleting`, if any. */
export function DeleteProjectDialog() {
  const groups = useGroupActions();
  const project = useGroups((g) =>
    g.deleting === null ? null : (g.data?.projects.find((p) => p.id === g.deleting) ?? null),
  );
  const dialog = useRef<HTMLDivElement>(null);
  const close = () => {
    groups?.askDelete(null);
  };
  useModalFocus(project !== null, dialog, close);
  if (project === null || groups === null) return null;
  const n = project.sessions;
  return (
    <>
      <div data-testid="sessions.delete-project-dialog-scrim" className="scrim" onClick={close} />
      <div
        data-testid="sessions.delete-project-dialog-modal"
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="delete-project-title"
        ref={dialog}
        tabIndex={-1}
      >
        <header>
          <h2 id="delete-project-title">Delete the project &ldquo;{project.name}&rdquo;?</h2>
        </header>
        <section>
          <p data-testid="delete-project-text">
            {n === 0
              ? "It holds no sessions."
              : `Its ${String(n)} session${n === 1 ? "" : "s"} move${n === 1 ? "s" : ""} to “Not in a project”.`}{" "}
            No session is deleted.
          </p>
        </section>
        <footer className="modal-actions">
          <Button
            data-testid="sessions.delete-project-dialog-tb-btn"
            type="button"

            data-dialog-initial-focus="true"
            onClick={close}
          >
            Cancel
          </Button>
          <Button
            data-testid="sessions.delete-project-dialog-close"
            type="button"
            shape="outlined"
            variant="danger"
            onClick={() => {
              close();
              void groups.remove(project.id);
            }}
          >
            Delete project
          </Button>
        </footer>
      </div>
    </>
  );
}
