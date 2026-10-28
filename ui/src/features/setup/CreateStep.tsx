/**
 * Setup step for a new private Vibe Work vault: paste the Mistral key (or take the one Vibe already
 * has) and choose a password. One password field with
 * a strength hint and show/hide, no second confirm field.
 * Depends on: AppContext, useKeys, fields, VibeKeyOffer, ConsoleLink. Called by: SetupScreen.
 * Invariants: the key and the password live only in this component's state until submit, then both
 * are cleared whether or not the call worked; a failure leaves the user on this step with the reason.
 */
import { Button } from "../../components/Controls";
import { useState } from "react";
import { term } from "../../vocabulary";

import { Selector } from "../../components/Selector";
import { useApp } from "../shell/AppContext";
import { ConsoleLink } from "./ConsoleLink";
import { NewPasswordField, Problem, SecretField } from "./fields";
import { VibeKeyNote } from "./VibeKeyOffer";
import { useKeyUi } from "./useKeys";

/** The step. */
export function CreateStep() {
  const { keys } = useApp();
  const vibeKeys = useKeyUi((s) => s.vibeKeys);
  const busy = useKeyUi((s) => s.busy);
  const problem = useKeyUi((s) => s.problem);
  const found = vibeKeys[0];
  const [useVibe, setUseVibe] = useState(true);
  const [key, setKey] = useState("");
  const [password, setPassword] = useState("");
  const fromVibe = found !== undefined && useVibe;
  const ready = password !== "" && (fromVibe || key.trim() !== "");

  const submit = () => {
    if (keys === null || !ready) return;
    const how = fromVibe ? { importFrom: found.source } : { key };
    const pass = password;
    // Clear first: whatever happens next, the inputs no longer hold the secrets.
    setKey("");
    setPassword("");
    void keys.createWithKey(pass, how);
  };

  return (
    <form
      className="setup-step"
      data-testid="setup-vault-form"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <h3>{term("vaultKinds", "vibeWork").tooltip}</h3>
      <p>
        This vault is private to Vibe Work and saved in its own folder. Choose one password to open
        it.
      </p>
      {found !== undefined && (
        <fieldset className="key-offer">
          <legend className="key-sr">Which key</legend>
          <VibeKeyNote found={found} />
          <Selector
            label="Which key"
            testId="setup.key-choice"
            value={useVibe ? "vibe" : "paste"}
            options={[
              { value: "vibe", label: "Use Vibe's key", testId: "setup.key-vibe" },
              { value: "paste", label: "Paste a different key", testId: "setup.key-paste" },
            ]}
            onChange={(value) => {
              setUseVibe(value === "vibe");
            }}
          />
        </fieldset>
      )}
      {!fromVibe && (
        <>
          <SecretField
            label="Paste your Mistral API key"
            testId="setup.key-input"
            value={key}
            onChange={setKey}
            placeholder="Your key"
            autoFocus
          />
          <ConsoleLink />
        </>
      )}
      <NewPasswordField value={password} onChange={setPassword} testId="setup.password-input" />
      <p className="key-field-hint">
        If you forget the password, paste your key again. Vibe Work never sees your Mistral account
        password.
      </p>
      <Problem text={problem?.text ?? null} />
      <div className="key-actions">
        <Button
          type="submit"
          appearance="dialog"
          data-testid="setup.save"
          disabled={busy || !ready}
        >
          {busy ? "Saving…" : "Save and test the key"}
        </Button>
        <Button
          data-testid="setup.create-step-dismiss"
          type="button"

          onClick={() => {
            keys?.dismiss();
          }}
        >
          Not now
        </Button>
      </div>
    </form>
  );
}
