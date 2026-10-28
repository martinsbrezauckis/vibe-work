/**
 * Files' folder actions before a session exists (History > Changes since the last commit): Review all changes,
 * Commit…, Revert all changes… with its confirmation, and Turn on version history with its plan and identity.
 * The state lives here, not in the component, so the `ui/explorer` route and the buttons run the same steps.
 * Depends on: the explorer store (its `run` sends the core's gitBrowser explore ops), state/git (the existing
 * Commit dialog), state/gitIdentity, createStore. Called by: WorkspaceActions, FilesPanel, explorerRoute.
 * Invariants:
 * - Revert runs only from the confirmation, with the head, fingerprint and new-file list of the review it shows;
 *   the core rechecks all three, so a folder that changed since is refused, never reverted blind.
 * - Turning on history needs `confirmed` and a valid identity; Cancel writes nothing.
 * - The state belongs to one folder: when the explorer shows another folder it starts empty (the component was
 *   keyed by folder before).
 */
import type { ChangesView, CommitIdentity, InitPlan } from "../../protocol/core";
import { createStore } from "../../state/createStore";
import { validIdentity } from "../../state/gitIdentity";
import type { App } from "../shell/AppContext";
import type { Explorer } from "./store";

/** The folder actions' state. */
interface WorkspaceState {
  folder: string | null;
  /** The open inline review (Review all changes), or the one the Revert confirmation lists. */
  review: { view: ChangesView; head: string } | null;
  /** The Revert all confirmation is open. */
  confirm: boolean;
  busy: boolean;
  error: string | null;
  /** The Turn on version history flow is open (its plan loads first). */
  init: boolean;
  plan: InitPlan | null;
  identity: CommitIdentity;
}

const EMPTY: WorkspaceState = {
  folder: null,
  review: null,
  confirm: false,
  busy: false,
  error: null,
  init: false,
  plan: null,
  identity: { name: "", email: "" },
};

/** The words shown when an action could not finish. */
const FAILED = "The action could not finish. Refresh the folder and try again.";

function createWorkspaceModel(app: App, explorer: Explorer) {
  const store = createStore<WorkspaceState>({ ...EMPTY, folder: explorer.store.get().folder });
  // A newer Turn on version history replaces the answers of an older one (folder switch, Cancel, reopen).
  let initCall = 0;
  explorer.store.subscribe(() => {
    const folder = explorer.store.get().folder;
    if (folder !== store.get().folder) {
      initCall++;
      store.set({ ...EMPTY, folder });
    }
  });
  const key = () => `files-workspace:${store.get().folder ?? ""}`;
  const run = async (work: () => Promise<void>) => {
    store.set((s) => ({ ...s, busy: true, error: null }));
    try {
      await work();
    } catch {
      store.set((s) => ({ ...s, error: FAILED }));
    } finally {
      store.set((s) => ({ ...s, busy: false }));
    }
  };
  const read = async () => {
    const folder = store.get().folder;
    if (!folder) return;
    const result = await explorer.run(folder, { op: "review" });
    if (result.kind === "review" && store.get().folder === folder)
      store.set((s) => ({ ...s, review: { view: result.view, head: result.head } }));
  };
  return {
    store,
    /** The Commit dialog's session key for this folder (`files-workspace:<folder>`). */
    key,
    /** Review all changes: the inline review of every change since the last commit. */
    review: () => run(read),
    /** Close review. */
    closeReview() {
      store.set((s) => ({ ...s, review: null }));
    },
    /** Commit…: the existing Commit dialog for this folder. */
    commit: () =>
      run(async () => {
        await app.git?.load(key());
        await app.git?.openCommit(key());
      }),
    /** Revert all changes…: reads a fresh review and opens the confirmation that lists it. */
    askRevert: () =>
      run(async () => {
        await read();
        if (store.get().review !== null) store.set((s) => ({ ...s, confirm: true }));
      }),
    /** The confirmation's Cancel: nothing is reverted. */
    cancelRevert() {
      store.set((s) => ({ ...s, confirm: false }));
    },
    /** The confirmation's Revert and delete, for exactly the review it lists. */
    confirmRevert: () =>
      run(async () => {
        const { folder, review, confirm } = store.get();
        if (!folder || review === null || !confirm) return;
        await explorer.run(folder, {
          op: "revert",
          head: review.head,
          fingerprint: review.view.fingerprint,
          deletePaths: review.view.files.filter((f) => f.status === "added").map((f) => f.path),
        });
        store.set((s) => ({ ...s, confirm: false, review: null }));
        explorer.refresh();
      }),
    /** Turn on version history…: opens the flow and reads its plan and the Git identity. */
    openInit() {
      const folder = store.get().folder;
      if (!folder || store.get().init) return;
      const call = ++initCall;
      store.set((s) => ({ ...s, init: true, plan: null, error: null }));
      const alive = () => call === initCall && store.get().folder === folder;
      void explorer
        .run(folder, { op: "planHistory" })
        .then((result) => {
          if (alive() && result.kind === "initPlan")
            store.set((s) => ({ ...s, plan: result.plan }));
        })
        .catch(() => {
          if (alive())
            store.set((s) => ({
              ...s,
              error: "This folder could not be read. Refresh it and try again.",
            }));
        });
      void app.core
        ?.run({ type: "git", request: { op: "identity" } })
        .then((answer) => {
          if (alive() && answer.type === "git" && answer.result.kind === "identity") {
            const identity = answer.result.settings.identity;
            store.set((s) => ({ ...s, identity }));
          }
        })
        .catch(() => undefined);
    },
    /** The plan dialog's Cancel (or its close): nothing is written. */
    closeInit() {
      initCall++;
      store.set((s) => ({ ...s, init: false, plan: null }));
    },
    /** The identity fields of the plan dialog. */
    setIdentity(identity: CommitIdentity) {
      store.set((s) => ({ ...s, identity }));
    },
    /** Turn on version history: creates the repository and the first commit the plan describes. */
    confirmInit: () =>
      run(async () => {
        const { folder, plan, identity } = store.get();
        if (!folder || plan === null || !validIdentity(identity)) return;
        await explorer.run(folder, { op: "enableHistory", confirmed: true, identity });
        initCall++;
        store.set((s) => ({ ...s, init: false, plan: null }));
        explorer.refresh();
      }),
  };
}

/** The folder-actions model. */
export type WorkspaceModel = ReturnType<typeof createWorkspaceModel>;

const models = new WeakMap<Explorer, WorkspaceModel>();

/** The one folder-actions model of `explorer` (created on first use; it lives as long as the explorer). */
export function workspaceModelOf(app: App, explorer: Explorer): WorkspaceModel {
  let model = models.get(explorer);
  if (model === undefined) {
    model = createWorkspaceModel(app, explorer);
    models.set(explorer, model);
  }
  return model;
}
