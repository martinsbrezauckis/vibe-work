/**
 * The Mistral key controller: the UI's model of the key and the vault (setup, Settings, the inline
 * unlock prompt) and the actions that change them. It talks to the core only through the one-way
 * `keys` command: a secret goes in (password, key), only status, counts and metadata come back.
 * Depends on: coreStore (`CoreController`), createStore, keyText, keyView, protocol/core types.
 * Called by: createApp (creates), useShellWiring (starts), the setup/settings components, `ui/setup` route.
 * Invariants:
 * - A secret argument is passed straight into the command. It is never stored in this state, put in
 *   a message, logged or returned; the components clear their inputs when they submit.
 * - A failed action sets `problem` (plain words) and never throws to the caller; `true`/`false` tells
 *   the caller whether to clear its inputs.
 * - "Is there a key service at all?" is learned from the first `readStatus`: a run that keeps its key
 *   in the environment answers `notConfigured` (or the call fails), and then `availability` is
 *   `none`: no setup screen, no Settings section.
 * - `setupDone` (remembered in localStorage; the page works without it) records that a key worked on
 *   this computer. Startup still asks for the selected vault's password when it is locked; later
 *   locks use the chat notice and Account recovery without covering the sessions.
 */
import type {
  CommandResult,
  CoreEvent,
  KeyCandidateView,
  KeyRequest,
  KeyStatusView,
  VaultChoice,
  VibeKeySource,
  VibeKeyView,
} from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { type Store, createStore } from "./createStore";
import { describeProblem, reasonOf } from "./keyText";
import { type SetupStep, needsSetup, stepOf } from "./keyView";

const DONE_KEY = "vw.keySetupDone";

/** Result of the last key test. */
export type TestState =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "ok"; models: number }
  | { phase: "failed"; reason: string; text: string };

/** Everything the key screens read. */
export interface KeyUi {
  /** `loading` until the first status; `none` when this run has no key service. */
  availability: "loading" | "none" | "ready";
  status: KeyStatusView | null;
  /** Mistral keys already in the open vault. */
  candidates: readonly KeyCandidateView[];
  /** Places where Vibe itself holds a key (metadata only). */
  vibeKeys: readonly VibeKeyView[];
  /** An action is running (buttons wait). */
  busy: boolean;
  /** The last failed action in plain words, with its reason word. */
  problem: { text: string; reason: string } | null;
  test: TestState;
  /** Startup or a waiting request needs an unlock; `keyNeeded` can also request key setup. */
  need: "unlock" | "setup" | null;
  /** The user chose "Not now" on the setup screen. */
  dismissed: boolean;
  /** A key worked on this computer before. */
  setupDone: boolean;
  /** Startup found the selected vault locked; show its password form even after prior setup. */
  startupUnlock: boolean;
  /** Vibe's own copy that was just imported, which the user may now remove. */
  removable: VibeKeySource | null;
  /** A short confirmation ("Vibe's copy was removed."), or `null`. */
  notice: string | null;
  /**
   * The setup screen stays up after a flow it started until the user presses Continue, so the test
   * outcome ("Key works" or the 401 text) is read, not skipped.
   */
  resultPending: boolean;
}

const INITIAL: KeyUi = {
  availability: "loading",
  status: null,
  candidates: [],
  vibeKeys: [],
  busy: false,
  problem: null,
  test: { phase: "idle" },
  need: null,
  dismissed: false,
  setupDone: false,
  startupUnlock: false,
  removable: null,
  notice: null,
  resultPending: false,
};

function loadDone(): boolean {
  try {
    return localStorage.getItem(DONE_KEY) === "1";
  } catch {
    return false;
  }
}

function saveDone(): void {
  try {
    localStorage.setItem(DONE_KEY, "1");
  } catch {
    // Not remembered; the setup screen may show once more. The app keeps working.
  }
}

/** Does the setup screen cover the content area in state `s`? (Pure; components select it.) */
export function covers(s: KeyUi): boolean {
  return (
    s.availability === "ready" &&
    s.status !== null &&
    (s.resultPending ||
      (s.startupUnlock && !s.dismissed) ||
      needsSetup(stepOf(s.status), s.setupDone, s.dismissed))
  );
}

