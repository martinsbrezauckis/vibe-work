/**
 * Rewind and branch from a user message: "Rewind to here" previews the files Vibe will
 * restore (`session/rewind/read`), then rewinds in place or into a new session (`session/rewind`);
 * "Branch" forks the conversation up to that message into a new session (`session/fork`, `attach:
 * false`) and opens it.
 * Depends on: coreStore, composerStore, shellStore, toastStore. Called by: createComposer, the user-row
 * buttons, the rewind dialog, `/rewind` and `/branch`, the `ui/composer` route.
 * Invariants:
 * - Anchors are our own message ids only (`model/session` `anchorProblem`), so the buttons
 *   are disabled for messages Vibe minted ids for.
 * - Nothing is rewound before the user confirmed the dialog; the dialog lists what Vibe reports, never
 *   a guess.
 * - A branch is `attach: false`: the source session keeps its channel and keeps running (the core
 *   leaves the channel with the source, `channels.rs`). A unified fork copies messages only, no steps
 *   and the new session shows them as plain messages.
 */
import type { JsonValue } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { ComposerStore } from "./composerStore";
import type { CoreController } from "./coreStore";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";

function field(value: JsonValue | undefined, key: string): JsonValue | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value[key]
    : undefined;
}

/** The rewind and branch actions. */
export function createRewindActions(deps: {
  core: CoreController;
  composer: ComposerStore;
  shell: Shell;
  toasts: Toasts;
}) {
  const { core, composer, shell, toasts } = deps;
  const error = (thrown: unknown) => toCallError(thrown).error.message;

  return {
    /** Opens the rewind dialog for `entryId` and loads Vibe's preview of the files to restore. */
    async openRewind(sessionId: string, entryId: string, text: string): Promise<void> {
      composer.setRewind({
        sessionId,
        entryId,
        text,
        loading: true,
        paths: [],
        hasFileChanges: false,
        error: null,
        busy: false,
      });
      try {
        const reply = await core.run({
          type: "call",
          method: "session/rewind/read",
          params: { sessionId, entryId },
        });
        const result = reply.type === "reply" ? reply.result : null;
        const paths = field(result, "paths");
        const current = composer.store.get().rewind;
        if (current?.entryId !== entryId) return;
        composer.setRewind({
          ...current,
          loading: false,
          hasFileChanges: field(result, "hasFileChanges") === true,
          paths: Array.isArray(paths)
            ? paths.filter((p): p is string => typeof p === "string")
            : [],
        });
      } catch (thrown) {
        const current = composer.store.get().rewind;
        if (current?.entryId === entryId) {
          composer.setRewind({ ...current, loading: false, error: error(thrown) });
        }
      }
    },
    closeRewind(): void {
      composer.setRewind(null);
    },
    /**
     * Rewinds as the open dialog says: in place (same session, the message goes back into the composer,
     * without closing the existing session) or into a new session (which is then opened).
     * Files are restored in both cases.
     */
    async confirmRewind(intoNewSession: boolean): Promise<boolean> {
      const dialog = composer.store.get().rewind;
      if (dialog === null || dialog.busy) return false;
      composer.setRewind({ ...dialog, busy: true, error: null });
      try {
        const result = await core.run({
          type: "rewindSession",
          sessionId: dialog.sessionId,
          entryId: dialog.entryId,
          restoreFiles: true,
          inplace: !intoNewSession,
        });
        const target = result.type === "session" ? result.snapshot.sessionId : dialog.sessionId;
        const detail = result.type === "session" ? result.detail : null;
        const message = field(detail, "message");
        const failed = field(detail, "restoreErrors");
        // Put back as text only (`literal`): the message may come from another client (CLI, Debug API)
        // and must never become a `!cmd` or `/command` by pressing Enter.
        if (typeof message === "string") composer.putBack(target, message);
        composer.setRewind(null);
        if (target !== dialog.sessionId) shell.actions.selectSession(target);
        if (Array.isArray(failed) && failed.length > 0)
          toasts.push({
            kind: "error",
            bellKind: "failure",
            text: `Rewound, but ${String(failed.length)} file(s) could not be restored.`,
          });
        return true;
      } catch (thrown) {
        const current = composer.store.get().rewind;
        if (current !== null) composer.setRewind({ ...current, busy: false, error: error(thrown) });
        return false;
      }
    },
    /** Branches the conversation at `entryId` into a new session and opens it. */
    async branch(sessionId: string, entryId: string): Promise<string | null> {
      try {
        const reply = await core.run({
          type: "call",
          method: "session/fork",
          params: { sourceSessionId: sessionId, entryId, attach: false },
        });
        const result = reply.type === "reply" ? reply.result : null;
        const id = field(field(field(result, "state"), "session"), "id");
        if (typeof id !== "string") throw new Error("Vibe did not return the new session.");
        // Load it into the store (a detached fork is stored by Vibe, the lobby can read it).
        await core.run({ type: "refreshSession", sessionId: id });
        shell.actions.selectSession(id);
        void core.refresh();
        return id;
      } catch (thrown) {
        toasts.push({
          kind: "error",
          bellKind: "failure",
          text: `Could not branch: ${error(thrown)}`,
        });
        return null;
      }
    },
  };
}

/** The rewind and branch actions. */
export type RewindActions = ReturnType<typeof createRewindActions>;
