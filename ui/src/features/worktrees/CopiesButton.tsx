/**
 * Sessions-footer entry for leftover copies, refreshed on session/folder changes and hidden when empty.
 * Depends on: core and worktrees stores, openCopies. Called by: shell/SessionList; no polling.
 */
import { useEffect } from "react";
import { useStore } from "../../state/createStore";
import { NO_WORKTREES } from "../../state/worktrees";
import { useApp } from "../shell/AppContext";
import { useCoreState, useSessionSummaries } from "../shell/hooks";
import { openCopies } from "./openCopies";

const DESCRIPTION = "Session copies (git worktrees) no chat uses anymore: review or remove them";

/** The count includes leftover and kept copies, excluding those used by live sessions. */
export function CopiesButton() {
  const app = useApp();
  const { worktrees } = app;
  const copies = useStore(worktrees?.store ?? NO_WORKTREES, (s) => s.orphans);
  const phase = useCoreState((s) => s.server.phase);
  const sessions = useSessionSummaries()
    .map((s) => `${s.id}:${s.cwd ?? ""}:${String(s.archived)}`)
    .sort()
    .join("|");
  useEffect(() => {
    if (phase === "running") void worktrees?.loadOrphans();
  }, [phase, sessions, worktrees]);
  if (worktrees === null || copies.length === 0) return null;
  return (
    <button
      data-testid="worktrees.copies-button-open-copies"
      type="button"
      className="rbtn-wide"
      title={DESCRIPTION}
      aria-description={DESCRIPTION}
      onClick={() => void openCopies(app)}
    >
      Leftover copies {copies.length}
    </button>
  );
}
