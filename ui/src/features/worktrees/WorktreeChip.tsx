/**
 * The session header's copy chip: the session's branch and the base it started from
 * ("vibe/fix-login · from main"), or that the copy is being made, missing or removed. Nothing for a session
 * that works in its folder.
 * Depends on: AppContext (`worktrees`), shell hooks (the session's folder, to re-ask when it moves), model.ts.
 * Called by: shell/Content (header).
 * Invariants: read only; asks again when the session's folder changes (Vibe moves it into the copy) and while
 * the copy is still being made.
 */
import { type ReactElement, useEffect, useState } from "react";
import { Icon } from "../../components/Icon";

import type { WorktreeInfo } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { chipText } from "./model";

/** How often a pending copy is asked about again (ms). */
const PENDING_POLL = 1000;

/** The chip for `sessionId`. */
export function WorktreeChip({ sessionId }: { sessionId: string }): ReactElement | null {
  const { worktrees } = useApp();
  const cwd = useCoreState((s) => s.sessions[sessionId]?.session["cwd"]);
  const [found, setFound] = useState<{ id: string; info: WorktreeInfo | null } | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (worktrees === null) return;
    let live = true;
    worktrees
      .run({ op: "info", sessionId })
      .then((result) => {
        if (live && result.kind === "info") setFound({ id: sessionId, info: result.info });
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [worktrees, sessionId, cwd, tick]);
  const info = found !== null && found.id === sessionId ? found.info : null;
  const pending = info?.state === "pending";
  useEffect(() => {
    if (!pending) return;
    const timer = setTimeout(() => {
      setTick((t) => t + 1);
    }, PENDING_POLL);
    return () => {
      clearTimeout(timer);
    };
  }, [pending, tick]);
  if (info === null) return null;
  return (
    <span
      className={`chip wt-chip wt-${info.state}`}
      title={info.path ?? undefined}
      data-worktree="chip"
    >
      <Icon name="git-branch" size={12} />
      {chipText(info)}
    </span>
  );
}
