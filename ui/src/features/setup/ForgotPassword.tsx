/**
 * "Forgot the password?" at every place that asks for the vault password. Nobody can recover a vault password, so
 * the dialog says what is possible: the app's own vault can be set aside (its folder is moved, nothing is deleted)
 * and a new vault started, where the person pastes their Mistral key again; what the old vault holds stays locked in
 * it. A shared ShellX or Vault-app vault is never moved; the dialog says so and offers nothing to do.
 * Depends on: useKeys (status, busy, problem), Modal, keyText. Called by: UnlockStep, settings/MistralKeys.
 * Invariants: nothing moves until the person presses the set-aside button in the dialog; a refusal from the core is
 * shown in the dialog with its plain reason (nothing moved then).
 */
import { type ReactElement, useState } from "react";

import { Button } from "../../components/Controls";
import { Modal } from "../../components/Modal";
import { vaultName } from "../../state/keyText";
import { useApp } from "../shell/AppContext";
import { Problem } from "./fields";
import { useKeyUi } from "./useKeys";

/** The folder that will hold the old vault: the private vault's own container (the set-aside folder's parent). */
function containerOf(path: string | null): string | null {
  if (path === null) return null;
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return cut > 0 ? path.slice(0, cut) : path;
}

/** The link and, while open, its dialog. */
export function ForgotPassword(): ReactElement | null {
  const { keys } = useApp();
  const status = useKeyUi((s) => s.status);
  const busy = useKeyUi((s) => s.busy);
  const problem = useKeyUi((s) => s.problem);
  const [open, setOpen] = useState(false);
  if (keys === null || status === null || status.vaultState !== "locked") return null;
  const own = status.vaultKind === "vibeWork";
  const folder = containerOf(status.vaultPath);
  const close = () => {
    keys.clearProblem();
    setOpen(false);
  };
  const refused = problem?.operation === "setAsideVault" ? problem.text : null;
  return (
    <>
      <Button
        data-testid="setup.forgot-password"
        type="button"
        appearance="link"
        onClick={() => {
          keys.clearProblem();
          setOpen(true);
        }}
      >
        Forgot the password?
      </Button>
      {open && (
        <Modal
          onClose={close}
          scrimCloses={!busy}
          className="modal forgot-password-dialog"
          aria-labelledby="forgot-password-title"
          data-testid="setup.forgot-password-dialog"
        >
          <header>
            <h2 id="forgot-password-title">Forgot the vault password?</h2>
          </header>
          <section>
            {own ? (
              <>
                <p>
                  Nobody can recover a vault password, not even Vibe Work. You can set this vault
                  aside and start a new one, then paste your Mistral key again.
                </p>
                <p>
                  Setting it aside moves the vault folder; nothing is deleted. The logins, passwords
                  and secrets saved in it stay locked inside it and do not come to the new vault.
                  Only the old password opens it again.
                </p>
                {folder !== null && (
                  <p>
                    It will be kept here: <code className="path">{folder}</code>.
                  </p>
                )}
              </>
            ) : (
              <p>
                Nobody can recover a vault password, not even Vibe Work. Your{" "}
                {vaultName(status.vaultKind)} is shared with other apps, so Vibe Work does not move
                or change it. If you remember the password later, unlock it here.
              </p>
            )}
            <Problem text={refused} />
          </section>
          <footer className="modal-actions">
            <Button
              data-testid="setup.forgot-password-cancel"
              type="button"
              data-dialog-initial-focus="true"
              onClick={close}
              disabled={busy}
            >
              {own ? "Cancel" : "Close"}
            </Button>
            {own && (
              <Button
                data-testid="setup.forgot-password-set-aside"
                type="button"
                variant="primary"
                disabled={busy}
                onClick={() => {
                  void keys.setAsideVault().then((ok) => {
                    if (ok) setOpen(false);
                  });
                }}
              >
                Set it aside and start a new vault
              </Button>
            )}
          </footer>
        </Modal>
      )}
    </>
  );
}
