/**
 * The session header's "Temporary" chip: shown while the open session is a temporary chat, with
 * what happens to it in words.
 * Depends on: AppContext, state/createStore, state/sessionTools. Called by: shell/Content (the session header).
 * Invariants: renders nothing for a normal session. A narrow header (a side panel open, or a chat area under
 * 720 px) shows only "Temporary"; the tooltip always carries the whole explanation.
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
      title="Temporary chat: deleted when you quit. Files in a folder you chose stay changed; the chat's own folder is removed."
    >
      <span>
        Temporary<span className="temporary-more"> · deleted when you quit</span>
      </span>
    </Badge>
  );
}
