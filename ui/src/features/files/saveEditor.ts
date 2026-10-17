/**
 * Save an open editor draft through the existing saveEdit path. Depends on App/Files; called by
 * FileEditor and ui/files. A failed save keeps the buffer and exposes the newest disk version.
 */
import type { App } from "../shell/AppContext";
import { saveEdit, type SaveOutcome } from "./saveEdit";

export async function saveEditor(
  app: App,
  sessionId: string,
  origin: "ui" | "debug-api" = "ui",
): Promise<SaveOutcome> {
  const files = app.files;
  const draft = files?.store.get().edits[sessionId];
  if (files === null || draft === undefined)
    return { kind: "error", message: "No editor is open." };
  if (draft.saving) return { kind: "error", message: "A save is already running." };
  files.edit.update(sessionId, { saving: true, problem: null });
  const result = await saveEdit(app, sessionId, draft.base.text ?? "", draft.text, origin, {
    file: draft.base.file,
    fingerprint: draft.base.fingerprint,
    name: draft.base.name,
    display: draft.base.display,
  });
  // The original draft may have been pruned while the core write was in flight.
  if (files.store.get().edits[sessionId]?.id !== draft.id) return result;
  files.edit.update(sessionId, { saving: false });
  if (result.kind === "saved" || result.kind === "unchanged") files.edit.cancel(sessionId);
  else if (result.kind === "stale") {
    files.edit.update(sessionId, { conflict: true });
    await (origin === "debug-api" ? files.debugApi : files.actions).reload(sessionId);
  } else files.edit.update(sessionId, { problem: result.message });
  return result;
}
