/**
 * The diff line comments' controller: the session's comments as the core placed them, the open
 * draft (the rows being commented), and the actions as typed `lineComments` commands. "Send to Vibe" puts
 * context chips into the composer through the one send path (`pages/point.ts` `insertChip`) and marks the
 * comments sent; the user sends the message.
 * Depends on: state/coreStore, state/createStore, changes/store (the current diff), pages/point + pages/model
 * (chip send path), lineCommentModel, protocol types. Called by: register.tsx, the row/thread components,
 * commentRoutes.ts.
 * Invariants:
 * - Comments are placed by the core (content anchors); this store never guesses a place.
 * - A comment is added only with a hunk id and row indexes; the core takes the quoted lines from its own diff.
 * - Reloads follow the diff: a new diff fingerprint or a `lineCommentsChanged` event reloads the shown session.
 *   A load that fails leaves the old list and the error text (the tab still works without comments).
 */
import { toCallError } from "../../protocol/client/types";
import type {
  DiffLine,
  LineCommentView,
  LineCommentsRequest,
  LineCommentsResult,
} from "../../protocol/core";
import type { CoreController } from "../../state/coreStore";
import { createStore } from "../../state/createStore";
import { pruneSessions } from "../../state/prune";
import type { App } from "../shell/AppContext";
import { insertChip } from "../pages/point";
import { commentChip, rangeBetween } from "./lineCommentModel";
import type { Changes } from "./store";

/** The rows being commented. */
export interface Draft {
  sessionId: string;
  path: string;
  hunkId: string;
  /** The row the user started at (a shift-click extends from here). */
  anchor: number;
  from: number;
  to: number;
}

/** The store's state. */
export interface LineCommentsUi {
  comments: Readonly<Record<string, readonly LineCommentView[]>>;
  draft: Draft | null;
  error: Readonly<Record<string, string | null>>;
}

/** Who is acting: the window, or an external client through a `ui/changes` route request. */
export type Origin = "ui" | "debug-api";

