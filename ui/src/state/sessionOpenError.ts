/** Plain missing-chat notice shared by selection and transcript loading.
 * Depends on typed errors and toasts; concurrent resume failures for one chat are said once per app run.
 */
import { toCallError } from "../protocol/client/types";
import type { Toasts } from "./toastStore";

const reported = new WeakMap<Toasts, Set<string>>();

/** Reports a missing stored chat and returns whether this error was handled. */
export function reportMissingChat(thrown: unknown, toasts: Toasts, id: string): boolean {
  const error = toCallError(thrown).error;
  if (
    !["session/resume", "session/read"].includes(error.method ?? "") ||
    error.serverCode !== "not_found"
  )
    return false;
  let ids = reported.get(toasts);
  if (ids === undefined) {
    ids = new Set();
    reported.set(toasts, ids);
  }
  if (!ids.has(id)) {
    ids.add(id);
    toasts.push({
      kind: "error",
      bellKind: "failure",
      text: "This chat is not in the current chat list; if you changed CLI sharing, switch it back to see it.",
    });
  }
  return true;
}
