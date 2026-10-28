/**
 * Session header Git facts. Uses the Git controller and core events; called by shell/Content.
 * Rechecks plain folders while mounted because git init need not emit a changed-file event.
 */
import { useEffect } from "react";
import { createStore, useStore } from "../../state/createStore";
import type { GitUi } from "../../state/git";
import type { GitStatusView } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";

const NO_GIT = createStore<GitUi>({
  status: {},
  selected: {},
  cards: {},
  dialog: null,
  busy: {},
  notice: {},
  loadError: {},
  output: {},
  committed: {},
});
const DISCOVERY_POLL = 1000;

/** Whether the session folder itself is in Git, independent of a selected child repository. */
function sessionHasGit(status: GitStatusView | undefined): boolean {
  return status?.repos.some((repo) => repo.root === ".") === true;
}

/** Follows the mounted session's folder without borrowing a Debug API request's origin. */
export function useSessionGit(sessionId: string): GitStatusView | undefined {
  const { git, core } = useApp();
  const cwd = useCoreState((s) => s.sessions[sessionId]?.session["cwd"]);
  const status = useStore(git?.store ?? NO_GIT, (s) => s.status[sessionId]);
  useEffect(() => {
    if (git === null) return;
    void git.load(sessionId, "window");
    return core?.listen((event) => {
      if (event.type === "worktreesChanged" || event.type === "gitBrowserChanged")
        void git.load(sessionId, "window");
    });
  }, [git, core, sessionId, cwd]);
  const hasGit = sessionHasGit(status);
  useEffect(() => {
    if (git === null || hasGit) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      await git.load(sessionId, "window");
      if (live) timer = setTimeout(() => void poll(), DISCOVERY_POLL);
    };
    timer = setTimeout(() => void poll(), DISCOVERY_POLL);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [git, sessionId, cwd, hasGit]);
  return status;
}
