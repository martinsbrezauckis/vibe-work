/**
 * The frame of the git dialogs: scrim, a labelled modal dialog with focus kept inside, Escape or a
 * click on the scrim closes it. The dialogs put their fields in `children` and their buttons in `actions`.
 * Depends on: components/Modal. Called by: CommitDialog, PrDialog, BringDialog.
 */
import { type ReactElement, type ReactNode } from "react";

import { Modal } from "../../components/Modal";

/** One git dialog. */
export function GitModal(props: {
  title: string;
  kind: string;
  onClose: () => void;
  children: ReactNode;
  actions: ReactNode;
}): ReactElement {
  const { title, kind, onClose, children, actions } = props;
  return (
    <Modal
      onClose={onClose}
      scrimTestId="git.git-modal-scrim"
      data-testid="git.git-modal-modal"
      className="modal git-modal"
      aria-label={title}
      data-git-dialog={kind}
    >
      <header>
        <h2>{title}</h2>
      </header>
      <section>{children}</section>
      <footer className="modal-actions">{actions}</footer>
    </Modal>
  );
}
