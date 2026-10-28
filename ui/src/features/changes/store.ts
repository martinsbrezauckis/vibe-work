/**
 * The Changes tab's controller: per-session views from the core, the selected file, the diff
 * layout, the pending confirmation, and every action as a typed `changes` command (or the existing
 * `rewindSession` for "undo turn"). Components render this store and call these actions only.
 * Depends on: state/coreStore (`run`, `listen`), state/createStore, model.ts, protocol types.
 * Called by: register.tsx, the views, the `ui/changes` route.
 * Invariants:
 * - Every revert (a file, a block, all) asks first; a file is deleted only after a confirmation that
 *   names it, and the core refuses (`confirmDeletes`) any request that does not name every deleted file.
 * - `stale` ("file changed") reloads the view and says so; nothing is retried behind the user's back.
 * - A `changesUpdated` event reloads a session the store already shows.
 * - Completed reads render during ongoing edits; later refreshes never hold an earlier caller open.
 */
import { toCallError } from "../../protocol/client/types";
import type {
  ChangesRequest,
  ChangesResult,
  ChangesView,
  CommitIdentity,
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
import { restoreRewindReply, type RewindDestination } from "../../state/composerRewind";
import { validIdentity } from "../../state/gitIdentity";
import { summarize } from "../../state/sessionSummary";
import { reviewPath, type ReviewTarget } from "./reviewTarget";
import { createReadQueue } from "./readQueue";

/** A confirmation waiting in the tab. */
export type Confirm =
  | { kind: "keepUnseen"; sessionId: string; file: FileChange }
  | { kind: "revertFile"; sessionId: string; file: FileChange; deletes: string[] }
  | { kind: "revertHunk"; sessionId: string; file: FileChange; hunk: Hunk }
  | { kind: "revertAll"; sessionId: string; fingerprint: string; deletes: string[]; files: number }
  | { kind: "undoTurn"; sessionId: string; turnNumber: number; preview: UndoPreview }
  | {
      kind: "history";
      sessionId: string;
      plan: InitPlan;
      identity: CommitIdentity;
      needsIdentity: boolean;
    };

/** The tab's state. */
export interface ChangesUi {
  views: Readonly<Record<string, ChangesView>>;
  busy: Readonly<Record<string, boolean>>;
  error: Readonly<Record<string, string | null>>;
  selected: Readonly<Record<string, string | null>>;
  review: Readonly<Record<string, ReviewTarget | undefined>>;
  layout: "split" | "unified";
  confirm: Confirm | null;
  completion: Readonly<Record<string, "history" | "commit" | undefined>>;
}

const initial: ChangesUi = {
  views: {},
  busy: {},
  error: {},
  selected: {},
  review: {},
  layout: "unified",
  confirm: null,
  completion: {},
};

function put<T>(map: Readonly<Record<string, T>>, key: string, value: T): Record<string, T> {
  return { ...map, [key]: value };
}

/** Creates the controller for `core`. */
export function createChanges(core: CoreController, rewind: RewindDestination) {
  const store = createStore<ChangesUi>(initial);
  const repositories: Record<string, string | undefined> = {};
  const queueRead = createReadQueue();

  const run = async (request: ChangesRequest): Promise<ChangesResult> => {
    if (
      "sessionId" in request &&
      repositories[request.sessionId] !== undefined &&
      repositories[request.sessionId] !== "." &&
      request.op !== "readRepo" &&
      request.op !== "read"
    )
      throw new Error("Keep and Revert are available in the session's review.");
    const result = await core.run({ type: "changes", request });
    if (result.type !== "changes") throw new Error(`unexpected ${result.type} result`);
    return result.result;
  };
  const setView = (view: ChangesView) => {
    store.set((s) => {
      const target = s.review[view.sessionId];
      return {
        ...s,
        views: put(s.views, view.sessionId, view),
        completion:
          view.files.length > 0 ? put(s.completion, view.sessionId, undefined) : s.completion,
        error: put(s.error, view.sessionId, null),
        ...(target === undefined
          ? {}
          : {
              selected: put(s.selected, view.sessionId, reviewPath(view, target.path)),
              review: target.ready
                ? s.review
                : put(s.review, view.sessionId, { ...target, ready: true }),
            }),
      };
    });
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
    // A rewind can come back with "Runtime work is pending" while Vibe finishes the last turn; the
    // window then asks the user to try again in a few seconds.
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
      if (result.kind === "view") {
        setView(result.view);
        if (request.op === "enableHistory")
          store.set((s) => ({ ...s, completion: put(s.completion, sessionId, "history") }));
      }
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
    store.set((s) =>
      pruneSessions(s, ["views", "busy", "error", "selected", "review", "completion"], ids),
    );
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
    /** Records a successful commit only for an empty review. */
    markSaved(sessionId: string): void {
      if ((store.get().views[sessionId]?.files.length ?? 0) === 0)
        store.set((s) => ({ ...s, completion: put(s.completion, sessionId, "commit") }));
    },
    /** Reads (and refreshes) the session's changes. */
    load(sessionId: string): Promise<void> {
      const repo = repositories[sessionId];
      return queueRead(JSON.stringify([sessionId, repo ?? "."]), async () => {
        try {
          const result = await run(
            repo === undefined || repo === "."
              ? { op: "read", sessionId }
              : { op: "readRepo", sessionId, repo },
          );
          if (repositories[sessionId] === repo && result.kind === "view") setView(result.view);
        } catch (thrown) {
          if (repositories[sessionId] === repo)
            store.set((s) => ({
              ...s,
              error: put(s.error, sessionId, toCallError(thrown).error.message),
            }));
        }
      });
    },
    /** Follows the Git bar's confirmed repository; refreshes when its status is re-read. */
    selectRepository(sessionId: string, repo: string | undefined): void {
      if (repositories[sessionId] !== repo) {
        const changedFolder = (repositories[sessionId] ?? ".") !== (repo ?? ".");
        repositories[sessionId] = repo;
        store.set((s) => ({
          ...s,
          views: Object.fromEntries(Object.entries(s.views).filter(([id]) => id !== sessionId)),
          selected: put(s.selected, sessionId, null),
          review: put(s.review, sessionId, undefined),
          completion: changedFolder ? put(s.completion, sessionId, undefined) : s.completion,
          confirm: s.confirm?.sessionId === sessionId ? null : s.confirm,
        }));
      }
      void controller.load(sessionId);
    },
    keepHunk: (sessionId: string, file: FileChange, hunk: Hunk) =>
      act(sessionId, { op: "keep", sessionId, target: hunkTarget(file, hunk) }),
    /** Reverts one block of a file, after a short confirmation (`confirmed` is the confirmation's own call). */
    async revertHunk(
      sessionId: string,
      file: FileChange,
      hunk: Hunk,
      confirmed = false,
    ): Promise<void> {
      if (!confirmed) {
        store.set((s) => ({ ...s, confirm: { kind: "revertHunk", sessionId, file, hunk } }));
        return;
      }
      await act(sessionId, { op: "revert", sessionId, target: hunkTarget(file, hunk) });
    },
    /** Keeps a file; one the user was not shown line by line (too large, binary) is kept only after a
     * confirmation that says so. */
    async keepFile(sessionId: string, file: FileChange, confirmed = false): Promise<void> {
      if (!shownInFull(file) && !confirmed) {
        store.set((s) => ({ ...s, confirm: { kind: "keepUnseen", sessionId, file } }));
        return;
      }
      await act(sessionId, { op: "keep", sessionId, target: fileTarget(file) });
    },
    /** Reverts a file after a short confirmation; one the session created is deleted only after the
     * confirmation names it. */
    async revertFile(sessionId: string, file: FileChange, confirmed = false): Promise<void> {
      const deletes = [deletedByRevert(file)].filter((p): p is string => p !== null);
      if (!confirmed) {
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
        if (result.kind === "initPlan") {
          const identity = result.identity ?? { name: "", email: "" };
          store.set((s) => ({
            ...s,
            confirm: {
              kind: "history",
              sessionId,
              plan: result.plan,
              identity,
              needsIdentity: !validIdentity(identity),
            },
          }));
        }
      } catch (thrown) {
        await fail(sessionId, thrown);
      }
    },
    dismissOffer: (sessionId: string) => act(sessionId, { op: "dismissOffer", sessionId }),
    /** "Start reviewing from now": a new baseline, only on the user's click (after one was lost). */
    restart: (sessionId: string) => act(sessionId, { op: "restart", sessionId }),
    /** Edits only the pending init prompt; Cancel never saves an identity. */
    editHistoryIdentity(identity: CommitIdentity): void {
      store.set((s) =>
        s.confirm?.kind === "history" ? { ...s, confirm: { ...s.confirm, identity } } : s,
      );
    },
    /** The user accepted the pending confirmation. */
    async accept(): Promise<void> {
      const pending = store.get().confirm;
      if (pending?.kind === "history" && !validIdentity(pending.identity)) return;
      store.set((s) => ({ ...s, confirm: null }));
      if (pending === null) return;
      const id = pending.sessionId;
      switch (pending.kind) {
        case "keepUnseen":
          return controller.keepFile(id, pending.file, true);
        case "revertFile":
          return controller.revertFile(id, pending.file, true);
        case "revertHunk":
          return controller.revertHunk(id, pending.file, pending.hunk, true);
        case "revertAll":
          return act(id, {
            op: "revert",
            sessionId: id,
            target: { kind: "all", fingerprint: pending.fingerprint },
            deletePaths: pending.deletes,
          });
        case "history":
          return act(id, {
            op: "enableHistory",
            sessionId: id,
            confirmed: true,
            identity: pending.identity,
          });
        case "undoTurn":
          setBusy(id, true);
          try {
            const result = await core.run({
              type: "rewindSession",
              sessionId: id,
              entryId: pending.preview.anchorEntryId,
              restoreFiles: true,
              inplace: true,
            });
            restoreRewindReply(result, rewind);
            await controller.load(id);
          } catch (thrown) {
            await fail(id, thrown);
            rewind.toasts.push({
              kind: "error",
              bellKind: "failure",
              text: `Could not undo turn: ${store.get().error[id] ?? "Restoring files failed."}`,
            });
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
      store.set((s) => ({
        ...s,
        selected: put(s.selected, sessionId, path),
        review: put(s.review, sessionId, undefined),
      }));
    },
    /** Subagent edits are reviewed in their parent's Changes, alongside the parent's own edits. */
    owner(sessionId: string): string {
      const session = core.store.get().sessions[sessionId];
      return session === undefined ? sessionId : (summarize(session).parentId ?? sessionId);
    },
    /** Selects a transcript change after refreshing the session's review. */
    async review(sessionId: string, path: string, line: number): Promise<void> {
      if (repositories[sessionId] !== undefined) repositories[sessionId] = ".";
      store.set((s) => ({ ...s, review: put(s.review, sessionId, { path, line, ready: false }) }));
      await controller.load(sessionId);
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
