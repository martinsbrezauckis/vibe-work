/**
 * Stable sessions-footer entry to the existing copies inventory, refreshed on session/folder changes.
 * Depends on: core and worktrees stores, openCopies. Called by: shell/SessionList; no polling.
 */
import { useEffect } from "react";
import { useStore } from "../../state/createStore";
import { NO_WORKTREES } from "../../state/worktrees";
import { useApp } from "../shell/AppContext";
import { useCoreState, useSessionSummaries } from "../shell/hooks";
import { openCopies } from "./openCopies";

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
  if (worktrees === null) return null;
  return (
    <button type="button" className="rbtn-wide" onClick={() => void openCopies(app)}>
      Copies {copies.length}
    </button>
  );
}
