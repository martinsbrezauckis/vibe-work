/**
 * Settings, "Mistral key": what holds the key and whether it is open, with shared test/replace, vault lock
 * and the "Lock the vault when Vibe Work quits" switch. Same words as the setup screen. Hidden when this run keeps
 * its key in the environment (no key service).
 * Depends on: AppContext, useKeys, KeyStep, VaultLockOnQuit. Called by: settings/SettingsPage (the Settings dialog is gone).
 * Invariants: secrets are only ever passed to the controller.
 */
import { KeyNotice } from "../shell/KeyNotice";
import { Button } from "../../components/Controls";

import { VAULT_LOCKED_NOTICE, vaultName } from "../../state/keyText";
import { TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { MistralKeyDetails } from "../secrets/MistralKeySettings";
import { SharedVaultLockNote, VaultLockControls } from "../secrets/VaultLockControls";
import { KeyStep } from "./KeyStep";
import { useKeyUi } from "./useKeys";
import { VaultLockOnQuit } from "./VaultLockOnQuit";

/** The section. */
export function KeySettings() {
  const { keys, shell } = useApp();
  const availability = useKeyUi((s) => s.availability);
  const status = useKeyUi((s) => s.status);
  const notice = useKeyUi((s) => s.notice);
  if (availability === "failed") return <KeyNotice inSettings />;
  if (keys === null || availability !== "ready" || status === null) return null;
  const unlocked = status.vaultSelected && status.vaultState === "unlocked";
  const locked = status.vaultSelected && status.vaultState === "locked";
  const ready = unlocked && status.keyState === "ready";
  const where = `your ${vaultName(status.vaultKind)}`;

  return (
    <section aria-labelledby="key-settings-title" data-setting="mistral-key">
      <h3 id="key-settings-title">Mistral key</h3>
      {unlocked && !ready && <p className="key-line">The vault is open but holds no key yet.</p>}
      {locked && (
        <p className="key-line">
          {VAULT_LOCKED_NOTICE} Your key is in {where}.
        </p>
      )}
      {!unlocked && !locked && <p className="key-line">No key is set up yet.</p>}
      {status.vaultSelected && (
        <p className="key-line">
          {TEXT.vault}: {vaultName(status.vaultKind)}
          {status.vaultPath !== null && (
            <>
              {" "}
              · Folder: <span className="vault-path">{status.vaultPath}</span>
            </>
          )}
        </p>
      )}
      {ready && (
        <MistralKeyDetails
          location="account"
          extra={<VaultLockControls mode="vault" display="button" />}
        />
      )}
      {notice !== null && <p className="key-field-hint ui-help">{notice}</p>}

      {!unlocked && !locked && (
        <Button
          data-testid="setup.key-settings-reopen"
          type="button"
          shape="outlined"
          onClick={() => {
            keys.reopen();
            // The setup screen covers the work view: go there (Settings is a page over it).
            shell.actions.closeSettings();
          }}
        >
          Set up the key
        </Button>
      )}
      {unlocked && !ready && <KeyStep />}

      {ready && <SharedVaultLockNote />}
      {(unlocked || locked) && <VaultLockOnQuit />}
    </section>
  );
}