/** The controller. */
export interface KeyController {
  readonly store: Store<KeyUi>;
  /** The step the status calls for. */
  step(): SetupStep;
  /** Does the setup screen cover the content area now? */
  covers(): boolean;
  /** Reads the status. (Key events are listened to from creation, so none is missed before this.) */
  start(): Promise<void>;
  /** Re-reads status, saved keys and Vibe's key places. */
  load(): Promise<void>;
  /** First setup in one go: creates the vault, stores the key (typed or imported from Vibe), tests it. */
  createWithKey(
    password: string,
    remember: boolean,
    how: { key: string } | { importFrom: VibeKeySource },
  ): Promise<boolean>;
  unlock(password: string, vault?: VaultChoice, remember?: boolean): Promise<boolean>;
  saveKey(key: string): Promise<boolean>;
  useSaved(itemId: string): Promise<boolean>;
  importVibeKey(source: VibeKeySource): Promise<boolean>;
  removeVibeCopy(source: VibeKeySource): Promise<boolean>;
  testKey(): Promise<boolean>;
  lockNow(): Promise<boolean>;
  removeKey(): Promise<boolean>;
  /** Turns "remember this computer" on or off; needs the password (the core re-wraps the vault key). */
  setRemember(password: string, remember: boolean): Promise<boolean>;
  dismiss(): void;
  reopen(): void;
  /** Closes the result view after setup (the sessions show again). */
  finish(): void;
  clearProblem(): void;
}

