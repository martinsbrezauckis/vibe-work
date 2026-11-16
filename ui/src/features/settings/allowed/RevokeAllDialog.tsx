/**
 * The bulk revoke confirmation of the Allowed without asking page, in the existing destructive confirmation frame
 * (`sticker:dialog-destructive`: scrim, `.modal`, Cancel first and focused, the danger button last naming the act).
 * Escape, the scrim and Cancel revoke nothing. Renders only; the store holds the dialog and its actions.
 * Depends on: components/Controls, components/Modal, allowed/store. Called by: AllowedPage.
 */
import type { ReactElement } from "react";

import { Button } from "../../../components/Controls";
import { Modal } from "../../../components/Modal";
import type { Allowed, AllowedState } from "./store";

/** The open confirmation, if any. */
export function RevokeAllDialog({
  allowed,
  state,
}: {
  allowed: Allowed;
  state: AllowedState;
}): ReactElement | null {
  const dialog = state.dialog;
  if (dialog === null) return null;
  return (
    <Modal
      onClose={() => {
        allowed.cancelRevokeAll();
      }}
      scrimTestId="settings.allowed-dialog-scrim"
      data-testid="settings.allowed-dialog"
      aria-labelledby="allowed-dialog-title"
      data-leaf={dialog.leaf}
    >
      <header>
        <h2 id="allowed-dialog-title">{dialog.title}</h2>
      </header>
      <section>
        <p>{dialog.body}</p>
        <ul className="mono">
          {dialog.list.map((line, i) => (
            <li key={i}>{line}</li>
          ))}
        </ul>
        {dialog.more !== null && <p>{dialog.more}</p>}
      </section>
      <footer className="modal-actions">
        <Button
          data-testid="settings.allowed-dialog-cancel"
          data-dialog-initial-focus="true"
          disabled={state.dialogBusy}
          onClick={() => {
            allowed.cancelRevokeAll();
          }}
        >
          Cancel
        </Button>
        <Button
          data-testid="settings.allowed-dialog-confirm"
          shape="outlined"
          variant="danger"
          data-action="confirm"
          disabled={state.dialogBusy}
          onClick={() => void allowed.confirmRevokeAll()}
        >
          {state.dialogBusy ? "Revoking…" : dialog.confirm}
        </Button>
      </footer>
    </Modal>
  );
}
