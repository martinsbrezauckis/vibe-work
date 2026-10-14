/**
 * The session header's note on what `.worktreeinclude` copied into this session's copy: a small
 * expandable "Copied N files from .worktreeinclude" with the names and what was skipped and why. Shown only for
 * a copy started in this run (the report is kept in memory).
 * Depends on: AppContext (`worktrees`), createStore, IncludeList. Called by: shell/Content (header).
 * Invariants: read only; nothing for a session without a report.
 */
import type { ReactElement } from "react";

import { useStore } from "../../state/createStore";
import { NO_WORKTREES } from "../../state/worktrees";
import { useApp } from "../shell/AppContext";
import { IncludeList } from "./IncludeList";

/** The note for `sessionId`. */
export function IncludeNote({ sessionId }: { sessionId: string }): ReactElement | null {
  const { worktrees } = useApp();
  const report = useStore(worktrees?.store ?? NO_WORKTREES, (s) => s.included[sessionId]);
  if (report === undefined) return null;
  return <IncludeList report={report} plan={false} />;
}
