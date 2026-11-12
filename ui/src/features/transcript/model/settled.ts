/** Final display of entries left open by a failed or interrupted turn. Depends on entry accessors;
 * called by buildRows for live updates and saved snapshots. Raw history stays intact.
 */
import { type Obj, effectView, entryType, inProgress, stateOf } from "./entries";

// Stable projections preserve row memoization when another turn changes.
const stopped = new WeakMap<Obj, Obj>();
const failed = new WeakMap<Obj, Obj>();

/** A failed or interrupted turn cannot keep work live. Finished results win. */
export function settledEntry(entry: Obj, status: "failed" | "interrupted"): Obj {
  const cache = status === "failed" ? failed : stopped;
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
          status: status === "failed" ? "failed" : "cancelled",
          reason: status === "failed" ? "Turn failed" : "Turn interrupted",
          display: {
            verb: status === "failed" ? "Failed" : "Stopped",
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
