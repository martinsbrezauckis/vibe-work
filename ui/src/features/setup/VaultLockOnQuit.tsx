/**
 * Settings, with the vault controls: the "Lock the vault when Vibe Work quits" switch. Off (the default): the vault
 * keeps its last state, so an unlocked vault opens unlocked at the next start. On: quitting locks it and deletes
 * the key the computer kept.
 * Depends on: AppContext, useKeys, Controls. Called by: KeySettings.
 * Invariants: the shown state is the core's (`lockVaultOnQuit` in the key status); a change goes through the key
 * controller and needs no password.
 */
import { SettingsRow, Switch } from "../../components/Controls";

import { useApp } from "../shell/AppContext";
import { useKeyUi } from "./useKeys";

/** The switch row. */
export function VaultLockOnQuit() {
  const { keys } = useApp();
  const on = useKeyUi((s) => s.status?.lockVaultOnQuit ?? false);
  const busy = useKeyUi((s) => s.busy);
  const problem = useKeyUi((s) =>
    s.problem?.operation === "setLockVaultOnQuit" ? s.problem.text : null,
  );
  if (keys === null) return null;
  return (
    <>
      <SettingsRow data-setting="vault-lock-on-quit">
        <span>
          <label htmlFor="vault-lock-on-quit-switch">Lock the vault when Vibe Work quits</label>
          {/* For screen readers only: the label says what the switch does. */}
          <span className="sr" id="vault-lock-on-quit-description">
            When off, an unlocked vault stays unlocked after a restart: its key is kept in this
            computer&apos;s credential store.
          </span>
        </span>
        <Switch
          data-testid="setup.vault-lock-on-quit"
          id="vault-lock-on-quit-switch"
          aria-describedby="vault-lock-on-quit-description"
          aria-checked={on}
          disabled={busy}
          onClick={() => {
            void keys.setLockOnQuit(!on);
          }}
        />
      </SettingsRow>
      {problem !== null && (
        <p className="key-problem" role="alert">
          {problem}
        </p>
      )}
    </>
  );
}
