/**
 * The session header's "Temporary" chip: shown while the open session is a temporary chat, with
 * what happens to it in words.
 * Depends on: AppContext, state/createStore, state/sessionTools. Called by: shell/Content (the session header).
 * Invariants: renders nothing for a normal session.
 */
import { Badge } from "../../components/Controls";
import type { ReactElement } from "react";

import { useStore } from "../../state/createStore";
import { NO_SESSION_TOOLS } from "../../state/sessionTools";
import { useApp } from "../shell/AppContext";

/** The chip, or nothing. */
export function TemporaryChip({ sessionId }: { sessionId: string }): ReactElement | null {
  const { sessionTools } = useApp();
  const temporary = useStore(sessionTools?.store ?? NO_SESSION_TOOLS, (s) =>
    s.temporary.includes(sessionId),
  );
  if (!temporary) return null;
  return (
    <Badge
      presentation="temporary"
      data-testid="temporary-chip"
      title="Temporary chat: Vibe Work deletes it when it quits. Files it changed stay changed."
    >
      Temporary · deleted when you quit
    </Badge>
  );
}
