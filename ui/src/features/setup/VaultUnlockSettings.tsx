/** Vault unlock policy settings. The controller saves after password verification; a rejected change stays pending. */
import { useState } from "react";
import type { VaultUnlockMethod } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { useKeyUi } from "./useKeys";
import { PasswordAsk } from "./PasswordAsk";
import { VaultUnlockChoice } from "./VaultUnlockChoice";

/** The same two choices as setup, followed by the existing password form when changing them. */
export function VaultUnlockSettings() {
  const { keys } = useApp();
  const status = useKeyUi((s) => s.status);
  const busy = useKeyUi((s) => s.busy);
  const [pending, setPending] = useState<VaultUnlockMethod | null>(null);
  if (keys === null || status?.vaultSelected !== true) return null;
  const saved = status.vaultKind === "desktop" ? "password" : (status.unlockMethod ?? "system");
  return (
    <div data-setting="vault-unlock-method">
      <VaultUnlockChoice
        value={pending ?? saved}
        disabled={busy}
        systemDisabled={status.vaultKind === "desktop"}
        onChange={(method) => {
          keys.clearProblem();
          setPending(method === saved ? null : method);
        }}
      />
      {pending !== null && (
        <PasswordAsk
          action="Save"
          onCancel={() => {
            keys.clearProblem();
            setPending(null);
          }}
          onSubmit={(password) => {
            void keys.setUnlockMethod(pending, password).then((ok) => {
              if (ok) setPending(null);
            });
          }}
        />
      )}
    </div>
  );
}
