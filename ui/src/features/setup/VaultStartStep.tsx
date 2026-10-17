/**
 * First-run vault choice. The core reports only vaults actually present; this screen also offers a
 * new private Vibe Work vault. A local choice is only committed when creation or unlock succeeds.
 * Depends on: key status, CreateStep, UnlockStep. Called by: SetupScreen.
 */
import { useState } from "react";

import type { VaultChoice } from "../../protocol/core";
import { term, TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { CreateStep } from "./CreateStep";
import { UnlockStep } from "./UnlockStep";
import { useKeyUi } from "./useKeys";

type Choice = VaultChoice | "create";
const EMPTY: readonly VaultChoice[] = [];

/** Offers creation and only the existing vaults reported by the core. */
export function VaultStartStep() {
  const { keys } = useApp();
  const available = useKeyUi((s) => s.status?.availableVaults ?? EMPTY);
  const [choice, setChoice] = useState<Choice | null>(null);
  const choose = (next: Choice | null) => {
    keys?.clearProblem();
    setChoice(next);
  };
  if (choice !== null) {
    return (
      <div className="setup-step">
        <button
          data-testid="setup.vault-start-step-choose"
          type="button"
          className="key-link"
          onClick={() => {
            choose(null);
          }}
        >
          Choose another vault
        </button>
        {choice === "create" ? <CreateStep /> : <UnlockStep choice={choice} />}
      </div>
    );
  }

  return (
    <div className="setup-step">
      <p>
        Vibe Work keeps your Mistral key and project secrets encrypted in a vault. Their values are
        never stored as plain text or shown to agents. The app needs your Mistral key to work, so
        choose a vault first and protect it with one password.
      </p>
      <div
        className="vault-choices"
        role="group"
        aria-label="Choose a vault"
        data-testid="setup-vault-chooser"
      >
        <button
          type="button"
          className="vault-choice"
          data-testid="setup.vault-start-create"
          aria-label={term("vaultKinds", "vibeWork").tooltip}
          title={term("vaultKinds", "vibeWork").tooltip}
          onClick={() => {
            choose("create");
          }}
        >
          <strong>{term("vaultKinds", "vibeWork").tooltip}</strong>
          <span>{term("vaultKinds", "vibeWork").description}</span>
        </button>
        {available.includes("shellx") && (
          <button
            type="button"
            className="vault-choice"
            data-testid="setup.vault-start-shellx"
            aria-label={term("vaultKinds", "shellx").tooltip}
            title={term("vaultKinds", "shellx").tooltip}
            onClick={() => {
              choose("shellx");
            }}
          >
            <strong>{term("vaultKinds", "shellx").tooltip}</strong>
            <span>{term("vaultKinds", "shellx").description}</span>
          </button>
        )}
        {available.includes("desktop") && (
          <button
            type="button"
            className="vault-choice"
            data-testid="setup.vault-start-desktop"
            aria-label={term("vaultKinds", "desktop").tooltip}
            title={term("vaultKinds", "desktop").tooltip}
            onClick={() => {
              choose("desktop");
            }}
          >
            <strong>{term("vaultKinds", "desktop").tooltip}</strong>
            <span>{term("vaultKinds", "desktop").description}</span>
          </button>
        )}
      </div>
      {available.length === 0 && <p className="key-field-hint">{TEXT.vaultChoiceEmpty}</p>}
      <button
        data-testid="setup.vault-start-step-dismiss"
        type="button"
        className="key-link"
        onClick={() => keys?.dismiss()}
      >
        Not now
      </button>
    </div>
  );
}
