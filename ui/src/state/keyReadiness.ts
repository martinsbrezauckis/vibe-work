/**
 * Mistral key and vault guards and recovery copy, derived from the key controller's metadata.
 * Depends on: keyStore, keyView. Called by: the shell notice, status bar and composer.
 * Invariants: one stable description per state; no secrets or I/O; only a rejected key test blocks
 * sending. Runs without a key service keep their existing environment-key flow.
 */
import type { Store } from "./createStore";
import type { KeyUi } from "./keyStore";
import { VAULT_LOCKED_NOTICE } from "./keyText";
import { stepOf } from "./keyView";
import { term } from "../vocabulary";

/** Why the agent cannot answer, and where the user can make the key usable. */
export interface KeyReadiness {
  kind: "setup" | "locked" | "missing" | "failed" | "testing" | "blocked" | "unread";
  /** One sentence shared by the chat notice and the Send tooltip. */
  message: string;
  /** The same state in a short status-bar label. */
  status: string;
  action: string;
  destination: "setup" | "account" | "retry";
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

const UNREAD: KeyReadiness = {
  kind: "unread",
  message: "Could not read the Mistral key status. Try again.",
  status: "Mistral key status unavailable",
  action: "Retry",
  destination: "retry",
};

/** Returns a stable blocking description, or `null` when the key can be used. */
export function keyReadiness(state: KeyUi): KeyReadiness | null {
  if (state.availability === "failed") return UNREAD;
  if (state.availability !== "ready") return null;
  if (state.status?.vaultState === "selectedVaultMissing") return VAULT_MISSING;
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
      if (
        state.test.phase === "failed" &&
        ["unauthorized", "forbidden"].includes(state.test.reason)
      )
        return FAILED;
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
const VAULT_MISSING: KeyReadiness = {
  kind: "blocked",
  message: "Selected vault not found.",
  status: "Vault not found",
  action: "Choose a vault",
  destination: "setup",
};

/** How long a read waits for a key that is still being read or tested before it goes on with what it knows. */
const KEY_SETTLE_LIMIT_MS = 30_000;

/**
 * True while the key is neither usable nor refused yet: its first status has not arrived, or its test is running.
 * A locked vault, a missing key and a rejected test are answers, not moments of waiting.
 */
function keySettling(state: KeyUi): boolean {
  return state.availability === "loading" || keyReadiness(state)?.kind === "testing";
}

/**
 * Resolves once the stored key can be judged: at once when it already can, else when the first status arrives or the
 * running test ends. Gives up after `limitMs` so a stalled status read cannot hold a request for ever; the caller then
 * decides with `keyReadiness` as it stands. Never rejects.
 */
export function whenKeySettled(
  store: Store<KeyUi>,
  limitMs: number = KEY_SETTLE_LIMIT_MS,
): Promise<void> {
  if (!keySettling(store.get())) return Promise.resolve();
  return new Promise((resolve) => {
    const finish = () => {
      clearTimeout(timer);
      stop();
      resolve();
    };
    const stop = store.subscribe(() => {
      if (!keySettling(store.get())) finish();
    });
    const timer = setTimeout(finish, limitMs);
  });
}
