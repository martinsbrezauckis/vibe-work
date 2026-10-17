/**
 * The session row's mark for loops: while Vibe holds recurring prompts for the session, a small repeat icon on its
 * row. The words are its label and tooltip; icon only, because a word tag squeezed the title to one letter at
 * 1280 px (see the voice chat mark).
 * Depends on: the composer store hooks (the loops last read), the loops model, Icon. Called by: sessions/SessionRow.
 * Invariants: render only; nothing for a session without loops (or whose loops were not read yet: the list is read
 * when the session's composer opens).
 */
import type { ReactElement } from "react";

import { Icon } from "../../components/Icon";
import { useComposerState } from "../composer/hooks";
import { loopsRun } from "../composer/model/loops";

/** The mark for `sessionId`'s row, or nothing. */
export function LoopsTag({ sessionId }: { sessionId: string }): ReactElement | null {
  const n = useComposerState((s) => s.loops[sessionId]?.length ?? 0);
  if (n === 0) return null;
  const words = `${loopsRun(n)} in this session`;
  return (
    <span
      className="loops-row"
      role="img"
      aria-label={words}
      title={words}
      data-testid="sessions.loops-tag"
      data-count={n}
    >
      <Icon name="refresh-cw" size={14} />
    </span>
  );
}
