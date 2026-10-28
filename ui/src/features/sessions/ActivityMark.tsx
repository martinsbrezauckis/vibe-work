/**
 * The session activity badge. No box: a bordered square read as a checkbox in a list. Idle draws
 * nothing, running is a small breathing orange dot (a CSS pseudo-element, still under reduced motion), done is a plain
 * tick, "needs input" a bold "!", failed and stuck a red dot. The accessible name (and `data-badge`) carries the state,
 * so colour is never the only signal; the glyph itself is decorative.
 * Depends on: state/sessionSummary (`Badge`, labels), components/Icon. Called by: SessionRow, "Needs you" rows,
 * project cards.
 * Invariants: no logic here (the badge is derived once, in `sessionSummary.ts`); the dot is the only moving part
 * and stops under `prefers-reduced-motion` (tokens.css).
 */
import type { ReactElement } from "react";

import { Icon } from "../../components/Icon";
import { BADGE_LABEL, type Badge as BadgeKind } from "../../state/sessionSummary";

function glyph(kind: BadgeKind): ReactElement | null {
  switch (kind) {
    case "done":
      return <Icon name="check" size={12} />;
    case "input":
      return <b className="sb-bang">!</b>;
    case "run":
    case "failed":
    case "stuck":
    case "idle":
      return null;
  }
}

/** The badge for one state. */
export function ActivityMark({ kind }: { kind: BadgeKind }) {
  return (
    <span
      className={`sb ${kind === "stuck" ? "failed" : kind}`}
      role="img"
      aria-label={BADGE_LABEL[kind]}
      title={kind === "stuck" ? BADGE_LABEL[kind] : undefined}
      data-badge={kind}
    >
      {glyph(kind)}
    </span>
  );
}
