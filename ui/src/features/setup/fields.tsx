/**
 * Small form pieces the key screens share: a secret text field with show/hide, the password field
 * with its strength hint, the "remember on this computer" choice with its disclosure, and the
 * problem line.
 * Depends on: state/keyText. Called by: the setup steps, KeySettings, PasswordAsk.
 * Invariants: secret inputs are `type="password"` until the user presses Show, have autocomplete
 * off and no spellcheck, and are controlled by the parent so it can clear them on submit; nothing
 * here keeps a secret in its own state; labels are real `<label>`s. Every secret input carries
 * `data-secret`: the Debug API's element routes match selectors with its value emptied, window
 * screenshots blank it, and during a capture it shows dots with Show disabled (`shell/secretSafety.ts`).
 */
import { useId, useState } from "react";

import { passwordHint } from "../../state/keyText";
import { useCaptureLocked } from "../shell/secretSafety";

/** A secret text input with a Show/Hide button. */
export function SecretField(props: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  /** Extra line under the field. */
  hint?: string | undefined;
  hintLevel?: string | undefined;
  placeholder?: string | undefined;
  autoFocus?: boolean | undefined;
  /** `current-password` (default) or `new-password`: tells password managers what this is. */
  autoComplete?: "current-password" | "new-password" | "off";
  invalid?: boolean | undefined;
  /** Stable selector supplied by the calling setup step. */
  testId: string;
}) {
  const id = useId();
  const [shown, setShown] = useState(false);
  // while the Debug API captures the window the value is never shown in clear.
  const locked = useCaptureLocked();
  const hintId = `${id}-hint`;
  return (
    <div className="key-field">
      <label htmlFor={id}>{props.label}</label>
      <div className="key-field-row">
        <input
          id={id}
          data-secret="true"
          data-testid={props.testId}
          type={shown && !locked ? "text" : "password"}
          value={props.value}
          placeholder={props.placeholder}
          autoFocus={props.autoFocus}
          autoComplete={props.autoComplete ?? "off"}
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          aria-invalid={props.invalid === true ? true : undefined}
          aria-describedby={props.hint === undefined ? undefined : hintId}
          onChange={(e) => {
            props.onChange(e.target.value);
          }}
        />
        <button
          data-testid={`${props.testId}.show`}
          type="button"
          className="tb-btn"
          aria-pressed={shown}
          disabled={locked}
          onClick={() => {
            setShown(!shown);
          }}
        >
          {shown ? "Hide" : "Show"}
        </button>
      </div>
      {props.hint !== undefined && (
        <p id={hintId} className="key-field-hint" data-level={props.hintLevel}>
          {props.hint}
        </p>
      )}
    </div>
  );
}

/** The password field for a new password: one field, a strength hint, no second confirm field. */
export function NewPasswordField(props: {
  value: string;
  onChange: (value: string) => void;
  testId: string;
}) {
  const hint = passwordHint(props.value);
  return (
    <SecretField
      label="Choose a password to lock it"
      testId={props.testId}
      value={props.value}
      onChange={props.onChange}
      hint={hint.text}
      hintLevel={hint.level}
      autoComplete="new-password"
    />
  );
}

/** "Remember on this computer", on by default, with a plain explanation behind a button. */
export function RememberChoice(props: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  testId: string;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  return (
    <div className="key-remember">
      <label className="key-check">
        <input
          type="checkbox"
          data-testid={props.testId}
          checked={props.checked}
          onChange={(e) => {
            props.onChange(e.target.checked);
          }}
        />
        <span>Remember on this computer</span>
      </label>
      <button
        data-testid={`${props.testId}.explanation`}
        type="button"
        className="key-link"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => {
          setOpen(!open);
        }}
      >
        What does this mean?
      </button>
      {open && (
        <p id={id} className="key-field-hint remember-note">
          Vibe Work keeps a code in your system&apos;s keychain that opens the vault, so you do not
          type the password every time. Programs that run as you on this computer, including
          commands an agent runs for you, could reach that code until Vibe Work runs agents in a
          sandbox. Turn this off on a shared computer. You can change it later in Settings.
        </p>
      )}
    </div>
  );
}

/** An error line (empty when `text` is null). */
export function Problem({ text }: { text: string | null }) {
  if (text === null) return null;
  return (
    <p className="key-problem" role="alert">
      {text}
    </p>
  );
}
