/**
 * The session header's "Temporary" chip: shown while the open session is a temporary chat, with
 * what happens to it in words.
 * Depends on: AppContext, state/createStore, state/sessionTools. Called by: shell/Content (the session header).
 * Invariants: renders nothing for a normal session. A narrow header (a side panel open, or a chat area under
 * 1040 px) shows only "Temporary"; the whole explanation is the chip's tooltip, reachable without a mouse: the chip
 * is in the Tab order, the tooltip is its accessible description, it shows while the chip is hovered or focused,
 * and Esc hides it while focus stays.
 */
import { Badge } from "../../components/Controls";
import { type ReactElement, useId, useState } from "react";

import { useStore } from "../../state/createStore";
import { NO_SESSION_TOOLS } from "../../state/sessionTools";
import { useApp } from "../shell/AppContext";

/** What a temporary chat keeps and loses: the chip's tooltip. */
const EXPLANATION =
  "Temporary chat: deleted when you quit. Files in a folder you chose stay changed; the chat's own folder is removed.";

/** The chip, or nothing. */
export function TemporaryChip({ sessionId }: { sessionId: string }): ReactElement | null {
  const { sessionTools } = useApp();
  const temporary = useStore(sessionTools?.store ?? NO_SESSION_TOOLS, (s) =>
    s.temporary.includes(sessionId),
  );
  const tip = useId();
  const [shown, setShown] = useState(false);
  if (!temporary) return null;
  return (
    <span
      className="temporary-explained"
      data-shown={shown ? "true" : "false"}
      onMouseEnter={() => {
        setShown(true);
      }}
      onMouseLeave={() => {
        setShown(false);
      }}
    >
      <Badge
        presentation="temporary"
        data-testid="temporary-chip"
        tabIndex={0}
        // A note role: Orca on Linux reads the description of a focused note, not of a plain focusable span.
        role="note"
        aria-describedby={tip}
        onFocus={() => {
          setShown(true);
        }}
        onBlur={() => {
          setShown(false);
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape" && shown) {
            e.stopPropagation();
            setShown(false);
          }
        }}
      >
        <span>
          Temporary<span className="temporary-more"> · deleted when you quit</span>
        </span>
      </Badge>
      <span className="temporary-tooltip" role="tooltip" id={tip}>
        {EXPLANATION}
      </span>
    </span>
  );
}
