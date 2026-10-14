/**
 * Plain-words text for the Mistral key screens: what a failed key or vault operation means and what
 * to do about it, and the password hint. Pure functions, no state.
 * Depends on: protocol/core types. Called by: keyStore, the setup and settings components.
 * Invariants: no text here contains a secret (the core never returns one); wording uses "password"
 * and "key", never Vault jargon ("passphrase", "keyfile", "resource"); every error text says what
 * to do next when the user can do something.
 */
import type { CoreError } from "../protocol/core";
import { TEXT, vaultTerm } from "../vocabulary";

/** Shared notice wherever a cryptographically locked vault stops sessions. */
export const VAULT_LOCKED_NOTICE = TEXT.vaultLockedNotice;

/** Stable product label for the selected vault, across setup and Settings. */
export function vaultName(kind: string | null): string {
  return vaultTerm(kind).name;
}

/** What each reason word from the core's `keyService` errors means for the user. */
const REASONS: Readonly<Record<string, string>> = {
  wrongPassphrase: "That password did not open your vault. Check it and try again.",
  emptyPassphrase: "Type a password first.",
  invalidKey:
    "That does not look like a Mistral API key. Copy the whole key again, without spaces or line breaks.",
  unauthorized:
    "Mistral did not accept this key (error 401). Check that you copied all of it, or create a new key in the Mistral console and paste that.",
  forbidden:
    "Mistral knows this key but does not let it use the models (error 403). Check the key's permissions and your plan in the Mistral console, or use another key.",
  rateLimited:
    "Mistral says there are too many requests right now (error 429). The key may be fine; try again in a minute.",
  mistralServerError: "Mistral had a problem on its side. Try again in a few minutes.",
  unexpectedStatus: "Mistral answered with something unexpected. Try again in a few minutes.",
  network:
    "Vibe Work could not reach Mistral. Check your internet connection (and any proxy or firewall), then try again.",
  locked: `${VAULT_LOCKED_NOTICE} Type your password to unlock it first.`,
  alreadyExists: "A vault already exists on this computer. Unlock it with its password instead.",
  conflict:
    "Another program changed the vault at the same moment, so nothing was saved. Try again.",
  corrupt:
    "The vault file could not be read. It may be damaged; check the vault folder and your backups.",
  selectionMismatch:
    "The selected vault changed since Vibe Work last opened it. Check the vault folder before trying again.",
  selectedVaultMissing:
    "The selected vault could not be found. Check that its folder is still available.",
  selectionRequired: "Choose a vault before unlocking it.",
  io: "Vibe Work could not read or write the vault folder. Check that the folder is writable.",
  rememberFailed:
    "The vault was opened, but this computer could not remember it. You will type the password again next time.",
  noKey: "There is no Mistral key yet. Paste one to continue.",
  nothingToImport: "Vibe's saved key is gone. Paste a key instead.",
  notACandidate: "That saved key is no longer in the vault. Pick another or paste a key.",
  existingKeyChange:
    "Vibe's own copy of the key could not be changed. Remove it in Vibe if you want it gone.",
  settings: "Vibe Work could not save its key setting. Check that its data folder is writable.",
  noVault: "There is no vault on this computer yet.",
  twoVaults: "Choose which vault Vibe Work should use on this computer.",
  remoteNotSupported:
    "Your vault is stored on a server. Vibe Work only supports vaults on this computer for now.",
  legacyProfile:
    "Your vault is in an older location. Open ShellX once so it moves the vault, then come back.",
  setupUnfinished:
    "A vault was started in ShellX but not finished. Finish it there, then come back.",
  canonicalForbidden: "This test run is not allowed to open the real vault.",
  internal: "Something went wrong inside Vibe Work. Try again.",
  notConfigured: "This run keeps the key in the environment, so there is nothing to set up.",
};

/** The user-facing sentence for a failed `keys` command. */
export function describeProblem(error: CoreError): string {
  if (error.code === "confirmationDeclined") return "The request was declined. Nothing changed.";
  const reason = error.serverCode;
  if (error.code === "keyService" && reason !== null) {
    const known = REASONS[reason];
    if (known !== undefined) return known;
  }
  return error.message;
}

/** The title of what went wrong in one word, for tests and the state route (`""` when unknown). */
export function reasonOf(error: CoreError): string {
  return error.code === "keyService" ? (error.serverCode ?? "") : error.code;
}

/** A password's rough strength: only a hint, never a gate (the user decides). */
export interface PasswordHint {
  level: "empty" | "short" | "ok" | "good";
  text: string;
}

/** Judges `password` by length and variety; the text says what would make it better. */
export function passwordHint(password: string): PasswordHint {
  if (password === "") return { level: "empty", text: "Choose something you will remember." };
  const classes = [/[a-z]/, /[A-Z]/, /[0-9]/, /[^A-Za-z0-9]/].filter((r) =>
    r.test(password),
  ).length;
  if (password.length < 8)
    return { level: "short", text: "Short. 12 or more characters is much safer." };
  if (password.length >= 14 || (password.length >= 10 && classes >= 3))
    return { level: "good", text: "Good." };
  return { level: "ok", text: "Fine. A longer password, or a few words together, is safer." };
}

/** What to tell the user when the vault state (`remoteNotSupported`, ...) blocks setup. */
export function blockedText(vaultState: string): string {
  return REASONS[vaultState] ?? "The vault on this computer cannot be used yet.";
}
