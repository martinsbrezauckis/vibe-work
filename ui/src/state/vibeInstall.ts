/**
 * The Vibe install at setup: the UI's model of the install view and the actions the setup screen
 * takes. Vibe Work ships no Vibe; with the user's consent the core installs the pinned Vibe from PyPI into an
 * app-private folder (or the user picks an install they already have). Everything goes through the one `vibe`
 * command; progress arrives as `vibeInstall` core events.
 * Depends on: coreStore (`CoreController`), createStore, protocol/core types. Called by: createApp (creates),
 * useShellWiring (starts), the setup screen (`VibeInstallStep`), Content (`vibeCovers`), the `ui/setup` route.
 * The Vibe update actions (`prepareUpdate` makes the offer the dialog shows, `confirmUpdate` is the
 * user's click in it, `cancelUpdate`) and "Later" for the Vibe notice (`dismissed`).
 * Invariants: the view is the core's (never edited here); a failed action sets `problem` and never throws;
 * "is there an installer at all?" is learned from the first `readStatus` (`notConfigured` means a development
 * build that starts its own server: no install step).
 */
import type { CoreEvent, InstallView, VibeRequest } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { type Store, createStore } from "./createStore";

/** Everything the install step reads. */
export interface VibeUi {
  /** `loading` until the first status; `none` when this run installs nothing. */
  availability: "loading" | "none" | "ready";
  view: InstallView | null;
  /** A request is on its way (buttons wait). */
  busy: boolean;
  /** The last refused request in plain words, with its reason word. */
  problem: { text: string; reason: string } | null;
  /** The Vibe notice key the user dismissed with "Later". */
  dismissed: string | null;
}

const INITIAL: VibeUi = {
  availability: "loading",
  view: null,
  busy: false,
  problem: null,
  dismissed: null,
};

/** Does the install step cover the content area in state `s`? (Pure; components select it.) */
export function vibeCovers(s: VibeUi): boolean {
  return s.availability === "ready" && s.view !== null && s.view.phase.state !== "installed";
}

/** The setup screen's heading for the install: Update when an app update replaces an older app-owned Vibe. */
export function installTitle(view: InstallView | null): string {
  return view?.replacing === undefined ? "Install Vibe" : "Update Vibe";
}

/**
 * The status bar's words while the server is not installed: an install that runs says so (the replaced version is
 * named after an app update), else Vibe is not installed yet. Null when no install runs.
 */
export function installStatus(view: InstallView | null): string | null {
  if (view?.phase.state !== "installing") return null;
  return view.replacing === undefined
    ? "Installing Vibe…"
    : `Updating Vibe ${view.replacing} to ${view.pinnedVersion}…`;
}

/** The controller. */
export interface VibeInstall {
  readonly store: Store<VibeUi>;
  /** Reads the view (events are listened to from creation). */
  start(): Promise<void>;
  /** A fresh offer to review (also "Try again"). */
  prepare(): Promise<boolean>;
  /** The user agrees to the offer on screen. */
  confirm(offerId: string): Promise<boolean>;
  cancel(): Promise<boolean>;
  checkExisting(): Promise<boolean>;
  useExisting(path: string): Promise<boolean>;
  /** The offer for the available Vibe update (the dialog shows it). */
  prepareUpdate(): Promise<boolean>;
  /** The user's click on "Update Vibe" in that dialog. */
  confirmUpdate(offerId: string): Promise<boolean>;
  /** Closes the dialog (withdraws the offer) or stops a running update. */
  cancelUpdate(): Promise<boolean>;
  /** "Later" on the Vibe notice: hides it for its current state. */
  dismiss(key: string): void;
}

/** Creates the controller over `core`. */
export function createVibeInstall(core: CoreController): VibeInstall {
  const store = createStore<VibeUi>(INITIAL);
  const set = (patch: Partial<VibeUi>): void => {
    store.set((s) => ({ ...s, ...patch }));
  };

  const send = async (request: VibeRequest): Promise<boolean> => {
    set({ busy: true, problem: null });
    try {
      const result = await core.run({ type: "vibe", request });
      if (result.type === "vibeInstall") set({ view: result.view, availability: "ready" });
      return true;
    } catch (thrown) {
      const error = toCallError(thrown).error;
      if (error.code === "vibeInstall" && error.serverCode === "notConfigured") {
        set({ availability: "none", view: null });
      } else {
        set({ problem: { text: error.message, reason: error.serverCode ?? error.code } });
      }
      return false;
    } finally {
      set({ busy: false });
    }
  };

  core.listen((event: CoreEvent) => {
    if (event.type === "vibeInstall") set({ view: event.view, availability: "ready" });
  });

  return {
    store,
    start: async () => {
      await send({ op: "readStatus" });
    },
    prepare: () => send({ op: "prepare" }),
    confirm: (offerId) => send({ op: "confirm", offerId }),
    cancel: () => send({ op: "cancel" }),
    checkExisting: () => send({ op: "checkExisting" }),
    useExisting: (path) => send({ op: "useExisting", path }),
    prepareUpdate: () => send({ op: "prepareUpdate" }),
    confirmUpdate: (offerId) => send({ op: "confirmUpdate", offerId }),
    cancelUpdate: () => send({ op: "cancelUpdate" }),
    dismiss: (key) => {
      set({ dismissed: key });
    },
  };
}
