/**
 * A small inline "type your password" form for the Settings actions that need the vault password
 * (unlock, forget or remember this computer).
 * Depends on: fields, useKeys. Called by: KeySettings.
 * Invariants: the password is cleared the moment it is submitted; the failure text (for example a
 * wrong password) comes from the key controller's `problem`.
 */
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
        label={props.label ?? "Vault password"}
        value={password}
        onChange={setPassword}
        autoFocus={props.autoFocus}
        invalid={problem?.reason === "wrongPassphrase"}
      />
      <Problem text={problem?.text ?? null} />
      <div className="key-actions">
        <button type="submit" className="btn-primary" disabled={busy || password === ""}>
          {props.action}
        </button>
        {props.onCancel !== undefined && (
          <button type="button" className="tb-btn" onClick={props.onCancel}>
            Cancel
          </button>
        )}
      </div>
    </form>
  );
}
