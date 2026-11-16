/**
 * The window's confirmation before an MCP server is removed: names the server and says what changes
 * (Vibe stops using its tools; it can be added again). Removing deletes the server's entry
 * from Vibe's config (and a key server's link to its vault key), so the window asks first. The Debug API's
 * `removeServer` does not see this dialog: it acts as the user.
 * Depends on: components/Modal, hooks. Called by: ConnectorsPage (open while `connectors.removing`
 * names a server).
 * Invariants: Cancel has the initial focus (Enter does not remove by accident); Escape and the scrim
 * cancel; the red Remove comes last.
 */
import { Button } from "../../components/Controls";
import type { ReactElement } from "react";

import { Modal } from "../../components/Modal";
import { useConnectorActions, useConnectors } from "./hooks";

/** The dialog for the server in `connectors.removing`, if any. */
export function RemoveServerDialog(): ReactElement | null {
  const actions = useConnectorActions();
  const server = useConnectors((s) =>
    s.removing === null ? null : (s.view?.servers.find((v) => v.name === s.removing) ?? null),
  );
  const close = () => {
    actions?.askRemoveServer(null);
  };
  if (server === null || actions === null) return null;
  return (
    <Modal
      onClose={close}
      scrimTestId="connectors.remove-server-scrim"
      data-testid="connectors.remove-server-dialog"
      aria-labelledby="remove-server-title"
    >
      <header>
        <h2 id="remove-server-title">Remove {server.displayName}?</h2>
      </header>
      <section>
        <p>Vibe stops using its tools.</p>
      </section>
      <footer className="modal-actions">
        <Button
          data-testid="connectors.remove-server-cancel"
          type="button"
          data-dialog-initial-focus="true"
          onClick={close}
        >
          Cancel
        </Button>
        <Button
          data-testid="connectors.remove-server-confirm"
          type="button"
          shape="outlined"
          variant="danger"
          onClick={() => {
            close();
            void actions.removeServer(server.name);
          }}
        >
          Remove
        </Button>
      </footer>
    </Modal>
  );
}
