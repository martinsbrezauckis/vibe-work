/**
 * Updates: the UI side of the host's update flow (notify, download, then ask). The flow itself
 * (state machine, signature check, consent) lives in Rust (`vw_host::update_flow`); this module mirrors its view
 * in a store, turns it into the words the notice and the Settings section show, and sends the user's clicks.
 * Depends on: createStore, toastStore, protocol/client types. Called by: createApp (wiring), useShellWiring
 * (starts the stream), UpdateNotice, UpdateSettings, updatesRoutes.
 * Invariants:
 * - The UI never decides whether an update installs: "Install and restart" only asks the host, which refuses
 *   without a recorded agreement. While a Debug API UI-control request is served every call is marked with the
 *   `debug-api` origin (like the core controller does; a label for the host's log; no dialog).
 * - A build without an update channel (`subscribe` answers false) shows nothing: `available` stays false.
 * - "Later" hides the notice for the current state of one version only; a new state (downloaded, failed) shows
 *   it again.
 * - A failed check alone (offline) is not shown as a banner: only Settings mentions it.
 */
import type { CoreClient, ExecuteOptions, UpdateView } from "../protocol/client/types";
import { toCallError } from "../protocol/client/types";
import { createStore } from "./createStore";
import type { Toasts } from "./toastStore";

/** What the notice shows for a view, or `null` when it shows nothing. */
export interface NoticeModel {
  /** `ready` and `failed` ask for attention; the rest inform. */
  tone: "info" | "ask" | "error";
  headline: string;
  /** Short release notes or a hint. */
  detail: string | null;
  /** 0..1 while downloading with a known size; `null` otherwise. */
  progress: number | null;
  actions: readonly NoticeAction[];
  /** Identifies this state of this version, for "Later". */
  key: string;
}

/** A button of the notice or the Settings section. */
export type NoticeAction = "download" | "install" | "openReleasePage" | "retry" | "later";

const NOTES_MAX = 200;

/**
 * The host the "Open release page" button goes to, shown beside it: the user sees where the
 * system browser will open before clicking. The host builds the link from the official repository only
 * (`vw_host::update_flow::release_page_for`); `null` for no link or a value that is not an absolute URL.
 */
export function releasePageHost(releasePage: string | null): string | null {
  if (releasePage === null) return null;
  try {
    const host = new URL(releasePage).host;
    return host === "" ? null : host;
  } catch {
    return null;
  }
}

function firstNotes(notes: string | null): string | null {
  if (notes === null) return null;
  const line = notes
    .split("\n")
    .map((l) => l.trim())
    .find((l) => l !== "");
  if (line === undefined) return null;
  return line.length > NOTES_MAX ? `${line.slice(0, NOTES_MAX - 1)}…` : line;
}

/** The notice for `view`; plain words. `null` for idle, checking, a failed check, and the install in progress. */
export function noticeFor(view: UpdateView): NoticeModel | null {
  const phase = view.phase;
  const linkOnly = view.mode === "linkOnly";
  switch (phase.type) {
    case "available":
      return {
        tone: "info",
        headline: `Vibe Work ${phase.version} is available.`,
        detail: linkOnly
          ? "This copy was installed with a package manager, which also installs updates. You can read what is new on the release page."
          : (firstNotes(phase.notes) ?? "You chose to be asked before updates download."),
        progress: null,
        actions: linkOnly
          ? view.releasePage === null
            ? ["later"]
            : ["openReleasePage", "later"]
          : ["download", "later"],
        key: `available:${phase.version}`,
      };
    case "downloading":
      return {
        tone: "info",
        headline: `Downloading Vibe Work ${phase.version}…`,
        detail: "Nothing is installed until you say so.",
        progress:
          phase.total !== null && phase.total > 0
            ? Math.min(1, phase.received / phase.total)
            : null,
        actions: [],
        key: `downloading:${phase.version}`,
      };
    case "ready":
      return {
        tone: "ask",
        headline: `Vibe Work ${phase.version} is ready to install.`,
        detail:
          "Vibe Work will restart. Your sessions and unsent drafts are kept; a turn that is running is interrupted.",
        progress: null,
        actions: ["install", "later"],
        key: `ready:${phase.version}`,
      };
    case "installing":
      return {
        tone: "info",
        headline: `Installing Vibe Work ${phase.version}…`,
        detail: null,
        progress: null,
        actions: [],
        key: `installing:${phase.version}`,
      };
    case "restarting":
      return {
        tone: "info",
        headline: `Vibe Work ${phase.version} is installed. Restarting…`,
        detail: null,
        progress: null,
        actions: [],
        key: `restarting:${phase.version}`,
      };
    case "failed":
      if (phase.stage === "check") return null;
      return {
        tone: "error",
        headline:
          phase.stage === "download"
            ? "The update could not be downloaded."
            : "The update could not be installed.",
        detail: phase.error,
        progress: null,
        actions: ["retry", "later"],
        key: `failed:${phase.stage}:${phase.version ?? ""}`,
      };
    default:
      return null;
  }
}

