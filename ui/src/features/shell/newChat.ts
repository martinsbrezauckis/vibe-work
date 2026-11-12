/**
 * New chat: the rail's second item, Ctrl N and the list's "New chat" button all open the same
 * view: the start view with a real composer, focused, its folder chip preselected (the current project's main folder,
 * else the folder chosen last, else the newest session's folder, else none; never a temporary chat's folder, and the
 * core drops a folder that no longer exists). No session exists until the first
 * send (`state/composer.ts`); pressing New chat again on that view only focuses the composer, keeping its draft.
 * Depends on: home (leaving pages), composerFocus, the session summaries. Called by: Rail, shell actions (Ctrl N),
 * SessionList, the `ui/rail` route.
 * Invariants: never reuses or reopens a conversation; the previous session is left exactly as it was (its draft,
 * scroll, panel); while a terminal or a page is open on the New chat screen its folder stays (`app.newChat`); a folder a Debug API client put on the chip is replaced only when the user leaves a session for
 * New chat (its mark then ends, `newFolderBy` "session").
 */
import { NEW_CHAT } from "../../state/newChatKey";
import { summarize } from "../../state/sessionSummary";
import { requestComposerFocus } from "../../state/composerFocus";
import type { App } from "./AppContext";
import { goHome } from "./home";

/**
 * The project's main folder, falling back to an ordinary session's working folder. A temporary chat gives none: a
 * folder Vibe Work made for it is deleted with it, so a chat started there would lose its files (F7).
 */
function folderOf(app: App, id: string): string | null {
  if (app.sessionTools?.isTemporary(id) === true) return null;
  const groups = app.projects?.groups.store.get().data;
  const project = groups?.projects.find((p) => p.id === groups.members[id]);
  if (project?.mainFolder) return project.mainFolder;
  const entry = app.core?.store.get().sessions[id];
  // A copy with no project facts yet must not silently become the next chat's folder.
  return entry === undefined || entry.session["worktree"] != null ? null : summarize(entry).cwd;
}

/** The newest session's folder (by last activity), when any session has one. */
function newestFolder(app: App): string | null {
  const entries = Object.values(app.core?.store.get().sessions ?? {}).map((e) => summarize(e));
  const withFolder = entries.filter((s) => s.cwd !== null && !s.archived);
  withFolder.sort((a, b) => b.bumpedAt - a.bumpedAt);
  for (const session of withFolder) {
    const folder = folderOf(app, session.id);
    if (folder !== null) return folder;
  }
  return null;
}

/** Opens the new-chat view (or focuses its composer when it is already shown). */
export function openNewChat(app: App): void {
  const { shell } = app;
  goHome(app);
  const s = shell.store.get();
  if (s.selected !== null) {
    const cwd = folderOf(app, s.selected);
    shell.actions.selectSession(null);
    // A terminal or a page open there: the screen comes back unchanged, its folder included.
    if (
      app.newChat?.holds() !== true &&
      (cwd !== null || app.core?.store.get().sessions[s.selected]?.session["worktree"] != null)
    )
      shell.actions.setNewFolder(cwd, "session");
  } else if (s.newFolder === null) {
    const cwd = newestFolder(app);
    if (cwd !== null) shell.actions.setNewFolder(cwd, "session");
  }
  requestComposerFocus(NEW_CHAT);
}
