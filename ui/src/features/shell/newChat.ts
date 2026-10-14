/**
 * New chat: the rail's second item, Ctrl N and the list's "New chat" button all open the same
 * view: the start view with a real composer, focused, its folder chip preselected (the current session's folder,
 * else the folder chosen last, else the newest session's folder, else none). No session exists until the first
 * send (`state/composer.ts`); pressing New chat again on that view only focuses the composer, keeping its draft.
 * Depends on: home (leaving pages), composerFocus, the session summaries. Called by: Rail, shell actions (Ctrl N),
 * SessionList, the `ui/rail` route.
 * Invariants: never reuses or reopens a conversation; the previous session is left exactly as it was (its draft,
 * scroll, panel); a folder a Debug API client put on the chip is replaced only when the user leaves a session for
 * New chat (its mark then ends, `newFolderBy` "session").
 */
import { NEW_CHAT } from "../../state/newChatKey";
import { summarize } from "../../state/sessionSummary";
import { requestComposerFocus } from "../../state/composerFocus";
import type { App } from "./AppContext";
import { goHome } from "./home";

/** The working folder of session `id`, when known. */
function folderOf(app: App, id: string): string | null {
  const entry = app.core?.store.get().sessions[id];
  return entry === undefined ? null : summarize(entry).cwd;
}

/** The newest session's folder (by last activity), when any session has one. */
function newestFolder(app: App): string | null {
  const entries = Object.values(app.core?.store.get().sessions ?? {}).map((e) => summarize(e));
  const withFolder = entries.filter((s) => s.cwd !== null && !s.archived);
  withFolder.sort((a, b) => b.bumpedAt - a.bumpedAt);
  return withFolder[0]?.cwd ?? null;
}

/** Opens the new-chat view (or focuses its composer when it is already shown). */
export function openNewChat(app: App): void {
  const { shell } = app;
  goHome(app);
  const s = shell.store.get();
  if (s.selected !== null) {
    const cwd = folderOf(app, s.selected);
    shell.actions.selectSession(null);
    if (cwd !== null) shell.actions.setNewFolder(cwd, "session");
  } else if (s.newFolder === null) {
    const cwd = newestFolder(app);
    if (cwd !== null) shell.actions.setNewFolder(cwd, "session");
  }
  requestComposerFocus(NEW_CHAT);
}
