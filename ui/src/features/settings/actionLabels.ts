/**
 * Single-source UI labels for buttons and action controls on every Settings page and related cards.
 * Each component imports its displayed label from here, and `catalog.ts` indexes the exact same
 * strings so Settings search never drifts from what renders on screen.
 * Depends on: `../../vocabulary`. Called by: `catalog.ts`, `SettingsPage.tsx`, `SettingsNav.tsx`,
 * `openHit.ts`, Settings and Secrets components.
 * Invariants: never hardcode canonical vocabulary strings from `vocabulary.json`; always derive
 * browser names and Connect/Vault labels from `TEXT`.
 */
import { TEXT } from "../../vocabulary";

/** Individual named label exports (mangled/inlined cleanly by Vite instead of one large object). */
export const ACT_SHOW_COMMANDS = "Show commands";
/** Button label for scanning other coding agent setups. */
export const ACT_LOOK_FOR_SETUPS = "Look for setups";
/** Button label for importing selected setups. */
export const ACT_BRING_SELECTED = "Bring selected";
/** Button label for undoing the latest setup import. */
export const ACT_UNDO = "Undo";
/** Button label for opening leftover chat copies review. */
export const ACT_REVIEW_COPIES = "Review copies";

/** Button label for allowing a local-network request. */
export const ACT_ALLOW = "Allow";
/** Button label for blocking a local-network request. */
export const ACT_BLOCK = "Block";
/** Button label for forgetting a saved local-network decision. */
export const ACT_FORGET = "Forget";
/** Button label for opening the Vibe Work Firefox add-ons page. */
export const ACT_FIREFOX_ADDONS = "Add-ons";
/** Button label for starting Firefox pairing. */
export const ACT_FIREFOX_PAIR = "Pair Firefox";
/** Disabled button label while connecting a browser extension. */
export const ACT_CONNECTING_ELLIPSIS = "Connecting…";
/** Builds the browser connection action label for `browser`. */
export const connectBrowserLabel = (browser: string): string =>
  `${TEXT.connectorConnect} ${browser}`;
/** Builds the browser pairing action label for `browser`. */
export const pairBrowserLabel = (browser: string): string => `Pair ${browser}`;
/** Button label for reopening a browser pairing page. */
export const ACT_OPEN_PAIRING_PAGE = "Open pairing page";
/** Button label for retrying browser extension setup. */
export const ACT_SETUP_AGAIN = "Set up again";
/** Button label for enabling a browser extension. */
export const ACT_TURN_ON = "Turn on";
/** Button label for opening a removal confirmation dialog. */
export const ACT_REMOVE_ELLIPSIS = "Remove…";
/** Button label for confirming removal. */
export const ACT_REMOVE = "Remove";
/** Button label for cancelling an inline form or confirmation. */
export const ACT_CANCEL = "Cancel";

/** Button label for re-checking browser signed-in accounts. */
export const ACT_REFRESH_ACCOUNTS = "Refresh accounts";
/** Button label for closing a sign-in check panel. */
export const ACT_DONE_CHECKING = "Done";
/** Button label for refreshing plan usage metrics. */
export const ACT_REFRESH_USAGE = "Refresh usage";
/** Action label prefix for signing in to an account provider. */
export const ACT_SIGN_IN = "Sign in";
/** Action label prefix for signing out of an account provider. */
export const ACT_SIGN_OUT = "Sign out";

/** Button label for saving edits. */
export const ACT_SAVE = "Save";
/** Button label for opening instruction diff review. */
export const ACT_REVIEW_CHANGES = "Review changes";
/** Button label for discarding unsaved instruction edits. */
export const ACT_DISCARD_CHANGES = "Discard changes";
/** Button label for returning from diff review to editing. */
export const ACT_BACK_TO_EDITING = "Back to editing";
/** Button label for reloading an instruction file after read failure. */
export const ACT_LOAD_FILE_AGAIN = "Load the file again";
/** Button label for appending terminal AGENTS.md instructions. */
export const ACT_ADD_TERMINAL_INSTRUCTIONS = "Add my terminal's AGENTS.md";
/** Button label for opening the add Mistral library form. */
export const ACT_ADD_LIBRARY = "Add a library";
/** Button label for opening the add local folder/drive form. */
export const ACT_ADD_FOLDER = "Add a folder or drive";
/** Button label for submitting a knowledge base item. */
export const ACT_ADD = "Add";
/** Button label for opening a native folder picker. */
export const ACT_CHOOSE_FOLDER = "Choose folder…";

