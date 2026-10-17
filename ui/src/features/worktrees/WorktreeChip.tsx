/**
 * The session header's location chip: project, named copy when present, and the working branch,
 * or that the copy is being made, missing or removed. Full location and path on hover/expansion.
 * Depends on: AppContext (worktrees, project groups), header Git facts, shell hooks, LocationChip.
 * Called by: shell/Content (header).
 * Invariants: read only; asks again when the session's folder changes (Vibe moves it into the copy) and while
 * the copy is still being made.
 */
import { type ReactElement, useEffect, useState } from "react";

import type { GitStatusView, WorktreeInfo } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { createStore, useStore } from "../../state/createStore";
import type { GroupsState } from "../../state/projectGroups";
import { locationText } from "./location";
import { LocationChip } from "./LocationChip";

/** How often a pending copy is asked about again (ms). */
const PENDING_POLL = 1000;
const NO_GROUPS = createStore<GroupsState>({
  data: null,
  error: null,
  renaming: null,
  creating: false,
  mover: null,
  deleting: null,
});

/** The chip for `sessionId`. */
export function WorktreeChip({
  sessionId,
  status,
}: {
  sessionId: string;
  status: GitStatusView | undefined;
}): ReactElement | null {
  const { worktrees, projects } = useApp();
  const cwd = useCoreState((s) => s.sessions[sessionId]?.session["cwd"]);
  const groups = useStore(projects?.groups.store ?? NO_GROUPS, (s) => s.data);
  const project = groups?.projects.find((p) => p.id === groups.members[sessionId])?.name ?? null;
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
  const text = locationText(project, typeof cwd === "string" ? cwd : null, info, status);
  if (text === null) return null;
  return (
    <LocationChip
      key={sessionId}
      text={text}
      path={info?.path ?? (typeof cwd === "string" ? cwd : undefined)}
      state={info?.state ?? "ready"}
    />
  );
}
