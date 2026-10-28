/**
 * The bulk revoke confirmation of the Allowed without asking page, in the existing destructive confirmation frame
 * (`sticker:dialog-destructive`: scrim, `.modal`, Cancel first and focused, the danger button last naming the act).
 * Escape, the scrim and Cancel revoke nothing. Renders only; the store holds the dialog and its actions.
 * Depends on: components/Controls, hooks/useModalFocus, allowed/store. Called by: AllowedPage.
 */
import { type ReactElement, useRef } from "react";

import { Button } from "../../../components/Controls";
import { useModalFocus } from "../../../hooks/useModalFocus";
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
  const ref = useRef<HTMLDivElement>(null);
  useModalFocus(dialog !== null, ref, () => {
    allowed.cancelRevokeAll();
  });
  if (dialog === null) return null;
  return (
    <>
      <div
        data-testid="settings.allowed-dialog-scrim"
        className="scrim"
        onClick={() => {
          allowed.cancelRevokeAll();
        }}
      />
      <div
        data-testid="settings.allowed-dialog"
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="allowed-dialog-title"
        ref={ref}
        tabIndex={-1}
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
      </div>
    </>
  );
}
