/**
 * The Changes tab's controller: per-session views from the core, the selected file, the diff
 * layout, the pending confirmation, and every action as a typed `changes` command (or the existing
 * `rewindSession` for "undo turn"). Components render this store and call these actions only.
 * Depends on: state/coreStore (`run`, `listen`), state/createStore, model.ts, protocol types.
 * Called by: register.tsx, the views, the `ui/changes` route.
 * Invariants:
 * - A file is deleted only after a confirmation that names it: a revert that would delete asks first,
 *   and the core refuses (`confirmDeletes`) any request that does not name every deleted file.
 * - `stale` ("file changed") reloads the view and says so; nothing is retried behind the user's back.
 * - A `changesUpdated` event reloads a session the store already shows.
 */
import { toCallError } from "../../protocol/client/types";
import type {
  ChangesRequest,
  ChangesResult,
  ChangesView,
  FileChange,
  Hunk,
  InitPlan,
  Target,
  UndoPreview,
} from "../../protocol/core";
import type { CoreController } from "../../state/coreStore";
import { createStore } from "../../state/createStore";
import { pruneSessions } from "../../state/prune";
import { bulkDeletes, deletedByRevert, shownInFull } from "./model";

/** A confirmation waiting in the tab. */
export type Confirm =
  | { kind: "keepUnseen"; sessionId: string; file: FileChange }
  | { kind: "revertFile"; sessionId: string; file: FileChange; deletes: string[] }
  | { kind: "revertAll"; sessionId: string; fingerprint: string; deletes: string[]; files: number }
  | { kind: "undoTurn"; sessionId: string; turnNumber: number; preview: UndoPreview }
  | { kind: "history"; sessionId: string; plan: InitPlan };

/** The tab's state. */
export interface ChangesUi {
  views: Readonly<Record<string, ChangesView>>;
  busy: Readonly<Record<string, boolean>>;
  error: Readonly<Record<string, string | null>>;
  selected: Readonly<Record<string, string | null>>;
  layout: "split" | "unified";
  confirm: Confirm | null;
}

const initial: ChangesUi = {
  views: {},
  busy: {},
  error: {},
  selected: {},
  layout: "unified",
  confirm: null,
};

function put<T>(map: Readonly<Record<string, T>>, key: string, value: T): Record<string, T> {
  return { ...map, [key]: value };
}

