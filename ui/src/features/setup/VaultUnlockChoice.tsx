/** Two explicit unlock choices, shared by vault creation and settings. Uses the product vocabulary and native radios. */
import { useId } from "react";
import type { VaultUnlockMethod } from "../../protocol/core";
import { detectMac } from "../../state/shortcuts";
import { TEXT } from "../../vocabulary";

/** The person's existing OS sign-in, with the platform's own name. */
export function systemUnlockLabel(): string {
  if (detectMac()) return TEXT.vaultUnlockMac;
  const nav = typeof navigator === "undefined" ? undefined : navigator;
  const platform =
    (nav as (Navigator & { userAgentData?: { platform?: string } }) | undefined)?.userAgentData
      ?.platform ??
    nav?.platform ??
    "";
  return /win/i.test(platform) ? TEXT.vaultUnlockWindows : TEXT.vaultUnlockLinux;
}

/** No menu or explanatory copy; the selected radio is the saved or pending choice. */
export function VaultUnlockChoice({
  value,
  onChange,
  disabled = false,
  systemDisabled = false,
}: {
  value: VaultUnlockMethod;
  onChange: (value: VaultUnlockMethod) => void;
  disabled?: boolean;
  systemDisabled?: boolean;
}) {
  const name = useId();
  return (
    <fieldset className="vault-unlock-choice" disabled={disabled}>
      <legend>{TEXT.vaultUnlockLabel}</legend>
      {(["system", "password"] as const).map((method) => (
        <label key={method}>
          <input
            type="radio"
            disabled={method === "system" && systemDisabled}
            name={name}
            value={method}
            checked={value === method}
            data-testid="setup.vault-unlock-option"
            data-item-id={method}
            onChange={() => {
              onChange(method);
            }}
          />
          <span>{method === "system" ? systemUnlockLabel() : TEXT.vaultUnlockPassword}</span>
        </label>
      ))}
    </fieldset>
  );
}
