/**
 * The session header's PR chip: "PR #7 · open · checks failing" when the
 * session's branch has a known pull request. A click opens the Changes tab, where the card is.
 * Depends on: AppContext (`git`, `shell`), model.ts. Called by: shell/Content (header).
 * Invariant: read only; shows nothing without a known pull request.
 */
import type { ReactElement } from "react";

import { useStore } from "../../state/createStore";
import type { Git } from "../../state/git";
import { useApp } from "../shell/AppContext";
import { stateText } from "./model";

/** The chip for `sessionId` (nothing without a core). */
export function PrChip({ sessionId }: { sessionId: string }): ReactElement | null {
  const { git } = useApp();
  return git === null ? null : <Chip git={git} sessionId={sessionId} />;
}

function Chip({ git, sessionId }: { git: Git; sessionId: string }): ReactElement | null {
  const { shell } = useApp();
  const card = useStore(
    git.store,
    (s) => s.cards[sessionId]?.card ?? s.status[sessionId]?.pr ?? null,
  );
  if (card === null) return null;
  const checks =
    card.checks.failed > 0
      ? "checks failing"
      : card.checks.pending > 0
        ? "checks running"
        : card.checks.passed > 0
          ? "checks passed"
          : "";
  const text = [`PR #${String(card.number)}`, stateText(card).toLowerCase(), checks]
    .filter((t) => t !== "")
    .join(" · ");
  return (
    <button
      data-testid="git.pr-chip-open-tab"
      type="button"
      className={`chip pr-chip pr-${card.state}${card.checks.failed > 0 ? " bad" : ""}`}
      data-git="pr-chip"
      title={`Show PR #${String(card.number)} in Changes tab`}
      aria-label={`Show PR #${String(card.number)} in Changes tab`}
      onClick={() => {
        shell.actions.openTab("changes");
      }}
    >
      {text}
    </button>
  );
}
