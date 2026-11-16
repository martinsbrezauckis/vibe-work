/**
 * A close decision for an unsaved file. Uses the app dialog frame; the buttons run `decideClose`, the same
 * function as the `ui/editors` route's `closeDialog`. Called by file tabs and the close shortcut.
 */
import { createPortal } from "react-dom";
import { useState } from "react";
import { Button } from "../../components/Controls";
import { Modal } from "../../components/Modal";
import { useStore } from "../../state/createStore";
import type { Files } from "../../state/files";
import { useApp } from "../shell/AppContext";
import { decideClose, type CloseChoice } from "./closeDecision";
/**
 * The "Save changes to <file>?" dialog with Cancel, Discard and Save; shows the reason when a save and close was
 * refused.
 */
export function CloseFileDialog({
  files,
  sessionId,
  file,
  onClose,
}: {
  files: Files;
  sessionId: string;
  file: string;
  onClose: () => void;
}) {
  const app = useApp();
  const [busy, setBusy] = useState(false);
  // A refused Save and close (stale or failed) is kept on the workspace, so a Debug API decision shows here too.
  const error = useStore(files.workspace.store, (s) => s[sessionId]?.closeProblem ?? null);
  const shown = files.workspace.of(sessionId).documents[file]?.shown;
  const decide = (choice: CloseChoice) => {
    setBusy(true);
    void decideClose(app, files, sessionId, file, choice, "ui").finally(() => {
      setBusy(false);
    });
  };
  return createPortal(
    <Modal
      onClose={onClose}
      scrimTestId="files.close-file-scrim"
      data-testid="files.close-file-dialog"
      aria-label={`Save changes to ${shown?.display ?? "the file"}?`}
    >
      <header>
        <h2>Save changes to {shown?.display ?? "the file"}?</h2>
      </header>
      {error && (
        <section>
          <p className="key-problem" role="alert">
            {error}
          </p>
        </section>
      )}
      <footer className="modal-actions">
        <Button
          data-testid="files.close-file-cancel"
          data-dialog-initial-focus="true"
          disabled={busy}
          onClick={onClose}
        >
          Cancel
        </Button>
        <Button
          data-testid="files.close-file-discard"
          variant="danger"
          disabled={busy}
          onClick={() => {
            decide("discard");
          }}
        >
          Discard
        </Button>
        <Button
          data-testid="files.close-file-save"
          variant="primary"
          disabled={busy}
          onClick={() => {
            decide("save");
          }}
        >
          {busy ? "Saving…" : "Save and close"}
        </Button>
      </footer>
    </Modal>,
    document.body,
  );
}