/** Creates the controller for `core`. */
export function createChanges(core: CoreController) {
  const store = createStore<ChangesUi>(initial);

  const run = async (request: ChangesRequest): Promise<ChangesResult> => {
    const result = await core.run({ type: "changes", request });
    if (result.type !== "changes") throw new Error(`unexpected ${result.type} result`);
    return result.result;
  };
  const setView = (view: ChangesView) => {
    store.set((s) => ({
      ...s,
      views: put(s.views, view.sessionId, view),
      error: put(s.error, view.sessionId, null),
    }));
  };
  const setBusy = (sessionId: string, busy: boolean) => {
    store.set((s) => ({ ...s, busy: put(s.busy, sessionId, busy) }));
  };
  const fail = async (sessionId: string, thrown: unknown): Promise<void> => {
    const error = toCallError(thrown).error;
    if (error.code === "confirmDeletes") {
      const deletes = (error.summary ?? "").split("\n").filter((p) => p !== "");
      const view = store.get().views[sessionId];
      if (view !== undefined)
        store.set((s) => ({
          ...s,
          confirm: {
            kind: "revertAll",
            sessionId,
            fingerprint: view.fingerprint,
            deletes,
            files: view.files.length,
          },
        }));
      return;
    }
    // Vibe 2.26.0 refuses a rewind while its runtime still finishes work after a turn, also when the
    // session already shows idle (`_runtime.py:1386-1391`); it passes a few seconds later.
    const pending = error.message.includes("Runtime work is pending");
    const message =
      error.code === "stale"
        ? "File changed. Refreshed: check again."
        : pending
          ? "Vibe is still finishing the last turn. Try Undo again in a few seconds."
          : error.message;
    if (error.code === "stale") await controller.load(sessionId);
    store.set((s) => ({ ...s, error: put(s.error, sessionId, message) }));
  };
  /** Runs a request that answers with a view, with the busy flag and error handling. */
  const act = async (sessionId: string, request: ChangesRequest): Promise<void> => {
    setBusy(sessionId, true);
    try {
      const result = await run(request);
      if (result.kind === "view") setView(result.view);
    } catch (thrown) {
      await fail(sessionId, thrown);
    } finally {
      setBusy(sessionId, false);
    }
  };
  const hunkTarget = (file: FileChange, hunk: Hunk): Target => ({
    kind: "hunk",
    path: file.path,
    blob: file.blob,
    hunkId: hunk.id,
  });
  const fileTarget = (file: FileChange): Target => ({
    kind: "file",
    path: file.path,
    blob: file.blob,
  });

  // a deleted session's entries go with it.
  core.onSessionsGone((ids) => {
    store.set((s) => pruneSessions(s, ["views", "busy", "error", "selected"], ids));
  });
  const stopListening = core.listen((event) => {
    if (event.type === "changesUpdated" && store.get().views[event.sessionId] !== undefined)
      void controller.load(event.sessionId);
    // after `lagged` the core's events for this were missed: read again what is shown.
    if (event.type === "lagged")
      for (const id of Object.keys(store.get().views)) void controller.load(id);
  });

  const controller = {
    store,
    /** Reads (and refreshes) the session's changes. */
    async load(sessionId: string): Promise<void> {
      try {
        const result = await run({ op: "read", sessionId });
        if (result.kind === "view") setView(result.view);
      } catch (thrown) {
        store.set((s) => ({
          ...s,
          error: put(s.error, sessionId, toCallError(thrown).error.message),
        }));
      }
    },
    keepHunk: (sessionId: string, file: FileChange, hunk: Hunk) =>
      act(sessionId, { op: "keep", sessionId, target: hunkTarget(file, hunk) }),
    revertHunk: (sessionId: string, file: FileChange, hunk: Hunk) =>
      act(sessionId, { op: "revert", sessionId, target: hunkTarget(file, hunk) }),
    /** Keeps a file; one the user was not shown line by line (too large, binary) is kept only after a
     * confirmation that says so. */
    async keepFile(sessionId: string, file: FileChange, confirmed = false): Promise<void> {
      if (!shownInFull(file) && !confirmed) {
        store.set((s) => ({ ...s, confirm: { kind: "keepUnseen", sessionId, file } }));
        return;
      }
      await act(sessionId, { op: "keep", sessionId, target: fileTarget(file) });
    },
    /** Reverts a file; one the session created is deleted only after the confirmation names it. */
    async revertFile(sessionId: string, file: FileChange, confirmed = false): Promise<void> {
      const deletes = [deletedByRevert(file)].filter((p): p is string => p !== null);
      if (deletes.length > 0 && !confirmed) {
        store.set((s) => ({ ...s, confirm: { kind: "revertFile", sessionId, file, deletes } }));
        return;
      }
      await act(sessionId, {
        op: "revert",
        sessionId,
        target: fileTarget(file),
        deletePaths: deletes,
      });
    },
    keepAll(sessionId: string): Promise<void> {
      const view = store.get().views[sessionId];
      if (view === undefined) return Promise.resolve();
      return act(sessionId, {
        op: "keep",
        sessionId,
        target: { kind: "all", fingerprint: view.fingerprint },
      });
    },
    /** Asks before reverting everything (naming every file it deletes). */
    askRevertAll(sessionId: string): void {
      const view = store.get().views[sessionId];
      if (view === undefined || view.files.length === 0) return;
      store.set((s) => ({
        ...s,
        confirm: {
          kind: "revertAll",
          sessionId,
          fingerprint: view.fingerprint,
          deletes: bulkDeletes(view),
          files: view.files.length,
        },
      }));
    },
    /** Shows what undoing a turn restores and what it cannot (folders without git). */
    async askUndo(sessionId: string, turnId: string, turnNumber: number): Promise<void> {
      try {
        const result = await run({ op: "previewUndo", sessionId, turnId });
        if (result.kind === "undoPreview")
          store.set((s) => ({
            ...s,
            confirm: { kind: "undoTurn", sessionId, turnNumber, preview: result.preview },
          }));
      } catch (thrown) {
        await fail(sessionId, thrown);
      }
    },
    /** Shows what turning on version history commits. */
    async askHistory(sessionId: string): Promise<void> {
      try {
        const result = await run({ op: "planHistory", sessionId });
        if (result.kind === "initPlan")
          store.set((s) => ({ ...s, confirm: { kind: "history", sessionId, plan: result.plan } }));
      } catch (thrown) {
        await fail(sessionId, thrown);
      }
    },
    dismissOffer: (sessionId: string) => act(sessionId, { op: "dismissOffer", sessionId }),
    /** "Start reviewing from now": a new baseline, only on the user's click (after one was lost). */
    restart: (sessionId: string) => act(sessionId, { op: "restart", sessionId }),
    /** The user accepted the pending confirmation. */
    async accept(): Promise<void> {
      const pending = store.get().confirm;
      store.set((s) => ({ ...s, confirm: null }));
      if (pending === null) return;
      const id = pending.sessionId;
      switch (pending.kind) {
        case "keepUnseen":
          return controller.keepFile(id, pending.file, true);
        case "revertFile":
          return controller.revertFile(id, pending.file, true);
        case "revertAll":
          return act(id, {
            op: "revert",
            sessionId: id,
            target: { kind: "all", fingerprint: pending.fingerprint },
            deletePaths: pending.deletes,
          });
        case "history":
          return act(id, { op: "enableHistory", sessionId: id, confirmed: true });
        case "undoTurn":
          setBusy(id, true);
          try {
            await core.run({
              type: "rewindSession",
              sessionId: id,
              entryId: pending.preview.anchorEntryId,
              restoreFiles: true,
              inplace: true,
            });
            await controller.load(id);
          } catch (thrown) {
            await fail(id, thrown);
          } finally {
            setBusy(id, false);
          }
      }
    },
    /** The user declined the pending confirmation. */
    cancel(): void {
      store.set((s) => ({ ...s, confirm: null }));
    },
    select(sessionId: string, path: string): void {
      store.set((s) => ({ ...s, selected: put(s.selected, sessionId, path) }));
    },
    setLayout(layout: ChangesUi["layout"]): void {
      store.set((s) => ({ ...s, layout }));
    },
    stop(): void {
      stopListening();
    },
  };
  return controller;
}

/** The controller type. */
export type Changes = ReturnType<typeof createChanges>;
