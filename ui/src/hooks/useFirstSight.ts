/**
 * "Is this the first time this thing is on screen?" for arrival motion (feel item 7): the transcript is
 * virtualised, so a card re-mounts whenever it scrolls back into view; only its first mount may animate.
 * Depends on: react. Called by: ApprovalCard, QuestionCard.
 * Invariants: keyed by a stable id (the callback id); remembered for the window's life (a reload replays once).
 */
import { useState } from "react";

const seen = new Set<string>();

/** True on the first mount ever for `key`, false on every later mount. */
export function useFirstSight(key: string): boolean {
  const [first] = useState(() => {
    const fresh = !seen.has(key);
    seen.add(key);
    return fresh;
  });
  return first;
}
