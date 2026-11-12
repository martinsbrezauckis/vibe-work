/** Deduplicates Git reads at turn completion, including fast turns and either event order; used by Changes registration. */
import type { SessionEntry } from "../../state/sessionSummary";
import { statusType } from "../../state/sessionSummary";

/** Keeps the shown chat's completed turns across snapshots so replayed history cannot cause more reads. */
export function createTurnRefresh() {
  let shown: string | null = null;
  let working = false;
  let active: string | undefined;
  const checked = new Set<string>();
  return (id: string | null, entry: SessionEntry | undefined): boolean => {
    const status = entry === undefined ? null : statusType(entry.session);
    const busy = status === "running" || status === "blocked";
    const turns = entry?.state?.["turns"];
    const ended = (Array.isArray(turns) ? turns : []).flatMap((turn) =>
      typeof turn === "object" &&
      turn !== null &&
      !Array.isArray(turn) &&
      typeof turn["id"] === "string" &&
      typeof turn["status"] === "string" &&
      ["completed", "failed", "interrupted", "cancelled"].includes(turn["status"])
        ? [turn["id"]]
        : [],
    );
    const changed = shown !== id;
    const finished = !changed && !busy && (working || ended.some((turn) => !checked.has(turn)));
    if (changed) {
      checked.clear();
      active = undefined;
    }
    if (changed || finished) {
      for (const turn of ended) checked.add(turn);
      if (finished && active !== undefined) checked.add(active);
    }
    const sessionStatus = entry?.session["status"];
    if (
      busy &&
      typeof sessionStatus === "object" &&
      sessionStatus !== null &&
      !Array.isArray(sessionStatus)
    ) {
      const turn = sessionStatus["activeTurnId"];
      active = typeof turn === "string" ? turn : undefined;
    }
    shown = id;
    working = busy;
    return finished;
  };
}
