/**
 * Saving a quick edit, shared by the editor's Save and the `ui/files edit` route: the core's
 * `files write` (conflict-checked; the core also moves the edit into the session's Changes baseline).
 * Depends on: AppContext types, state/files, model/editNote. Called by: saveEditor, filesRoutes.
 * Invariants: a save puts nothing into the composer: Vibe's own edit tool reads the file from disk, and
 * "Point Vibe at this" is the explicit way to tell it about a change.
 */
import type { FileReader } from "../../state/files";
import type { App } from "../shell/AppContext";
import { editDiff } from "./model/editNote";

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
  base?: { file: string; fingerprint: string },
  reader?: FileReader,
): Promise<SaveOutcome> {
  const files = reader ?? (base ? app.files?.document(sessionId, base.file) : null) ?? app.files;
  if (files === null) return { kind: "error", message: "Vibe Work is not connected." };
  if (text === original) return { kind: "unchanged" };
  const actions = origin === "debug-api" ? files.debugApi : files.actions;
  const result = await actions.write(
    sessionId,
    text,
    base === undefined ? undefined : { file: base.file, fingerprint: base.fingerprint },
  );
  if (result.kind !== "saved") return result;
  const { changed } = editDiff(original, text);
  return { kind: "saved", changed };
}
