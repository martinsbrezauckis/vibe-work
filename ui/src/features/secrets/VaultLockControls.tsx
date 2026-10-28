/**
 * Shared direct lock buttons for the Vault header, status bar and Account settings, and the help line shown where
 * Lock is offered for a vault shared with ShellX (one vault file, so Lock also locks ShellX's copy at its next start).
 * Agent pause protects project secrets, website logins and MCP keys; vault lock also stops sessions.
 */
import { Icon } from "../../components/Icon";
import { Button, IconButton } from "../../components/Controls";
import { term, TEXT } from "../../vocabulary";
import { useKeyUi } from "../setup/useKeys";
import { useVaultAccess } from "./useVaultAccess";

export type VaultLockMode = "agent" | "vault";
export type VaultLockDisplay = "icon" | "status" | "button";
export const VAULT_LOCK_TIP = term("locks", "vaultLock").tooltip;

/** Direct action; its label, icon and tooltip always describe the current state and click. */
export function VaultLockControls({
  mode,
  display = "button",
}: {
  mode: VaultLockMode;
  display?: VaultLockDisplay;
}) {
  const access = useVaultAccess();
  const locked = mode === "vault" ? access.vaultLocked : access.agentPaused !== false;
  const unavailable = mode === "agent" && access.agentPaused === null;
  const recovery = mode === "agent" && access.vaultLocked;
  const action = recovery
    ? "vaultUnlock"
    : mode === "vault"
      ? locked
        ? "vaultUnlock"
        : "vaultLock"
      : locked
        ? "agentResume"
        : "agentPause";
  const { name: label, tooltip: tip } = term("locks", action);
  const status =
    unavailable && !recovery
      ? TEXT.agentUnavailableStatus
      : recovery
        ? TEXT.vaultLockedStatus
        : mode === "vault"
          ? locked
            ? TEXT.vaultLockedStatus
            : TEXT.vaultUnlockedStatus
          : locked
            ? TEXT.agentPausedStatus
            : TEXT.agentAvailableStatus;
  const icon =
    mode === "agent" && display === "icon"
      ? "key-round"
      : locked || recovery
        ? "lock"
        : "lock-open";
  if (display === "icon")
    return (
      <IconButton
        icon={icon}
        state={locked || recovery ? "locked" : "unlocked"}
        label={label}
        aria-pressed={locked || recovery}
        aria-description={status}
        title={tip}
        data-testid={`secrets.vault-${mode}-${display}`}
        disabled={
          access.busy || (mode === "vault" && !access.vaultAvailable) || (unavailable && !recovery)
        }
        onClick={mode === "vault" ? access.toggleVault : access.toggleAgent}
      />
    );
  return (
    <Button
      type="button"
      appearance={display === "status" ? "status" : undefined}
      shape={display === "button" ? "outlined" : "ghost"}
      data-testid={
        display === "status" ? "secrets.statusbar-vault" : `secrets.vault-${mode}-${display}`
      }
      aria-label={display === "status" ? status : label}
      title={tip}
      disabled={
        access.busy || (mode === "vault" && !access.vaultAvailable) || (unavailable && !recovery)
      }
      onClick={mode === "vault" ? access.toggleVault : access.toggleAgent}
    >
      <Icon name={icon} size={display === "status" ? 14 : 16} />
      {display === "status" ? <span>{status}</span> : label}
    </Button>
  );
}

/** Where Lock is offered for an unlocked vault shared with ShellX: what Lock does there too. Nothing otherwise. */
export function SharedVaultLockNote() {
  const shared = useKeyUi(
    (s) => s.status?.vaultKind === "shellx" && s.status.vaultState === "unlocked",
  );
  if (!shared) return null;
  return (
    <p className="key-field-hint ui-help" data-testid="secrets.vault-shared-lock-note">
      {TEXT.vaultSharedLockNote}
    </p>
  );
}
