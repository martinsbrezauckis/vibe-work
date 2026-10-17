/**
 * Shared direct lock buttons for the Vault header, status bar and Account settings.
 * Agent pause protects project secrets, website logins and MCP keys; vault lock also stops sessions.
 */
import { Icon } from "../../components/Icon";
import { IconButton } from "../../components/Controls";
import { term, TEXT } from "../../vocabulary";
import { useVaultAccess } from "./useVaultAccess";

export type VaultLockMode = "agent" | "vault";
export type VaultLockDisplay = "icon" | "status" | "button";

export const AGENT_PAUSE_TIP = term("locks", "agentPause").tooltip;
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
        aria-label={label}
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
    <button
      type="button"
      className={display === "status" ? "sb-vault" : "tb-btn"}
      data-size={display === "button" ? "small" : undefined}
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
    </button>
  );
}
