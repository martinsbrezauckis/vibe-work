/**
 * The rewind dialog: shows the message being rewound to and the files Vibe will restore
 * (`session/rewind/read`), then rewinds in place or into a new session. Nothing is rewound before a
 * button here is pressed.
 * Depends on: the composer store hooks, AppContext, Modal. Called by: Composer.
 * Invariants: render only; the file list is Vibe's answer, never a guess; while loading or busy the
 * buttons are disabled; Escape or the scrim closes without rewinding.
 */
import { Button } from "../../components/Controls";
import type { ReactElement } from "react";

import { Modal } from "../../components/Modal";
import { useApp } from "../shell/AppContext";
import { useComposerState } from "./hooks";

/** The dialog, when one is open. */
export function RewindDialog(): ReactElement | null {
  const { composer } = useApp();
  const dialog = useComposerState((s) => s.rewind);
  const close = () => composer.actions?.closeRewind();
  if (dialog === null || composer.actions === null) return null;
  const actions = composer.actions;
  const blocked = dialog.loading || dialog.busy;
  return (
    <Modal
      onClose={close}
      scrimTestId="composer.rewind-dialog-scrim"
      data-testid="composer.rewind-dialog-modal"
      className="modal rewind-dialog"
      aria-labelledby="rewind-title"
    >
      <header>
        <h2 id="rewind-title">Rewind to this message?</h2>
      </header>
      <section>
        <p className="rewind-msg">{dialog.text || "(this message)"}</p>
        <p className="d">
          Everything after it leaves the chat. Your message goes back into the composer so you can
          change it and send it again.
        </p>
      </section>
      <section aria-live="polite">
        <h3>Files that will be restored</h3>
        {dialog.loading ? (
          <p className="d">Asking Vibe which files changed…</p>
        ) : dialog.error !== null ? (
          <p className="key-problem" role="alert">
            {dialog.error}
          </p>
        ) : dialog.paths.length === 0 ? (
          <p className="d">No files changed after this message.</p>
        ) : (
          <ul className="rewind-files">
            {dialog.paths.map((path) => (
              <li key={path} className="mono">
                {path}
              </li>
            ))}
          </ul>
        )}
      </section>
      <section className="rewind-acts">
        <Button
          data-testid="composer.rewind-dialog-mini"
          type="button"
          appearance="compact"
          title="Cancel rewind (Escape)"
          aria-label="Cancel rewind (Escape)"
          onClick={close}
        >
          Cancel
        </Button>
        <Button
          data-testid="composer.rewind-dialog-confirm-rewind"
          type="button"
          shape="outlined"
          variant="danger"
          title="Rewind into a new chat and restore files"
          aria-label="Rewind into a new chat and restore files"
          disabled={blocked}
          onClick={() => {
            void actions.confirmRewind(true);
          }}
        >
          Rewind into a new chat
        </Button>
        <Button
          data-testid="composer.rewind-dialog-confirm-rewind-2"
          type="button"
          shape="outlined"
          variant="danger"
          title="Rewind here and restore files"
          aria-label="Rewind here and restore files"
          data-dialog-initial-focus="true"
          disabled={blocked}
          onClick={() => {
            void actions.confirmRewind(false);
          }}
        >
          Rewind here
        </Button>
      </section>
    </Modal>
  );
}
