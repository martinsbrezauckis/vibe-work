/**
 * Settings, "Mistral key": what holds the key and whether it is open, with shared test/replace, remove, vault lock
 * and forget (or remember) this computer. Same words as the setup screen. Hidden when this run keeps
 * its key in the environment (no key service).
 * Depends on: AppContext, useKeys, KeyStep, PasswordAsk, fields. Called by: settings/SettingsPage (the Settings dialog is gone).
 * Invariants: removing a key asks first; forgetting or remembering this computer needs the password
 * (the core re-wraps the vault key), asked inline; secrets are only ever passed to the controller.
 */
import { useState } from "react";

import { VAULT_LOCKED_NOTICE, vaultName } from "../../state/keyText";
import { TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { MistralKeyDetails } from "../secrets/MistralKeySettings";
import { VAULT_LOCK_TIP, VaultLockControls } from "../secrets/VaultLockControls";
import { KeyStep } from "./KeyStep";
import { PasswordAsk } from "./PasswordAsk";
import { useKeyUi } from "./useKeys";

type Mode = "none" | "remove" | "remember" | "forget";

/** The section. */
export function KeySettings() {
  const { keys, shell } = useApp();
  const availability = useKeyUi((s) => s.availability);
  const status = useKeyUi((s) => s.status);
  const busy = useKeyUi((s) => s.busy);
  const notice = useKeyUi((s) => s.notice);
  const [mode, setMode] = useState<Mode>("none");
  if (keys === null || availability !== "ready" || status === null) return null;
  const back = () => {
    keys.clearProblem();
    setMode("none");
  };
  const unlocked = status.vaultSelected && status.vaultState === "unlocked";
  const locked = status.vaultSelected && status.vaultState === "locked";
  const ready = unlocked && status.keyState === "ready";
  const canRemember = status.vaultKind === "shellx" || status.vaultKind === "vibeWork";
  const where = `your ${vaultName(status.vaultKind)}`;

  return (
    <section aria-labelledby="key-settings-title" data-setting="mistral-key">
      <h3 id="key-settings-title">Mistral key</h3>
      {ready && (
        <p className="key-line">
          A key is in use. It is kept in {where}, which is open.{" "}
          {status.rememberedDevice
            ? "This computer remembers the password."
            : "This computer does not remember the password."}
        </p>
      )}
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
          . Changing vaults is not available yet.
        </p>
      )}
      {ready && <MistralKeyDetails location="account" />}
      {notice !== null && <p className="key-field-hint">{notice}</p>}

      {locked && mode === "none" && (
        <PasswordAsk
          action="Unlock"
          onSubmit={(password) => {
            void keys.unlock(password);
          }}
        />
      )}
      {!unlocked && !locked && (
        <button
          type="button"
          className="tb-btn"
          onClick={() => {
            keys.reopen();
            // The setup screen covers the work view: go there (Settings is a page over it).
            shell.actions.closeSettings();
          }}
        >
          Set up the key
        </button>
      )}
      {unlocked && !ready && mode === "none" && <KeyStep />}

      {ready && mode === "none" && (
        <div className="key-actions wrap">
          <button
            type="button"
            className="tb-btn"
            onClick={() => {
              setMode("remove");
            }}
          >
            Remove key
          </button>
          <VaultLockControls mode="vault" display="button" />
          {canRemember && (
            <button
              type="button"
              className="tb-btn"
              onClick={() => {
                setMode(status.rememberedDevice ? "forget" : "remember");
              }}
            >
              {status.rememberedDevice ? "Forget this computer" : "Remember this computer"}
            </button>
          )}
        </div>
      )}
      {ready && mode === "none" && <p className="key-field-hint">{VAULT_LOCK_TIP}</p>}
      {mode === "remove" && (
        <div className="key-offer">
          <p>
            Remove the key from Vibe Work? Sessions will ask for a key again. Nothing else in your
            vault changes.
          </p>
          <div className="key-actions">
            <button
              type="button"
              className="btn-primary"
              disabled={busy}
              onClick={() => {
                void keys.removeKey().then(back);
              }}
            >
              Remove key
            </button>
            <button type="button" className="tb-btn" onClick={back}>
              Cancel
            </button>
          </div>
        </div>
      )}
      {(mode === "forget" || mode === "remember") && (
        <div className="key-offer">
          <p>
            {mode === "forget"
              ? "Forget this computer? You will type the password each time the app starts."
              : "Remember this computer? Programs that run as you could then reach the unlock code in the system keychain."}
          </p>
          <PasswordAsk
            action={mode === "forget" ? "Forget this computer" : "Remember this computer"}
            autoFocus
            onSubmit={(password) => {
              void keys.setRemember(password, mode === "remember").then((ok) => {
                if (ok) back();
              });
            }}
            onCancel={back}
          />
        </div>
      )}
    </section>
  );
}
