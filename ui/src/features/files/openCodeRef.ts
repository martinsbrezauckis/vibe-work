/**
 * Code-chip navigation through the File tab's normal path checks. Depends on App and CodeRef;
 * called by composer and transcript chips. Existing file drafts retain their preview protection.
 */
import type { App } from "../shell/AppContext";
import type { CodeRef } from "./model/context";
import { absoluteFilePath } from "./model/context";
import { tabOf } from "../../state/files";

/** Opens source text through a previously checked handle or the session's normal path resolution. */
export async function openCodeRef(app: App, sessionId: string, ref: CodeRef): Promise<void> {
  if (app.files === null) return;
  app.shell.actions.openTab("file");
  try {
    const cwd = app.core?.store.get().sessions[sessionId]?.session["cwd"];
    const known =
      typeof cwd === "string"
        ? tabOf(app.files.store.get(), sessionId).recent.find(
            (file) => absoluteFilePath(file.display, cwd) === ref.path,
          )
        : undefined;
    // Like a verified file link, an existing handle is checked again by the core on read.
    // It also lets a deleted file reach the File tab's existing missing-file notice.
    const opened =
      known === undefined
        ? await app.files.actions.openText(sessionId, ref.path, ref.start || null, ref.end)
        : await app.files.actions
            .show(sessionId, { ...known, line: ref.start || null, end: ref.end })
            .then(() => true);
    if (opened) {
      if (app.find.store.get().file?.sessionId === sessionId) app.find.actions.close("file");
      const draft = app.files.store.get().edits[sessionId];
      if (draft !== undefined && !draft.saving && draft.text === draft.base.text)
        app.files.edit.cancel(sessionId);
      await app.files.actions.setMode(sessionId, "source");
    } else {
      app.files.actions.note(sessionId, "This file cannot be opened here.");
    }
  } catch (error: unknown) {
    app.toasts.push({
      kind: "warn",
      bellKind: "failure",
      text: error instanceof Error ? error.message : "Could not open the file.",
    });
  }
}
