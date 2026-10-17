/**
 * A small inline "type your password" form for the Settings actions that need the vault password
 * (unlock, forget or remember this computer).
 * Depends on: fields, useKeys. Called by: KeySettings.
 * Invariants: the password is cleared the moment it is submitted; the failure text (for example a
 * wrong password) comes from the key controller's `problem`.
 */
import { Button } from "../../components/Controls";
import { useState } from "react";

import { Problem, SecretField } from "./fields";
import { useKeyUi } from "./useKeys";

/** The form. */
export function PasswordAsk(props: {
  label?: string;
  action: string;
  autoFocus?: boolean;
  onSubmit: (password: string) => void;
  onCancel?: () => void;
}) {
  const busy = useKeyUi((s) => s.busy);
  const problem = useKeyUi((s) => s.problem);
  const [password, setPassword] = useState("");
  return (
    <form
      className="key-ask"
      onSubmit={(e) => {
        e.preventDefault();
        if (password === "") return;
        const pass = password;
        setPassword("");
        props.onSubmit(pass);
      }}
    >
      <SecretField
        testId="setup.password-ask-secretfield"
        label={props.label ?? "Vault password"}
        value={password}
        onChange={setPassword}
        autoFocus={props.autoFocus}
        invalid={problem?.reason === "wrongPassphrase"}
      />
      <Problem text={problem?.text ?? null} />
      <div className="set-actions">
        <Button
          data-testid="setup.password-ask-btn-primary"
          type="submit"
          small
          variant="primary"
          disabled={busy || password === ""}
        >
          {props.action}
        </Button>
        {props.onCancel !== undefined && (
          <Button
            data-testid="setup.password-ask-tb-btn"
            type="button"
            small
            onClick={props.onCancel}
          >
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}
