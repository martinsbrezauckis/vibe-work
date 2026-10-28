/** Changed-file references for the owning session. Depends on the file chip builder and composer facade; called by Git rows. */
import { term } from "../../vocabulary";
import { requestComposerFocus } from "../../state/composerFocus";
import { DRAWER_MAX } from "../../state/panelLayout";
import { UiControlError } from "../../state/uiControl";
import { absoluteFilePath, fileChip, sameFileChip } from "../files/model/context";
import type { App } from "../shell/AppContext";
import type { BrowserTarget } from "./model";

/** Read-only context insertion needs an admitted session, regardless of pending Git writes. */
export function filePointReason(target: BrowserTarget): string | null {
  if (target.restricted) return "This folder is outside the allowed project scope.";
  return target.sessionId == null || target.repo == null
    ? `${term("workNames", "chat").tooltip} there first`
    : null;
}

/** Adds the existing whole-file chip, with changed-file wording, and shows its session's composer. */
export function pointGitFile(app: App, target: BrowserTarget, path: string): void {
  const reason = filePointReason(target);
  if (reason !== null) throw new UiControlError("denied", reason);
  if (!target.files.some((file) => file.path === path))
    throw new UiControlError("notFound", "File is not in this node.");
  const sessionId = target.sessionId;
  const cwd = sessionId == null ? null : app.core?.store.get().sessions[sessionId]?.session["cwd"];
  if (sessionId == null || typeof cwd !== "string" || app.composer.actions === null)
    throw new UiControlError("denied", `${term("workNames", "chat").tooltip} there first`);
  const chip = fileChip({
    path: absoluteFilePath(path, target.path),
    project: cwd,
    name: path,
    unsaved: false,
  });
  chip.label += " · changed";
  chip.text += " It has uncommitted changes; see them with git diff.";
  const duplicate = (app.composer.store.get().context[sessionId] ?? []).some((item) =>
    sameFileChip(item, chip),
  );
  if (!duplicate) app.composer.actions.insertContext(sessionId, chip);
  app.composer.note(
    sessionId,
    `${duplicate ? "Already in" : "Added to"} your message: ${chip.label}`,
  );
  app.shell.actions.selectSession(sessionId);
  if (window.innerWidth <= DRAWER_MAX) app.shell.actions.closePanel();
  requestComposerFocus(sessionId);
}
