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
 *   in the environment answers `notConfigured`, and then `availability` is
 *   `none`: no setup screen, no Settings section.
 * - `setupDone` (remembered in localStorage; the page works without it) records that a key worked on
 *   this computer. Startup still asks for the selected vault's password when it is locked; later
 *   locks use the chat notice and Account recovery without covering the sessions.
 */
import type {
  CommandResult,
  CoreEvent,
  KeyCandidateView,
  MistralKeyView,
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
type TestState =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "ok"; models: number }
  | { phase: "failed"; reason: string; text: string };

/** Everything the key screens read. */
export interface KeyUi {
  /** `loading` until the first status; `none` when this run has no key service. */
  availability: "loading" | "none" | "ready" | "failed";
  status: KeyStatusView | null;
  /** Mistral keys already in the open vault. */
  candidates: readonly KeyCandidateView[];
  /** Places where Vibe itself holds a key (metadata only). */
  vibeKeys: readonly VibeKeyView[];
  /** Saved model credentials, metadata only. */
  mistralKeys: readonly MistralKeyView[];
  switching: string | null;
  /** An action is running (buttons wait). */
  busy: boolean;
  /** The last failed action in plain words, with its reason word. */
  problem: { text: string; reason: string; operation?: KeyRequest["op"] } | null;
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
   * Holds setup while its test runs, fails or has not run, or a removable Vibe copy / rememberNote
   * needs a decision or reading. A working key with neither continues with one toast, so success
   * does not require an extra Continue click.
   */
  resultPending: boolean;
}

const INITIAL: KeyUi = {
  availability: "loading",
  status: null,
  candidates: [],
  vibeKeys: [],
  mistralKeys: [],
  switching: null,
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
    how: { key: string } | { importFrom: VibeKeySource },
  ): Promise<boolean>;
  unlock(password: string, vault?: VaultChoice): Promise<boolean>;
  saveKey(key: string): Promise<boolean>;
  addMistralKey(name: string, key: string): Promise<boolean>;
  useMistralKey(itemId: string): Promise<boolean>;
  renameMistralKey(itemId: string, name: string): Promise<boolean>;
  removeMistralKey(itemId: string): Promise<boolean>;
  useSaved(itemId: string): Promise<boolean>;
  importVibeKey(source: VibeKeySource): Promise<boolean>;
  removeVibeCopy(source: VibeKeySource): Promise<boolean>;
  testKey(): Promise<boolean>;
  lockNow(): Promise<boolean>;
  /** Turns "Lock the vault when Vibe Work quits" on or off (no password: nothing is re-wrapped). */
  setLockOnQuit(on: boolean): Promise<boolean>;
  dismiss(): void;
  reopen(): void;
  /** Closes the result view after setup (the sessions show again). */
  finish(): void;
  clearProblem(): void;
}

