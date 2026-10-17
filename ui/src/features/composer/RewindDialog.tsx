/**
 * The rewind dialog: shows the message being rewound to and the files Vibe will restore
 * (`session/rewind/read`), then rewinds in place or into a new session. Nothing is rewound before a
 * button here is pressed.
 * Depends on: the composer store hooks, AppContext, useModalFocus. Called by: Composer.
 * Invariants: render only; the file list is Vibe's answer, never a guess; while loading or busy the
 * buttons are disabled; Escape or the scrim closes without rewinding.
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useRef } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";
import { useApp } from "../shell/AppContext";
import { useComposerState } from "./hooks";

/** The dialog, when one is open. */
export function RewindDialog(): ReactElement | null {
  const { composer } = useApp();
  const dialog = useComposerState((s) => s.rewind);
  const ref = useRef<HTMLDivElement>(null);
  const close = () => composer.actions?.closeRewind();
  useModalFocus(dialog !== null, ref, close);
  if (dialog === null || composer.actions === null) return null;
  const actions = composer.actions;
  const blocked = dialog.loading || dialog.busy;
  return (
    <>
      <div data-testid="composer.rewind-dialog-scrim" className="scrim" onClick={close} />
      <div
        data-testid="composer.rewind-dialog-modal"
        ref={ref}
        className="modal rewind-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="rewind-title"
        tabIndex={-1}
      >
        <header>
          <h2 id="rewind-title">Rewind to this message?</h2>
        </header>
        <section>
          <p className="rewind-msg">{dialog.text || "(this message)"}</p>
          <p className="d">
            Everything after it leaves the session. Your message goes back into the composer so you
            can change it and send it again.
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
          <button
            data-testid="composer.rewind-dialog-mini"
            type="button"
            className="mini"
            title="Cancel rewind (Escape)"
            aria-label="Cancel rewind (Escape)"
            onClick={close}
          >
            Cancel
          </button>
          <Button
            data-testid="composer.rewind-dialog-confirm-rewind"
            type="button"
            small
            variant="danger"
            title="Rewind into a new session and restore files"
            aria-label="Rewind into a new session and restore files"
            disabled={blocked}
            onClick={() => {
              void actions.confirmRewind(true);
            }}
          >
            Rewind into a new session
          </Button>
          <Button
            data-testid="composer.rewind-dialog-confirm-rewind-2"
            type="button"
            small
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
      </div>
    </>
  );
}
