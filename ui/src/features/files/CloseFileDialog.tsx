/**
 * A close decision for an unsaved file. Uses the app dialog frame; the buttons run `decideClose`, the same
 * function as the `ui/editors` route's `closeDialog`. Called by file tabs and the close shortcut.
 */
import { createPortal } from "react-dom";
import { useRef, useState } from "react";
import { Button } from "../../components/Controls";
import { useModalFocus } from "../../hooks/useModalFocus";
import { useStore } from "../../state/createStore";
import type { Files } from "../../state/files";
import { useApp } from "../shell/AppContext";
import { decideClose, type CloseChoice } from "./closeDecision";
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
  const root = useRef<HTMLDivElement>(null);
  const [busy, setBusy] = useState(false);
  // A refused Save and close (stale or failed) is kept on the workspace, so a Debug API decision shows here too.
  const error = useStore(files.workspace.store, (s) => s[sessionId]?.closeProblem ?? null);
  useModalFocus(true, root, onClose);
  const shown = files.workspace.of(sessionId).documents[file]?.shown;
  const decide = (choice: CloseChoice) => {
    setBusy(true);
    void decideClose(app, files, sessionId, file, choice, "ui").finally(() => {
      setBusy(false);
    });
  };
  return createPortal(
    <>
      <div className="scrim" data-testid="files.close-file-scrim" onClick={onClose} />
      <div
        data-testid="files.close-file-dialog"
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-label={`Save changes to ${shown?.display ?? "the file"}?`}
        ref={root}
        tabIndex={-1}
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
      </div>
    </>,
    document.body,
  );
}
