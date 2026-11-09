/** Final display of entries left open by an interrupted turn. Depends on entry accessors;
 * called by buildRows for live updates and saved snapshots. Raw history stays intact.
 */
import { type Obj, effectView, entryType, inProgress, stateOf } from "./entries";

// Stable projections preserve row memoization when another turn changes.
const stopped = new WeakMap<Obj, Obj>();

/** An interrupted turn cannot keep a tool, reasoning or narration live. Finished results win. */
export function interruptedEntry(entry: Obj): Obj {
  const cached = stopped.get(entry);
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
          status: "cancelled",
          reason: "Turn interrupted",
          display: { verb: "Stopped", message: view.target, success: false },
        },
      };
    }
  } else if ((type === "reasoning" || type === "message") && inProgress(entry)) {
    result = { ...entry, generationStatus: "completed" };
  }
  stopped.set(entry, result);
  return result;
}
