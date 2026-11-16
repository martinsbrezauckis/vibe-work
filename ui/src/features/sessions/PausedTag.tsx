/**
 * The session row's mark while the person has paused the chat's browser: a pause icon whose label says whether the
 * agent is waiting. Icon only, like the loops mark (a word tag squeezed the title at 1280 px).
 * Depends on: the core state (`pausedBrowser`, from `pausedTools` events), Icon. Called by: sessions/SessionRow.
 * Invariants: render only; nothing for a chat whose browser is not paused.
 */
import type { ReactElement } from "react";

import { Icon } from "../../components/Icon";
import { useCoreState } from "../shell/hooks";

/** The words of the mark: the agent waits, or the browser is paused with nothing waiting. */
export function pausedTagText(waiting: boolean): string {
  return waiting ? "Browser paused: the agent is waiting for you" : "Browser paused";
}

/** The mark for `sessionId`'s row, or nothing. */
export function PausedTag({ sessionId }: { sessionId: string }): ReactElement | null {
  const waiting = useCoreState((s) => s.pausedBrowser[sessionId]);
  if (waiting === undefined) return null;
  const words = pausedTagText(waiting);
  return (
    <span
      className="paused-row"
      role="img"
      aria-label={words}
      title={words}
      data-testid="sessions.paused-tag"
      data-waiting={waiting}
    >
      <Icon name="pause" size={14} />
    </span>
  );
}
