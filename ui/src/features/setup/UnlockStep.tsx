/**
 * Unlocks the selected vault with one password. During first run `choice` is the user's explicit
 * choice; later the persisted core selection supplies the kind and cannot be switched here.
 * Depends on: AppContext, useKeys, fields. Called by: SetupScreen, VaultStartStep.
 * Invariants: the password is cleared on submit. First selection offers the unlock policy; later unlocks keep it.
 */
import { Button } from "../../components/Controls";
import { useState } from "react";

import { VaultUnlockChoice } from "./VaultUnlockChoice";
import type { VaultChoice, VaultUnlockMethod } from "../../protocol/core";
import { vaultName } from "../../state/keyText";
import { useApp } from "../shell/AppContext";
import { Problem, SecretField } from "./fields";
import { useKeyUi } from "./useKeys";

/** The unlock form. `choice` is supplied only for an unselected first-run vault. */
export function UnlockStep({ choice }: { choice?: VaultChoice }) {
  const { keys } = useApp();
  const status = useKeyUi((s) => s.status);
  const busy = useKeyUi((s) => s.busy);
  const problem = useKeyUi((s) => s.problem);
  const [password, setPassword] = useState("");
  const [unlockMethod, setUnlockMethod] = useState<VaultUnlockMethod>("system");
  const kind = choice ?? status?.vaultKind ?? null;

  return (
    <form
      className="setup-step"
      onSubmit={(e) => {
        e.preventDefault();
        if (keys === null || password === "") return;
        const pass = password;
        setPassword("");
        void keys.unlock(
          pass,
          choice,
          choice === undefined ? undefined : kind === "desktop" ? "password" : unlockMethod,
        );
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
      {choice !== undefined && (
        <VaultUnlockChoice
          value={kind === "desktop" ? "password" : unlockMethod}
          onChange={setUnlockMethod}
          disabled={busy}
          systemDisabled={kind === "desktop"}
        />
      )}
      <Problem text={problem?.text ?? null} />
      <div className="key-actions">
        <Button
          data-testid="setup.unlock-step-btn-primary"
          type="submit"
          appearance="dialog"
          disabled={busy || password === ""}
        >
          {busy ? "Unlocking…" : "Unlock"}
        </Button>
        <Button
          data-testid="setup.unlock-step-dismiss"
          type="button"

          onClick={() => keys?.dismiss()}
        >
          Not now
        </Button>
      </div>
    </form>
  );
}
