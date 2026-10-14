/**
 * Mistral key and vault guards and recovery copy, derived from the key controller's metadata.
 * Depends on: keyStore, keyView. Called by: the shell notice, status bar and composer.
 * Invariants: one stable description per state; no secrets or I/O; a failed test blocks sending
 * until a new test succeeds. Runs without a key service keep their existing environment-key flow.
 */
import type { KeyUi } from "./keyStore";
import { VAULT_LOCKED_NOTICE } from "./keyText";
import { stepOf } from "./keyView";
import { term } from "../vocabulary";

/** Why the agent cannot answer, and where the user can make the key usable. */
export interface KeyReadiness {
  kind: "setup" | "locked" | "missing" | "failed" | "testing" | "blocked";
  /** One sentence shared by the chat notice and the Send tooltip. */
  message: string;
  /** The same state in a short status-bar label. */
  status: string;
  action: string;
  destination: "setup" | "account";
}

const SETUP: KeyReadiness = {
  kind: "setup",
  message: "No Mistral key is set up, so the agent cannot answer.",
  status: "Mistral key not set up",
  action: "Set up Mistral key",
  destination: "setup",
};
const LOCKED: KeyReadiness = {
  kind: "locked",
  message: VAULT_LOCKED_NOTICE,
  status: "Mistral key locked",
  action: term("locks", "vaultUnlock").name,
  destination: "account",
};
const PICK_VAULT: KeyReadiness = {
  ...LOCKED,
  action: "Open vault setup",
  destination: "setup",
};
const MISSING: KeyReadiness = {
  kind: "missing",
  message: "Your vault has no Mistral key, so the agent cannot answer.",
  status: "No Mistral key",
  action: "Add Mistral key",
  destination: "account",
};
const FAILED: KeyReadiness = {
  kind: "failed",
  message: "Your Mistral key did not pass its test, so the agent cannot answer.",
  status: "Mistral key test failed",
  action: "Check Mistral key",
  destination: "account",
};
const TESTING: KeyReadiness = {
  kind: "testing",
  message: "Your Mistral key is being tested, so the agent cannot answer yet.",
  status: "Testing Mistral key",
  action: "Check Mistral key",
  destination: "account",
};
const BLOCKED: KeyReadiness = {
  kind: "blocked",
  message: "Your vault is unavailable, so the agent cannot answer without your Mistral key.",
  status: "Mistral key unavailable",
  action: "Open vault setup",
  destination: "setup",
};

/** Returns a stable blocking description, or `null` when the key can be used. */
export function keyReadiness(state: KeyUi): KeyReadiness | null {
  if (state.availability !== "ready") return null;
  switch (stepOf(state.status).kind) {
    case "create":
      return SETUP;
    case "pickVault":
      return PICK_VAULT;
    case "unlock":
      return LOCKED;
    case "key":
      return MISSING;
    case "blocked":
      return BLOCKED;
    case "ready":
      if (state.test.phase === "failed") return FAILED;
      if (state.test.phase === "running") return TESTING;
      return null;
    case "loading":
      return null;
  }
}

/** Vault-only operations remain available without a Mistral key or a successful key test. */
export function vaultReadiness(state: KeyUi): KeyReadiness | null {
  const blocked = keyReadiness(state);
  if (blocked === null || ["missing", "failed", "testing"].includes(blocked.kind)) return null;
  if (blocked === SETUP) return VAULT_SETUP;
  return blocked;
}

const VAULT_SETUP: KeyReadiness = { ...SETUP, action: "Set up vault" };