/** Button label for retrying a failed network settings read. */
export const ACT_TRY_AGAIN = "Try again";
/** Button label for opening a native file picker. */
export const ACT_CHOOSE_FILE = "Choose file…";
/** Button label for testing proxy settings. */
export const ACT_TEST_CONNECTION = "Test connection";

/** Button label for testing the Mistral API key. */
export const ACT_TEST = "Test";
/** Button label for starting Mistral API key replacement. */
export const ACT_REPLACE = "Replace";
/** Button label for unlocking the vault before setting up a key. */
export const ACT_UNLOCK_IN_ACCOUNT = "Unlock in Account";
/** Button label for starting initial Mistral API key setup. */
export const ACT_SET_UP_KEY = "Set up the key";
/** Button label for submitting a replacement Mistral key and testing it. */
export const ACT_REPLACE_AND_TEST = "Replace and test";
/** Button label for opening the add secondary Mistral key form. */
export const ACT_ADD_KEY = "Add key";
/** Button label for activating an existing Mistral key entry. */
export const ACT_USE_THIS_KEY = "Use this key";
/** Button label for opening the new vault entry form. */
export const ACT_ADD_ENTRY = "+ Add entry";
/** Button label for saving a new generated vault secret. */
export const ACT_GENERATE = "Generate";
/** Button label for toggling password generator options. */
export const ACT_OPTIONS = "Options";
/** Button label for scanning chat logs for secrets now. */
export const ACT_CHECK_NOW = "Check now";
/** Button label for adding a custom safety rule. */
export const ACT_ADD_RULE = "Add rule";
/** Button label for clearing the Allowed search filter. */
export const ACT_CLEAR_SEARCH = "Clear the search";
/** Button label for revoking an allowed rule or permission. */
export const ACT_REVOKE = "Revoke";

/** Button label for downloading an app update. */
export const ACT_DOWNLOAD = "Download";
/** Button label for installing a downloaded update and restarting. */
export const ACT_INSTALL_AND_RESTART = "Install and restart";
/** Button label for opening the release notes page. */
export const ACT_OPEN_RELEASE_PAGE = "Open release page";
/** Button label for checking for app updates. */
export const ACT_CHECK_FOR_UPDATES = "Check for updates";
/** Button label for updating Vibe CLI. */
export const ACT_UPDATE_VIBE = "Update Vibe";
/** Button label for opening the feedback dialog. */
export const ACT_SEND_FEEDBACK = "Send feedback…";
/** Button label for regenerating the Debug API token. */
export const ACT_REGENERATE_TOKEN = "Regenerate token";

function browserControlActions(browser: string): readonly string[] {
  return [
    connectBrowserLabel(browser),
    pairBrowserLabel(browser),
    ACT_OPEN_PAIRING_PAGE,
    ACT_SETUP_AGAIN,
    ACT_TURN_ON,
    ACT_REMOVE_ELLIPSIS,
    ACT_REMOVE,
    ACT_CANCEL,
  ];
}

