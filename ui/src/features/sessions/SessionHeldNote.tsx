/**
 * A session another process holds (Vibe 2.26.1 lease holder): while the Vibe CLI (or another app)
 * has the session open, it is read-only here. The note says who holds it and that it unlocks by itself; the composer
 * under it is inert meanwhile (the core would refuse a send anyway, naming the holder). The core checks the shown
 * session every 2 s, so the lock lifts within about 2 s after the other process closes it.
 * Depends on: shell hooks (core state `held`). Called by: shell/Content (the session view).
 * Invariants: renders nothing for a free session.
 */
import { TEXT } from "../../vocabulary";
import type { ReactElement } from "react";

import { useCoreState } from "../shell/hooks";

/** The holding process's pid, or `null` while the session is free here. */
export function useHeldBy(sessionId: string): number | null {
  return useCoreState((s) => {
    const parent = Object.values(s.subagentCards).find(
      (card) => card.sessionId === sessionId,
    )?.parentSessionId;
    return s.held[sessionId] ?? (parent === undefined ? null : (s.held[parent] ?? null));
  });
}

/** The note above the composer, or nothing. */
export function SessionHeldNote({ sessionId }: { sessionId: string }): ReactElement | null {
  const pid = useHeldBy(sessionId);
  const kind = useCoreState((s) => s.heldKind[sessionId]);
  if (pid === null) return null;
  return (
    <p className="server-note held-note" role="status" data-testid="session-held">
      <span className="px warn" aria-hidden="true" />
      <span>
        {kind === "terminal"
          ? `${TEXT.thisSession} is open in the terminal`
          : `${TEXT.thisSession} is open in another window`}
        <br />
        {kind === "terminal"
          ? "Read-only here. Quit Vibe in the terminal to continue here."
          : "Read-only here. Close this session in the other window to continue here."}
      </span>
    </p>
  );
}