/** Creates the controller over `core`. */
export function createKeys(
  core: CoreController,
  notify: (text: string) => void = () => {},
): KeyController {
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
      if (request.op === "useMistralKey" && error.code === "confirmationDeclined") {
        notify("The Mistral key was kept.");
      } else {
        set({
          problem: { text: describeProblem(error), reason: reasonOf(error), operation: request.op },
        });
      }
      return null;
    } finally {
      running -= 1;
      if (running === 0) set({ busy: false });
    }
  };

  const readStatus = async (): Promise<KeyStatusView | null> => {
    try {
      const result = await core.run({ type: "keys", request: { op: "readStatus" } });
      if (result.type !== "keyStatus")
        throw new Error("The key status response was not understood.");
      return result.status;
    } catch (thrown) {
      const error = toCallError(thrown).error;
      const none = error.code === "keyService" && error.serverCode === "notConfigured";
      if (none) set({ availability: "none", status: null });
      else
        set({
          availability: "failed",
          status: null,
          problem: {
            text: `Could not read the Mistral key status. ${describeProblem(error)}`,
            reason: reasonOf(error),
            operation: "readStatus",
          },
        });
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
    let mistralKeys: readonly MistralKeyView[] = [];
    if (unlocked) {
      const saved = await core
        .run({ type: "keys", request: { op: "listMistralKeys" } })
        .catch(() => null);
      if (saved?.type === "mistralKeys") mistralKeys = saved.keys;
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
      ["loading", "failed"].includes(store.get().availability) &&
      status.vaultSelected &&
      step.kind === "unlock";
    store.set((s) => ({
      ...s,
      availability: "ready",
      problem: s.problem?.operation === "readStatus" ? null : s.problem,
      status,
      candidates,
      mistralKeys,
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
  const testKey = async (created = false): Promise<boolean> => {
    set({ test: { phase: "running" } });
    running += 1;
    set({ busy: true, problem: null });
    try {
      const result = await core.run({ type: "keys", request: { op: "testKey" } });
      if (result.type !== "keyTest") throw new Error("unexpected test result");
      set({ test: { phase: "ok", models: result.modelCount } });
      const s = store.get();
      if (s.resultPending && s.removable === null && s.status?.rememberNote == null) {
        // The toast replaces the result screen, so its one-time notice ("Created a new vault…") is said there and
        // must not linger into Settings > Mistral key later.
        set({ resultPending: false, dismissed: false, notice: null });
        notify(`${created ? "Vault created. " : ""}Key works.`);
      }
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
    // Record the import before testing: its removal offer must survive even a successful test.
    if (request.op === "importVibeKey") set({ removable: request.source });
    if (test && fromSetup) set({ resultPending: true });
    await load();
    if (test) await testKey();
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
    async createWithKey(password, how) {
      const created = await send({ op: "createVault", passphrase: password });
      if (created === null) return false;
      const stored = await send(
        "key" in how
          ? { op: "setupStoreKey", apiKey: how.key.trim() }
          : { op: "importVibeKey", source: how.importFrom },
      );
      if (stored !== null) {
        set({ resultPending: true, notice: "Created a new vault for Vibe Work." });
        if ("importFrom" in how) set({ removable: how.importFrom });
      }
      await load();
      if (stored === null) return false;
      await testKey(true);
      return true;
    },
    async unlock(password, vault) {
      const fromSetup = covering();
      const result = await send({
        op: "unlockVault",
        passphrase: password,
        ...(vault === undefined ? {} : { vault }),
      });
      if (result === null) return false;
      if (fromSetup) set({ resultPending: true });
      await load();
      // A vault that already holds the key: prove it works right away.
      if (fromSetup) {
        if (store.get().status?.keyState === "ready") await testKey();
        else set({ resultPending: false });
      }
      return true;
    },
    addMistralKey: (name, key) =>
      change({ op: "addMistralKey", name: name.trim(), apiKey: key.trim() }, false),
    async useMistralKey(itemId) {
      const label = store.get().mistralKeys.find((k) => k.itemId === itemId)?.label;
      set({ switching: itemId });
      try {
        const ok = await change({ op: "useMistralKey", itemId }, false);
        if (ok) {
          set({ test: { phase: "idle" } });
          notify(`Now using “${label ?? ""}”.`);
        }
        return ok;
      } finally {
        set({ switching: null });
      }
    },
    renameMistralKey: (itemId, name) =>
      change({ op: "renameMistralKey", itemId, name: name.trim() }, false),
    removeMistralKey: (itemId) => change({ op: "removeMistralKey", itemId }, false),
    saveKey: (key) => change({ op: "setupStoreKey", apiKey: key.trim() }, true),
    useSaved: (itemId) => change({ op: "useExistingKey", itemId }, true),
    importVibeKey: (source) => change({ op: "importVibeKey", source }, true),
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
    setLockOnQuit: (on) => change({ op: "setLockVaultOnQuit", on }, false),
    dismiss: () => {
      set({ dismissed: true });
    },
    finish: () => {
      // Leaving the result screen ends its notice too; Settings shows only notices of its own actions.
      set({ resultPending: false, dismissed: false, notice: null });
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
