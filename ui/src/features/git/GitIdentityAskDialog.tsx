/**
 * The question for the person's Git name and email while the agent's `git init` or `git commit` waits: one line on
 * why it asks and what Save does, the shared author fields, Cancel and Save. Escape and the scrim are Cancel.
 * It appears only when Git has no name or email, so Save only fills Git's empty global fields.
 * Depends on: AppContext (the core), state/gitIdentityAsk, GitIdentityFields, useModalFocus, Controls.
 * Called by: Shell.
 */
import { type ReactElement, useRef } from "react";

import { Button } from "../../components/Controls";
import { useModalFocus } from "../../hooks/useModalFocus";
import { useStore } from "../../state/createStore";
import { validIdentity } from "../../state/gitIdentity";
import { type IdentityAsk, gitIdentityAskFor } from "../../state/gitIdentityAsk";
import { useApp } from "../shell/AppContext";
import { GitIdentityFields } from "./GitIdentityFields";

/** The dialog for `ask`, while its question is open. */
export function IdentityAskDialog({ ask }: { ask: IdentityAsk }): ReactElement | null {
  const state = useStore(ask.store, (s) => s);
  const ref = useRef<HTMLDivElement>(null);
  const open = state.askId !== null;
  const cancel = () => {
    if (!state.busy) void ask.cancel();
  };
  useModalFocus(open, ref, cancel);
  if (!open) return null;
  return (
    <>
      <div data-testid="git.identity-ask-scrim" className="scrim" onClick={cancel} />
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="git-identity-ask-title"
        aria-describedby="git-identity-ask-text"
        data-testid="git.identity-ask-dialog"
        ref={ref}
        tabIndex={-1}
      >
        <header>
          <h2 id="git-identity-ask-title">Set your name and email for Git commits?</h2>
        </header>
        <section>
          <p id="git-identity-ask-text">
            Vibe is waiting to commit. Save adds them to Git’s settings.
          </p>
          <GitIdentityFields identity={state.draft} onChange={ask.edit} disabled={state.busy} />
          {state.error !== null && (
            <p className="key-problem" role="alert">
              {state.error}
            </p>
          )}
        </section>
        <footer className="modal-actions">
          <Button
            data-testid="git.identity-ask-cancel"
            data-dialog-initial-focus="true"
            disabled={state.busy}
            onClick={cancel}
          >
            Cancel
          </Button>
          <Button
            data-testid="git.identity-ask-save"
            variant="primary"
            shape="outlined"
            disabled={state.busy || !validIdentity(state.draft)}
            onClick={() => void ask.save()}
          >
            {state.busy ? "Saving…" : "Save"}
          </Button>
        </footer>
      </div>
    </>
  );
}

/** The app's dialog (nothing without a core). */
export function GitIdentityAskDialog(): ReactElement | null {
  const { core } = useApp();
  return core === null ? null : <IdentityAskDialog ask={gitIdentityAskFor(core)} />;
}
