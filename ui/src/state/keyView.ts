/**
 * Which setup screen the key status calls for: a pure mapping from the core's `KeyStatusView`
 * (plus two local flags) to one named step, so the screen, the Settings section and the
 * `ui/setup` route agree.
 * Depends on: protocol/core types. Called by: keyStore, the setup components.
 * Invariants: no I/O; an unknown vault state is treated as blocked with the core's own wording
 * (never as "no vault", which would invite creating a second vault).
 */
import type { KeyStatusView } from "../protocol/core";

/** The step the setup screen shows. */
export type SetupStep =
  | { kind: "loading" }
  /** The vault cannot be used from here; `reason` is the core's reason word. */
  | { kind: "blocked"; reason: string }
  /** No vault yet: explain the vault and offer private creation. */
  | { kind: "create" }
  /** No vault was selected yet: choose a private vault or an existing one. */
  | { kind: "pickVault" }
  /** A vault exists and is locked: password. */
  | { kind: "unlock" }
  /** The vault is open and has no key: paste one, or pick a saved one. */
  | { kind: "key" }
  /** The vault is open and a key is in use: test it. */
  | { kind: "ready" };

/** The step for `status` (`null`: not read yet). */
export function stepOf(status: KeyStatusView | null): SetupStep {
  if (status === null) return { kind: "loading" };
  // A discovered vault is not a choice. First run must still offer Vibe Work's own vault.
  if (
    !status.vaultSelected &&
    status.availableVaults.length > 0 &&
    ["locked", "twoVaults", "noVault"].includes(status.vaultState)
  ) {
    return { kind: "pickVault" };
  }
  switch (status.vaultState) {
    case "noVault":
      return { kind: "create" };
    case "twoVaults":
      return { kind: "pickVault" };
    case "locked":
      return { kind: "unlock" };
    case "unlocked":
      return status.keyState === "ready" ? { kind: "ready" } : { kind: "key" };
    default:
      return { kind: "blocked", reason: status.vaultState };
  }
}

/**
 * Does first-run setup take over the content area? It does while the key cannot work yet and the
 * user has not said "not now". After a key has worked (`setupDone` true), a later lock uses the chat
 * notice and Account recovery. The controller separately covers startup when the persisted
 * selection is locked, so prior setup cannot skip the initial password form.
 */
export function needsSetup(step: SetupStep, setupDone: boolean, dismissed: boolean): boolean {
  if (dismissed) return false;
  switch (step.kind) {
    case "loading":
    case "ready":
      return false;
    case "unlock":
      return !setupDone;
    default:
      return true;
  }
}
