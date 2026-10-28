/**
 * New chat's quiet Send hint and the status of its open workspace, with Close all and its existing confirmation.
 * Depends on AppContext, the draft store, shared controls and modal focus. Called by NewChatPanel.
 * Terminals ask before closing; the composer's text and folder stay.
 */
import { type ReactElement, useRef, useState } from "react";

import { Button, LinkButton } from "../../components/Controls";
import { useModalFocus } from "../../hooks/useModalFocus";
import { useStore } from "../../state/createStore";
import { requestComposerFocus } from "../../state/composerFocus";
import { NEW_CHAT } from "../../state/newChatKey";
import type { NewChatDraft, NewChatOpen } from "../../state/newChatDraft";
import { useApp } from "../shell/AppContext";
import { closeAllBody, openSummary } from "./newChatText";

/** The Close all question. */
function CloseAllDialog({
  open,
  files,
  onCancel,
  onConfirm,
  busy,
}: {
  open: NewChatOpen;
  files: number;
  onCancel: () => void;
  onConfirm: () => void;
  busy: boolean;
}): ReactElement {
  const ref = useRef<HTMLDivElement>(null);
  useModalFocus(true, ref, onCancel);
  return (
    <>
      <div data-testid="sessions.new-chat-close-all-scrim" className="scrim" onClick={onCancel} />
      <div
        data-testid="sessions.new-chat-close-all-dialog"
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="new-chat-close-all-title"
        ref={ref}
        tabIndex={-1}
      >
        <header>
          <h2 id="new-chat-close-all-title">Close everything open in this new chat?</h2>
        </header>
        <section>
          <p>{closeAllBody(open, files)}</p>
        </section>
        <footer className="modal-actions">
          <Button
            data-testid="sessions.new-chat-close-all-cancel"
            data-dialog-initial-focus="true"
            disabled={busy}
            onClick={onCancel}
          >
            Cancel
          </Button>
          <Button
            data-testid="sessions.new-chat-close-all-confirm"
            shape="outlined"
            variant="danger"
            disabled={busy}
            onClick={onConfirm}
          >
            Close all
          </Button>
        </footer>
      </div>
    </>
  );
}

/** The row, the line and the status of the New chat screen. */
export function NewChatWorkspace({ draft }: { draft: NewChatDraft }): ReactElement {
  const { toasts } = useApp();
  const open = useStore(draft.store, (s) => s.open);
  const files = useStore(draft.store, (s) => s.files);
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const summary = openSummary(open, files);
  const close = async () => {
    setBusy(true);
    const done = await draft.closeAll();
    setBusy(false);
    setAsking(false);
    if (done && open.terminals === 0) toasts.feedback("Closed. Your text and folder stay.");
    requestComposerFocus(NEW_CHAT);
  };
  return (
    <div className="new-chat-work">
      {summary !== null && (
        <p className="key-line new-chat-work-status" role="status">
          <span>{summary}</span>{" "}
          <LinkButton
            data-testid="sessions.new-chat-close-all"
            onClick={() => {
              // Terminals stop programs, so that asks first; pages and files reopen, so they close at once.
              if (open.terminals > 0) setAsking(true);
              else void close();
            }}
          >
            Close all…
          </LinkButton>
        </p>
      )}
      {asking && (
        <CloseAllDialog
          open={open}
          files={files}
          busy={busy}
          onCancel={() => {
            setAsking(false);
          }}
          onConfirm={() => void close()}
        />
      )}
    </div>
  );
}
