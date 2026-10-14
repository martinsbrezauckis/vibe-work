/**
 * Saving a quick edit, shared by the editor's Save and the `ui/files edit` route: the core's
 * `files write` (conflict-checked), then the context chip that tells the agent.
 * Depends on: AppContext types, state/files, model/editNote. Called by: FileEditor, filesRoutes.
 * Invariants: the chip is inserted only after the file was saved; a Debug API save marks the chip as the Debug
 * API's (composer provenance).
 */
import type { App } from "../shell/AppContext";
import { editDiff, editNoteText } from "./model/editNote";

/** What a save did. */
export type SaveOutcome =
  | { kind: "saved"; changed: number }
  | { kind: "unchanged" }
  | { kind: "stale" }
  | { kind: "error"; message: string };

/** Saves `text` over the file `sessionId`'s File tab shows (read as `original`). */
export async function saveEdit(
  app: App,
  sessionId: string,
  original: string,
  text: string,
  origin: "ui" | "debug-api" = "ui",
  base?: { file: string; fingerprint: string; name: string; display: string },
): Promise<SaveOutcome> {
  const files = app.files;
  if (files === null) return { kind: "error", message: "Vibe Work is not connected." };
  if (text === original) return { kind: "unchanged" };
  const tab = files.store.get().tabs[sessionId];
  // The edit's own file when it pinned one (the tab may show another file by now).
  const name = base?.name ?? tab?.content?.name ?? "the file";
  const display = base?.display ?? tab?.shown?.display ?? name;
  const actions = origin === "debug-api" ? files.debugApi : files.actions;
  const result = await actions.write(
    sessionId,
    text,
    base === undefined ? undefined : { file: base.file, fingerprint: base.fingerprint },
  );
  if (result.kind !== "saved") return result;
  const { changed } = editDiff(original, text);
  const composer = origin === "debug-api" ? app.composer.debugApi : app.composer.actions;
  composer?.insertContext(sessionId, {
    label: `Your edit: ${name}`,
    source: "edit",
    text: editNoteText(display, original, text),
  });
  return { kind: "saved", changed };
}
