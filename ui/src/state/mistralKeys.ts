/** Derived Account row text and chat activity, used by Settings; metadata only. */
import type { MistralKeyTestView } from "../protocol/core";
import type { CoreState } from "./coreModel";
import { statusType } from "./sessionSummary";

/** Matches the core's busy rule, including approvals and queued turns. The core checks again on use. */
export function chatsWorking(state: CoreState): boolean {
  return Object.values(state.sessions).some((entry) => {
    const status = statusType(entry.session);
    const queue = entry.state?.["turnQueue"];
    const callbacks = entry.state?.["activeCallbacks"];
    return (
      !["idle", "failed", "archived"].includes(status) ||
      entry.openCallbackIds.length > 0 ||
      (Array.isArray(callbacks) && callbacks.length > 0) ||
      (queue !== null &&
        typeof queue === "object" &&
        !Array.isArray(queue) &&
        Array.isArray(queue["items"]) &&
        queue["items"].length > 0)
    );
  });
}

/** A saved test never includes the response body. */
export function savedTestText(test: MistralKeyTestView | null): string {
  if (test === null) return "Not tested yet";
  if (test.modelCount !== null) return "Key works";
  if (test.code !== null) return `Mistral answered ${String(test.code)}: ${test.text ?? ""}`;
  return test.text ?? "Not tested yet";
}
