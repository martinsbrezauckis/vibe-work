/**
 * Save open files before Vibe works (Settings › General, default on): before a message
 * goes out, every edited open file of the session is saved, so the file on disk is what the user sees.
 * Depends on: state/files (workspace), saveEditor. Called by: the composer's send hook (installed by register.tsx).
 * Invariants: a file whose disk version changed under the same lines (a conflict) is never saved this way; a failed
 * save never blocks the message; the answer is the note the composer shows after the send (null: nothing to say).
 */
import type { App } from "../shell/AppContext";
import { saveEditor } from "./saveEditor";

/**
 * Saves the session's edited open files; answers the composer's note for the ones that were not saved. `null` (at
 * once) when no open file has unsaved edits, so the message is not held back at all.
 */
export function saveOpenFiles(
  app: App,
  sessionId: string,
  origin: "ui" | "debug-api",
): Promise<string | null> | null {
  const files = app.files;
  if (files === null || !app.shell.store.get().saveBeforeSend) return null;
  const view = files.workspace.of(sessionId);
  const edited = view.order.flatMap((file) => {
    const doc = view.documents[file];
    const reader = files.document(sessionId, file);
    const draft = reader?.store.get().edits[sessionId];
    if (doc === undefined || doc.saved !== undefined || reader == null || draft === undefined)
      return [];
    return draft.saving || draft.text === draft.base.text ? [] : [{ reader, draft }];
  });
  if (edited.length === 0) return null;
  return (async () => {
    const skipped: string[] = [];
    const failed: string[] = [];
    for (const { reader, draft } of edited) {
      if (draft.conflict) {
        skipped.push(draft.base.name);
        continue;
      }
      const outcome = await saveEditor(app, sessionId, origin, reader);
      if (outcome.kind === "stale" || outcome.kind === "error") failed.push(draft.base.name);
    }
    const notes = [
      skipped.length > 0
        ? `${skipped.join(", ")} ${skipped.length === 1 ? "was not saved first: it changed on disk where you have unsaved edits; choose in its editor." : "were not saved first: they changed on disk where you have unsaved edits; choose in their editors."}`
        : null,
      failed.length > 0
        ? `${failed.join(", ")} could not be saved first; the unsaved edits are still in the editor.`
        : null,
    ].filter((n): n is string => n !== null);
    return notes.length === 0 ? null : `Sent. ${notes.join(" ")}`;
  })();
}
