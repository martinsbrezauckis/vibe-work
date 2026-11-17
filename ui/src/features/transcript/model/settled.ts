/** Final display of entries left open by a failed or interrupted turn. Depends on entry accessors;
 * called by buildRows for live updates and saved snapshots. Raw history stays intact.
 */
import {
  type Obj,
  effectView,
  entryType,
  inProgress,
  stateOf,
  obj,
  detailOf,
  str,
  personRefusal,
} from "./entries";

// Stable projections preserve row memoization when another turn changes.
const resolutions = {
  failed: {
    cache: new WeakMap<Obj, Obj>(),
    status: "failed",
    reason: "Turn failed",
    verb: "Failed",
  },
  interrupted: {
    cache: new WeakMap<Obj, Obj>(),
    status: "cancelled",
    reason: "Turn interrupted",
    verb: "Stopped",
  },
  declined: {
    cache: new WeakMap<Obj, Obj>(),
    status: "cancelled",
    reason: "approval cancel_turn",
    verb: "Declined",
  },
};

/** Answered No callbacks identify their own gated effect, including unfinished saved entries. */
export function declinedEffects(entries: readonly Obj[]): Set<string> {
  const ids = new Set<string>();
  for (const entry of entries) {
    const state = stateOf(entry);
    const decision = obj(obj(state["output"])?.["decision"])?.["type"];
    const id = str(entry["relatedEntryId"]);
    if (
      entryType(entry) === "callback" &&
      detailOf(entry)["kind"] === "approval" &&
      state["status"] === "answered" &&
      (personRefusal(entry["vwAskOutcome"]) ||
        (entry["vwAskOutcome"] === undefined && decision === "cancel_turn")) &&
      id !== null
    )
      ids.add(id);
  }
  return ids;
}

/** A failed or interrupted turn cannot keep work live. Finished results win. */
export function settledEntry(entry: Obj, status: "failed" | "interrupted" | "declined"): Obj {
  const { cache, status: effectStatus, reason, verb } = resolutions[status];
  const cached = cache.get(entry);
  if (cached !== undefined) return cached;
  const type = entryType(entry);
  let result = entry;
  if (type === "effect") {
    const view = effectView(entry);
    if (view.live) {
      result = {
        ...entry,
        generationStatus: "completed",
        state: {
          ...stateOf(entry),
          status: effectStatus,
          reason,
          display: {
            verb,
            message: view.target,
            success: false,
          },
        },
      };
    }
  } else if ((type === "reasoning" || type === "message") && inProgress(entry)) {
    result = { ...entry, generationStatus: "completed" };
  }
  cache.set(entry, result);
  return result;
}
