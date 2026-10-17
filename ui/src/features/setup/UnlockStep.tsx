/**
 * Unlocks the selected vault with one password. During first run `choice` is the user's explicit
 * choice; later the persisted core selection supplies the kind and cannot be switched here.
 * Depends on: AppContext, useKeys, fields. Called by: SetupScreen, VaultStartStep.
 * Invariants: the password is cleared on submit; remembered unlock is only offered for vaults
 * that support it (ShellX and Vibe Work), never the Vault app.
 */
import { useState } from "react";

import type { VaultChoice } from "../../protocol/core";
import { vaultName } from "../../state/keyText";
import { useApp } from "../shell/AppContext";
import { Problem, RememberChoice, SecretField } from "./fields";
import { useKeyUi } from "./useKeys";

/** The unlock form. `choice` is supplied only for an unselected first-run vault. */
export function UnlockStep({ choice }: { choice?: VaultChoice }) {
  const { keys } = useApp();
  const status = useKeyUi((s) => s.status);
  const busy = useKeyUi((s) => s.busy);
  const problem = useKeyUi((s) => s.problem);
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(true);
  const kind = choice ?? status?.vaultKind ?? null;
  const canRemember = kind === "shellx" || kind === "vibeWork";

  return (
    <form
      className="setup-step"
      onSubmit={(e) => {
        e.preventDefault();
        if (keys === null || password === "") return;
        const pass = password;
        setPassword("");
        void keys.unlock(pass, choice, canRemember ? remember : undefined);
      }}
    >
      <p>Type the password to open your {vaultName(kind)} and continue.</p>
      <SecretField
        testId="setup.unlock-step-secretfield"
        label="Vault password"
        value={password}
        onChange={setPassword}
        autoFocus
        invalid={problem?.reason === "wrongPassphrase"}
      />
      {canRemember && (
        <RememberChoice
          testId="setup.unlock-step-rememberchoice"
          checked={remember}
          onChange={setRemember}
        />
      )}
      <Problem text={problem?.text ?? null} />
      <div className="key-actions">
        <button
          data-testid="setup.unlock-step-btn-primary"
          type="submit"
          className="btn-primary"
          disabled={busy || password === ""}
        >
          {busy ? "Unlocking..." : "Unlock"}
        </button>
        <button
          data-testid="setup.unlock-step-dismiss"
          type="button"
          className="tb-btn"
          onClick={() => keys?.dismiss()}
        >
          Not now
        </button>
      </div>
    </form>
  );
}
