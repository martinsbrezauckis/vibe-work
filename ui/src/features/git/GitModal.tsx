/**
 * The frame of the git dialogs: scrim, a labelled modal dialog with focus kept inside, Escape or a
 * click on the scrim closes it. The dialogs put their fields in `children` and their buttons in `actions`.
 * Depends on: hooks/useModalFocus. Called by: CommitDialog, PrDialog, BringDialog.
 */
import { type ReactElement, type ReactNode, useRef } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";

/** One git dialog. */
export function GitModal(props: {
  title: string;
  kind: string;
  onClose: () => void;
  children: ReactNode;
  actions: ReactNode;
}): ReactElement {
  const { title, kind, onClose, children, actions } = props;
  const dialog = useRef<HTMLDivElement>(null);
  useModalFocus(true, dialog, onClose);
  return (
    <>
      <div data-testid="git.git-modal-scrim" className="scrim" onClick={onClose} />
      <div
        data-testid="git.git-modal-modal"
        className="modal git-modal"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        ref={dialog}
        tabIndex={-1}
        data-git-dialog={kind}
      >
        <header>
          <h2>{title}</h2>
        </header>
        <section>{children}</section>
        <footer className="modal-actions">{actions}</footer>
      </div>
    </>
  );
}