/** Creates the controller over `core`. */
export function createKeys(core: CoreController): KeyController {
  const store = createStore<KeyUi>({ ...INITIAL, setupDone: loadDone() });
  const set = (patch: Partial<KeyUi>): void => {
    store.set((s) => ({ ...s, ...patch }));
  };
  let running = 0;

  /** Sends one request; `null` plus `problem` set when it fails. */
  const send = async (request: KeyRequest): Promise<CommandResult | null> => {
    running += 1;
    set({ busy: true, problem: null, notice: null });
    try {
      return await core.run({ type: "keys", request });
    } catch (thrown) {
      const error = toCallError(thrown).error;
      set({ problem: { text: describeProblem(error), reason: reasonOf(error) } });
      return null;
    } finally {
      running -= 1;
      if (running === 0) set({ busy: false });
    }
  };

  const readStatus = async (): Promise<KeyStatusView | null> => {
    try {
      const result = await core.run({ type: "keys", request: { op: "readStatus" } });
      return result.type === "keyStatus" ? result.status : null;
    } catch (thrown) {
      const error = toCallError(thrown).error;
      const none = error.code !== "keyService" || error.serverCode === "notConfigured";
      if (none) set({ availability: "none", status: null });
      else set({ problem: { text: describeProblem(error), reason: reasonOf(error) } });
      return null;
    }
  };

  const load = async (): Promise<void> => {
    const status = await readStatus();
    if (status === null) return;
    const step = stepOf(status);
    const unlocked = status.vaultState === "unlocked";
    const keyReady = status.keyState === "ready";
    let candidates: readonly KeyCandidateView[] = [];
    let vibeKeys: readonly VibeKeyView[] = [];
    if (unlocked) {
      const listed = await core
        .run({ type: "keys", request: { op: "listKeyCandidates" } })
        .catch(() => null);
      if (listed?.type === "keyCandidates") candidates = listed.candidates;
    }
    if (step.kind === "create" || step.kind === "pickVault" || (unlocked && !keyReady)) {
      const found = await core
        .run({ type: "keys", request: { op: "detectVibeKey" } })
        .catch(() => null);
      if (found?.type === "vibeKeys") vibeKeys = found.found;
    }
    if (keyReady) saveDone();
    // Local setup history cannot unlock the core's persisted selection. Ask on the first status,
    // before a request needs the key; an unsuccessful remembered unlock needs the same recovery.
    const startupUnlock =
      store.get().availability === "loading" && status.vaultSelected && step.kind === "unlock";
    store.set((s) => ({
      ...s,
      availability: "ready",
      status,
      candidates,
      vibeKeys,
      setupDone: s.setupDone || keyReady,
      startupUnlock: step.kind === "unlock" && (s.startupUnlock || startupUnlock),
      need: startupUnlock
        ? "unlock"
        : unlocked && (s.need === "unlock" || (keyReady && s.need === "setup"))
          ? null
          : s.need,
    }));
  };

  /** Runs the test call and records the outcome (the key is in use already). */
  const testKey = async (): Promise<boolean> => {
    set({ test: { phase: "running" } });
    running += 1;
    set({ busy: true, problem: null });
    try {
      const result = await core.run({ type: "keys", request: { op: "testKey" } });
      if (result.type !== "keyTest") throw new Error("unexpected test result");
      set({ test: { phase: "ok", models: result.modelCount } });
      return true;
    } catch (thrown) {
      const error = toCallError(thrown).error;
      set({ test: { phase: "failed", reason: reasonOf(error), text: describeProblem(error) } });
      return false;
    } finally {
      running -= 1;
      if (running === 0) set({ busy: false });
    }
  };

  const covering = (): boolean => covers(store.get());

  /** Runs `request`; on success reloads the status, optionally tests the key. */
  const change = async (request: KeyRequest, test: boolean): Promise<boolean> => {
    const fromSetup = covering();
    const result = await send(request);
    if (result === null) return false;
    await load();
    if (test) {
      await testKey();
      if (fromSetup) set({ resultPending: true });
    }
    return true;
  };

  // Listen from the start, not from the first render: a `keyNeeded` that arrives between the core's
  // subscription and the window mounting must not be lost. The listener lives as long as the app.
  core.listen((event: CoreEvent) => {
    if (event.type === "keyNeeded") {
      set({ need: event.reason === "setup" ? "setup" : "unlock", dismissed: false });
      void load();
    } else if (event.type === "keyStatusChanged" || event.type === "lagged") {
      // after `lagged` the core's events for this were missed: read again what is shown.
      void load();
    }
  });

  return {
    store,
    step: () => stepOf(store.get().status),
    covers: () => covering(),
    start: load,
    load,
    async createWithKey(password, remember, how) {
      const created = await send({
        op: "createVault",
        passphrase: password,
        rememberDevice: remember,
      });
      if (created === null) return false;
      const stored = await send(
        "key" in how
          ? { op: "setupStoreKey", apiKey: how.key.trim() }
          : { op: "importVibeKey", source: how.importFrom },
      );
      await load();
      if (stored === null) return false;
      if ("importFrom" in how) set({ removable: how.importFrom });
      await testKey();
      set({ resultPending: true, notice: "Created a new vault for Vibe Work." });
      return true;
    },
    async unlock(password, vault, remember) {
      const fromSetup = covering();
      const result = await send({
        op: "unlockVault",
        passphrase: password,
        ...(vault === undefined ? {} : { vault }),
        ...(remember === undefined ? {} : { rememberDevice: remember }),
      });
      if (result === null) return false;
      await load();
      // A vault that already holds the key: prove it works right away.
      if (store.get().status?.keyState === "ready" && fromSetup) {
        await testKey();
        set({ resultPending: true });
      }
      return true;
    },
    saveKey: (key) => change({ op: "setupStoreKey", apiKey: key.trim() }, true),
    useSaved: (itemId) => change({ op: "useExistingKey", itemId }, true),
    async importVibeKey(source) {
      const ok = await change({ op: "importVibeKey", source }, true);
      if (ok) set({ removable: source });
      return ok;
    },
    async removeVibeCopy(source) {
      const ok = await change({ op: "removeVibeKeyOriginal", source }, false);
      if (ok) set({ removable: null, notice: "Vibe's own copy of the key was removed." });
      return ok;
    },
    testKey,
    async lockNow() {
      const ok = await change({ op: "lockVault" }, false);
      if (ok) set({ test: { phase: "idle" } });
      return ok;
    },
    async removeKey() {
      const ok = await change({ op: "removeKey" }, false);
      if (ok) set({ test: { phase: "idle" }, notice: "The key was removed from Vibe Work." });
      return ok;
    },
    setRemember: (password, remember) =>
      change({ op: "unlockVault", passphrase: password, rememberDevice: remember }, false),
    dismiss: () => {
      set({ dismissed: true });
    },
    finish: () => {
      set({ resultPending: false, dismissed: false });
    },
    reopen: () => {
      set({ dismissed: false });
    },
    clearProblem: () => {
      set({ problem: null, notice: null });
    },
  };
}

/** The controller the app uses. */
export type Keys = ReturnType<typeof createKeys>;
