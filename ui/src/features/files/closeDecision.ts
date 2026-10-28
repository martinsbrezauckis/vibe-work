/**
 * The unsaved-changes close decision: what Cancel, Discard and "Save and close" do for a file whose tab is being
 * closed. One function for the dialog's buttons and the `ui/editors` route, so both act the same way.
 * Depends on: state/files (workspace), saveEditor. Called by: CloseFileDialog, files/editorsRoute.
 * Invariants:
 * - Discard drops only that document's draft, then closes it; nothing is written.
 * - Save and close writes through the editor's Save path (the stale check included). When the save is refused
 *   (the file changed on disk) or fails, the file stays open with its draft and the dialog says why; it never
 *   overwrites a newer file on disk.
 */
import type { Files } from "../../state/files";
import type { App } from "../shell/AppContext";
import { saveEditor } from "./saveEditor";

/** The dialog's three buttons. */
export type CloseChoice = "cancel" | "discard" | "save";

/** What a close decision did. */
export type CloseOutcome =
  | { kind: "closed" }
  | { kind: "cancelled" }
  /** Save and close did not close: the file stays open; `message` is the dialog's explanation. */
  | { kind: "kept"; message: string };

/** The dialog's words when the file changed on disk before Save and close. */
const STALE_CLOSE = "The file changed on disk. Review it in the editor before saving.";

/**
 * Applies `choice` to the unsaved-changes dialog of `file` in `sessionId`.
 * Side effects: closes the dialog (cancel, discard, a successful save) or records its problem (a kept save);
 * a save writes the file through the core with `origin`.
 */
export async function decideClose(
  app: App,
  files: Files,
  sessionId: string,
  file: string,
  choice: CloseChoice,
  origin: "ui" | "debug-api",
): Promise<CloseOutcome> {
  const reader = files.document(sessionId, file);
  const close = (): CloseOutcome => {
    files.workspace.close(sessionId, file);
    files.workspace.cancelClose(sessionId);
    return { kind: "closed" };
  };
  if (choice === "cancel") {
    files.workspace.cancelClose(sessionId);
    return { kind: "cancelled" };
  }
  if (choice === "discard") {
    reader?.edit.cancel(sessionId);
    return close();
  }
  const result = await saveEditor(app, sessionId, origin, reader ?? undefined);
  if (result.kind === "saved" || result.kind === "unchanged") return close();
  const message = result.kind === "stale" ? STALE_CLOSE : result.message;
  files.workspace.closeFailed(sessionId, message);
  return { kind: "kept", message };
}