/** Creates the controller. */
export function createLineComments(app: App, core: CoreController, changes: Changes) {
  const store = createStore<LineCommentsUi>({ comments: {}, draft: null, error: {} });
  const run = async (request: LineCommentsRequest): Promise<LineCommentsResult> => {
    const result = await core.run({ type: "lineComments", request });
    if (result.type !== "lineComments") throw new Error(`unexpected ${result.type} result`);
    return result.result;
  };
  const setError = (sessionId: string, text: string | null) => {
    store.set((s) => ({ ...s, error: { ...s.error, [sessionId]: text } }));
  };
  const known = new Set<string>();
  let reloading: Record<string, boolean> = {};

  const controller = {
    store,

    /** Reads the session's comments with their places in the current diff. */
    async load(sessionId: string): Promise<void> {
      known.add(sessionId);
      if (reloading[sessionId] === true) return;
      reloading = { ...reloading, [sessionId]: true };
      try {
        const result = await run({ op: "list", sessionId });
        if (result.type === "list") {
          store.set((s) => ({
            ...s,
            comments: {
              ...s.comments,
              [sessionId]: result.comments.map((view) => ({
                ...view,
                comment: {
                  ...view.comment,
                  // Sent is permanent; an older list can finish after markSent's successful reply.
                  sentMs:
                    view.comment.sentMs ??
                    s.comments[sessionId]?.find(
                      (previous) => previous.comment.id === view.comment.id,
                    )?.comment.sentMs ??
                    null,
                },
              })),
            },
            error: { ...s.error, [sessionId]: null },
          }));
        }
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      } finally {
        reloading = { ...reloading, [sessionId]: false };
      }
    },

    /** Starts (or, with `extend`, widens) a draft at row `index` of a hunk with `hunkLines` rows. */
    begin(
      sessionId: string,
      path: string,
      hunkId: string,
      index: number,
      hunkLines: number,
      extend: boolean,
    ): void {
      const open = store.get().draft;
      const same = open !== null && open.hunkId === hunkId && open.path === path;
      const anchor = extend && same ? open.anchor : index;
      const range = rangeBetween(anchor, index, hunkLines);
      store.set((s) => ({ ...s, draft: { sessionId, path, hunkId, anchor, ...range } }));
    },

    /** Closes the draft without saving. */
    cancel(): void {
      store.set((s) => ({ ...s, draft: null }));
    },

    /** Saves the draft's comment; with `andSend` also puts it into the composer. Returns the new id or null. */
    async save(text: string, andSend: boolean, origin: Origin = "ui"): Promise<string | null> {
      const draft = store.get().draft;
      if (draft === null) return null;
      try {
        const result = await run({
          op: "add",
          sessionId: draft.sessionId,
          path: draft.path,
          hunkId: draft.hunkId,
          from: draft.from,
          to: draft.to,
          text,
        });
        if (result.type !== "added") return null;
        const added = result.comment;
        store.set((s) => ({
          ...s,
          draft: null,
          comments: {
            ...s.comments,
            [draft.sessionId]: [...(s.comments[draft.sessionId] ?? []), added],
          },
          error: { ...s.error, [draft.sessionId]: null },
        }));
        if (andSend) await controller.send(draft.sessionId, [added.comment.id], origin);
        return added.comment.id;
      } catch (thrown) {
        const error = toCallError(thrown).error;
        setError(draft.sessionId, error.message);
        if (error.code === "stale") void changes.load(draft.sessionId);
        return null;
      }
    },

    /** Deletes a comment. */
    async remove(sessionId: string, id: string): Promise<boolean> {
      try {
        await run({ op: "remove", sessionId, id });
        store.set((s) => ({
          ...s,
          comments: {
            ...s.comments,
            [sessionId]: (s.comments[sessionId] ?? []).filter((c) => c.comment.id !== id),
          },
        }));
        return true;
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
        return false;
      }
    },

    /** The quoted lines of an attached comment in the diff as the tab shows it now (`null`: not attached). */
    currentLines(sessionId: string, view: LineCommentView): DiffLine[] | null {
      if (view.state !== "attached") return null;
      const file = changes.store
        .get()
        .views[sessionId]?.files.find((f) => f.path === view.comment.path);
      const hunk = file?.hunks.find((h) => h.id === view.hunkId);
      return hunk === undefined ? null : hunk.lines.slice(view.from, view.to + 1);
    },

    /**
     * Puts comments `ids` into the session's composer, one chip each (one message when the user sends), and
     * marks them sent. Returns how many chips were inserted.
     */
    async send(sessionId: string, ids: readonly string[], origin: Origin = "ui"): Promise<number> {
      const all = store.get().comments[sessionId] ?? [];
      const inserted: string[] = [];
      for (const view of all.filter((c) => ids.includes(c.comment.id))) {
        const chip = commentChip(view, controller.currentLines(sessionId, view));
        const outcome = insertChip(app, sessionId, chip, origin);
        if (!outcome.ok) {
          setError(sessionId, outcome.reason);
          break;
        }
        inserted.push(view.comment.id);
      }
      if (inserted.length > 0) {
        try {
          await run({ op: "markSent", sessionId, ids: inserted });
          const sentMs = Date.now();
          store.set((s) => ({
            ...s,
            comments: {
              ...s.comments,
              [sessionId]: (s.comments[sessionId] ?? []).map((view) =>
                inserted.includes(view.comment.id)
                  ? { ...view, comment: { ...view.comment, sentMs: view.comment.sentMs ?? sentMs } }
                  : view,
              ),
            },
          }));
          await controller.load(sessionId);
        } catch (thrown) {
          setError(sessionId, toCallError(thrown).error.message);
        }
      }
      return inserted.length;
    },

    /** Stops following events. */
    stop(): void {
      stopEvents();
      stopView();
    },
  };

  // a deleted session's entries go with it.
  core.onSessionsGone((ids) => {
    for (const id of ids) known.delete(id);
    store.set((s) => {
      const pruned = pruneSessions(s, ["comments", "error"], ids);
      return pruned.draft !== null && ids.includes(pruned.draft.sessionId)
        ? { ...pruned, draft: null }
        : pruned;
    });
  });
  const stopEvents = core.listen((event) => {
    if (event.type === "lineCommentsChanged" && known.has(event.sessionId)) {
      void controller.load(event.sessionId);
    }
    // after `lagged` the core's events for this were missed: read again what is shown.
    if (event.type === "lagged") for (const id of known) void controller.load(id);
  });
  // The diff moved on (edits, keep, revert): the places may have changed.
  const prints: Record<string, string> = {};
  const stopView = changes.store.subscribe(() => {
    for (const [id, view] of Object.entries(changes.store.get().views)) {
      if (prints[id] === view.fingerprint) continue;
      prints[id] = view.fingerprint;
      if (known.has(id)) void controller.load(id);
    }
  });
  return controller;
}

/** The controller type. */
export type LineComments = ReturnType<typeof createLineComments>;