/** Action strings owned by each Settings `data-setting` row. */
export const SETTING_ROW_ACTIONS: Readonly<Record<string, readonly string[]>> = {
  "keyboard-shortcuts": [ACT_SHOW_COMMANDS],
  "import-setup": [ACT_LOOK_FOR_SETUPS, ACT_BRING_SELECTED, ACT_UNDO],
  "session-copies": [ACT_REVIEW_COPIES],
  "chrome-control": browserControlActions("Chrome"),
  "edge-control": browserControlActions("Edge"),
  "safari-control": browserControlActions("Safari"),
  "firefox-control": [
    ACT_FIREFOX_ADDONS,
    ACT_FIREFOX_PAIR,
    TEXT.connectorConnect,
    ACT_CONNECTING_ELLIPSIS,
    ACT_REMOVE_ELLIPSIS,
    ACT_REMOVE,
    ACT_CANCEL,
  ],
  "browser-local-network-sites": [ACT_ALLOW, ACT_BLOCK, ACT_FORGET],
  "browser-secret-permissions": [TEXT.secretPermissionsOpenVault, ACT_REVOKE],
  "signed-in-accounts": [ACT_SIGN_IN, ACT_SIGN_OUT, ACT_REFRESH_ACCOUNTS, ACT_DONE_CHECKING],
  "plan-usage": [ACT_REFRESH_USAGE],
  instructions: [
    ACT_REVIEW_CHANGES,
    ACT_SAVE,
    ACT_DISCARD_CHANGES,
    ACT_BACK_TO_EDITING,
    ACT_LOAD_FILE_AGAIN,
    ACT_ADD_TERMINAL_INSTRUCTIONS,
  ],
  "knowledge-mistral": [ACT_ADD_LIBRARY, ACT_ADD, ACT_CANCEL, ACT_REMOVE],
  "knowledge-local": [ACT_ADD_FOLDER, ACT_CHOOSE_FOLDER, ACT_ADD, ACT_CANCEL, ACT_REMOVE],
  "git-identity": [ACT_SAVE],
  "network-proxy": [ACT_TRY_AGAIN, ACT_TEST_CONNECTION, ACT_SAVE],
  "network-certificate": [ACT_CHOOSE_FILE, ACT_REMOVE],
  "mistral-key": [
    ACT_TEST,
    ACT_REPLACE,
    ACT_UNLOCK_IN_ACCOUNT,
    ACT_SET_UP_KEY,
    ACT_REPLACE_AND_TEST,
    ACT_CANCEL,
  ],
  "mistral-keys": [
    ACT_ADD_KEY,
    ACT_USE_THIS_KEY,
    ACT_SAVE,
    ACT_CANCEL,
    ACT_REMOVE_ELLIPSIS,
    ACT_REMOVE,
  ],
  "project-secrets": [ACT_ADD_ENTRY, ACT_CHOOSE_FOLDER],
  "add-entry": [ACT_GENERATE, ACT_OPTIONS, ACT_CANCEL],
  "secret-scan": [ACT_CHECK_NOW],
  "safety-custom": [ACT_ADD_RULE, ACT_REMOVE],
  "ask-allowed": [ACT_CLEAR_SEARCH, ACT_REVOKE],
  "allowed-sessions": [ACT_REVOKE],
  "allowed-earlier": [ACT_REVOKE],
  "allowed-vault": [ACT_REVOKE],
  "update-status": [
    ACT_DOWNLOAD,
    ACT_INSTALL_AND_RESTART,
    ACT_OPEN_RELEASE_PAGE,
    ACT_CHECK_FOR_UPDATES,
    ACT_UPDATE_VIBE,
  ],
  about: [ACT_SEND_FEEDBACK],
  "debug-token": [ACT_REGENERATE_TOKEN],
};

function stripEllipsis(text: string): string {
  return text.replace(/…+$/, "").trim();
}

/**
 * Whether `text` (already lower-cased) holds `word` at a word boundary, with singular fallback for trailing "s".
 */
export function holdsWordPrefix(text: string, word: string): boolean {
  const match = (w: string): boolean => {
    let i = text.indexOf(w);
    while (i !== -1) {
      const before = text[i - 1];
      if (before === undefined || /[\s/·.,()\-:+]/.test(before)) return true;
      i = text.indexOf(w, i + 1);
    }
    return false;
  };
  if (match(word)) return true;
  return word.length > 3 && word.endsWith("s") && match(word.slice(0, -1));
}

/**
 * Returns the action string in `actions` that best matches `query`, or `null` when none matches.
 */
export function matchedAction(
  actions: readonly string[] | undefined,
  query: string,
): string | null {
  if (actions === undefined || actions.length === 0) return null;
  const normalized = stripEllipsis(query).toLowerCase();
  if (normalized === "") return null;
  const words = normalized.split(/\s+/).filter((w) => w.length > 0);
  if (words.length === 0) return null;

  for (const action of actions) {
    if (stripEllipsis(action).toLowerCase() === normalized) return action;
  }
  for (const action of actions) {
    const low = action.toLowerCase();
    if (low.includes(normalized)) return action;
  }
  for (const action of actions) {
    const low = action.toLowerCase();
    if (words.every((w) => holdsWordPrefix(low, w))) return action;
  }
  return null;
}
