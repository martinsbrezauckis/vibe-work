/** Reuses admitted session Git and File actions. Depends on app stores; called by the tree and Debug route. */
import { TEXT } from "../../vocabulary";
import type { App } from "../shell/AppContext";
import { UiControlError } from "../../state/uiControl";
import { actionStep, type BrowserAction, type BrowserTarget } from "./model";

/**
 * Runs a git browser row action: opens the target's chat, starts a new chat or task in its folder, or selects the
 * repository in the chat's Changes tab and starts its commit, push, pull request or bring-into-base step. Throws
 * {@link UiControlError} when the action is not allowed or the target has no chat.
 */
export async function adapterAction(
  app: App,
  target: BrowserTarget,
  action: BrowserAction,
  origin: "ui" | "debug-api",
): Promise<void> {
  const reason = actionStep(target, action).reason;
  if (reason !== null) throw new UiControlError("denied", reason);
  if (action === "open-chat" && target.sessionId) {
    app.shell.actions.selectSession(target.sessionId);
    return;
  }
  if (action === "new-chat" || action === "new-task") {
    if (app.worktrees === null)
      throw new UiControlError("serverUnavailable", `${TEXT.newChat} is unavailable.`);
    app.shell.actions.selectSession(null);
    await app.worktrees.newChatHere(target.path, action === "new-task", origin);
    return;
  }
  const sessionId = target.sessionId;
  const repo = target.repo;
  const git = app.git;
  if (
    sessionId === null ||
    sessionId === undefined ||
    repo === null ||
    repo === undefined ||
    git === null
  )
    throw new UiControlError("denied", "Open a chat in this folder to use Git actions.");
  app.shell.actions.selectSession(sessionId);
  app.shell.actions.openTab("changes");
  await git.load(sessionId);
  await git.selectRepo(sessionId, repo);
  const chosen = git.store.get();
  if (
    (chosen.selected[sessionId] ?? chosen.status[sessionId]?.repo) !== repo ||
    chosen.status[sessionId]?.repo !== repo
  )
    throw new UiControlError("denied", "Repository selection changed. Refresh the Git tree.");
  if (action === "review") return;
  if (action === "commit") await git.openCommit(sessionId);
  else if (action === "push") await git.push(sessionId);
  else if (action === "pr") await git.openPr(sessionId);
  else await git.openBring(sessionId);
}

/**
 * Opens a changed file of `target` in its chat's File tab. Throws {@link UiControlError} when the file is not in the
 * target, the target has no chat, or the file is outside the chat's readable folder.
 */
export async function adapterFile(
  app: App,
  target: BrowserTarget,
  path: string,
  origin: "ui" | "debug-api",
): Promise<void> {
  if (!target.files.some((file) => file.path === path))
    throw new UiControlError("notFound", "File is not in this node.");
  if (target.sessionId === null || target.sessionId === undefined || app.files === null)
    throw new UiControlError("denied", "Open a chat in this folder to view its files.");
  const files = origin === "ui" ? app.files.actions : app.files.debugApi;
  const shown = await files.openText(target.sessionId, `${target.path}/${path}`);
  if (!shown)
    throw new UiControlError("notFound", "This file is outside the chat's readable folder.");
  app.shell.actions.selectSession(target.sessionId);
  app.shell.actions.openTab("file");
}
