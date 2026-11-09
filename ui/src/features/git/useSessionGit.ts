/**
 * Session header Git facts. Uses the Git controller and core events; called by shell/Content.
 * Rechecks on mount, core changes and window focus, including folders newly put under Git.
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
/** Follows the mounted session's folder without borrowing a Debug API request's origin. */
export function useSessionGit(sessionId: string): GitStatusView | undefined {
  const { git, core } = useApp();
  const cwd = useCoreState((s) => s.sessions[sessionId]?.session["cwd"]);
  // A chosen stored session is read once it is open: until its snapshot the core answers "not open".
  const opened = useCoreState((s) => s.sessions[sessionId]?.state !== null);
  const status = useStore(git?.store ?? NO_GIT, (s) => s.status[sessionId]);
  useEffect(() => {
    if (git === null || !opened) return;
    const refresh = () => {
      void git.load(sessionId, "window");
    };
    const visible = () => {
      if (document.visibilityState === "visible") refresh();
    };
    refresh();
    const stop = core?.listen((event) => {
      if (event.type === "worktreesChanged" || event.type === "gitBrowserChanged") refresh();
    });
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", visible);
    return () => {
      stop?.();
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [git, core, sessionId, cwd, opened]);
  return status;
}