/** One line for Settings: what is going on, including a failed check. */
export function statusLine(view: UpdateView): string {
  const phase = view.phase;
  switch (phase.type) {
    case "idle":
      return view.lastChecked === null
        ? `Vibe Work ${view.current}. Not checked yet in this run.`
        : `Vibe Work ${view.current} is up to date.`;
    case "checking":
      return "Checking for updates…";
    case "available":
      return `Version ${phase.version} is available.`;
    case "downloading":
      return `Downloading version ${phase.version}…`;
    case "ready":
      return `Version ${phase.version} is downloaded and ready to install.`;
    case "installing":
      return `Installing version ${phase.version}…`;
    case "restarting":
      return `Version ${phase.version} is installed. Restarting…`;
    case "failed":
      return `${phase.stage === "check" ? "Could not check for updates" : "Update failed"}: ${phase.error}`;
  }
}

/** The state the views read. */
export interface UpdatesState {
  /** This build has an update channel. */
  available: boolean;
  view: UpdateView | null;
  /** The notice key the user dismissed with "Later". */
  dismissed: string | null;
}

/** Creates the updates controller. */
export function createUpdates(options: {
  client: CoreClient;
  toasts: Toasts;
  /** The capability of the Debug API UI-control request being served, `null` when none is. */
  serving: () => string | null;
}) {
  const { client, toasts, serving } = options;
  const store = createStore<UpdatesState>({ available: false, view: null, dismissed: null });
  let started = false;

  const origin = (): ExecuteOptions | undefined => {
    const capability = serving();
    return capability === null ? undefined : { origin: "debug-api", capability };
  };

  async function run(step: () => Promise<void>): Promise<void> {
    try {
      await step();
    } catch (thrown) {
      const error = toCallError(thrown).error;
      // A declined native dialog is the user's answer, not a failure.
      if (error.code !== "confirmationDeclined")
        toasts.push({ kind: "error", bellKind: "failure", text: error.message });
      throw thrown;
    }
  }

  return {
    store,
    /** Starts the view stream once; resolves to whether this build has an update channel. */
    async start(): Promise<boolean> {
      if (started) return store.get().available;
      started = true;
      try {
        const available = await client.updates.subscribe((view) => {
          store.set((s) => ({ ...s, view }));
        });
        store.set((s) => ({ ...s, available }));
        return available;
      } catch {
        started = false;
        return false;
      }
    },
    /** "Later": hides the notice for its current state. */
    dismiss(): void {
      const view = store.get().view;
      const notice = view === null ? null : noticeFor(view);
      if (notice !== null) store.set((s) => ({ ...s, dismissed: notice.key }));
    },
    check: () => run(() => client.updates.check()),
    download: () => run(() => client.updates.download(origin())),
    install: () => run(() => client.updates.install(origin())),
    setAutoDownload: (enabled: boolean) => run(() => client.updates.setAutoDownload(enabled)),
    openReleasePage: () => run(() => client.updates.openReleasePage()),
  };
}

/** The updates controller of one app. */
export type Updates = ReturnType<typeof createUpdates>;
