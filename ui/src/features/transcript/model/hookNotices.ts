/**
 * Conversation views of Vibe's structured hook notices. The session history stays untouched for
 * diagnostics; only lifecycle/no-op notices and a duplicated skipped call leave the transcript.
 * Depends on: entries. Called by: build and the hook row. Unknown notices remain visible.
 */
import { type Obj, detailOf, entryId, entryType, stateOf, statusOf, str } from "./entries";

const HOOK_KINDS = new Set([
  "hook_run_started",
  "hook_run_completed",
  "hook_started",
  "hook_completed",
]);

/** One meaningful hook outcome, rendered as plain text outside collapsed steps. */
export interface HookLine {
  text: string;
  error: boolean;
}

/** A structured hook notice, never inferred from the human-readable message. */
export function isHookNotice(entry: Obj): boolean {
  return entryType(entry) === "notice" && HOOK_KINDS.has(str(detailOf(entry)["kind"]) ?? "");
}

/** The exact reason Vibe wraps around a skipped hook-denied call, without its internal markup. */
function denialReason(effect: Obj | undefined, hookName: string): string | null {
  if (effect === undefined || entryType(effect) !== "effect" || statusOf(effect) !== "skipped")
    return null;
  const reason = str(stateOf(effect)["reason"]) ?? "";
  const tool = str(detailOf(effect)["toolName"]) ?? "";
  const prefix = `<tool_error>Tool '${tool}' was denied by hook '${hookName}': `;
  const suffix = "</tool_error>";
  return reason.startsWith(prefix) && reason.endsWith(suffix)
    ? reason.slice(prefix.length, -suffix.length)
    : null;
}

/** `null` for lifecycle/no-op, a line for useful outcomes; called only for hook notices. */
export function hookLine(entry: Obj, byId: ReadonlyMap<string, Obj>): HookLine | null {
  const detail = detailOf(entry);
  if (detail["kind"] !== "hook_completed") return null;
  const content = str(detail["content"]) ?? "";
  const status = str(detail["status"]) ?? "";
  const name = str(detail["hookName"]) || "Hook";
  const effect = byId.get(str(detail["toolCallId"]) ?? "");
  const reason = detail["scope"] === "pre_tool" ? denialReason(effect, name) : null;
  if (detail["scope"] === "pre_tool" && status === "error") {
    if (content.endsWith(" (strict)"))
      return { text: `${name} failed; the call was not run.`, error: true };
    return {
      text:
        reason ||
        (content.startsWith("Denied tool '") ? `Not run: ${name} refused the call.` : content) ||
        `Not run: ${name} refused the call.`,
      error: true,
    };
  }
  // An invalid input rewrite can itself deny the call after a warning completion.
  if (reason !== null) return { text: reason || `Not run: ${name} refused the call.`, error: true };
  if (content.trim() !== "") return { text: `${name}: ${content}`, error: status === "error" };
  if (status === "ok") return null;
  // Keep warning/error/unknown outcomes visible, including failures after a tool already ran.
  return {
    text: `${name}: ${str(entry["message"]) || (status === "error" ? "failed" : "reported a warning")}`,
    error: status === "error",
  };
}

/** Entries represented in the conversation. Raw history and all callbacks remain in the store. */
export function transcriptEntries(entries: readonly Obj[]): Obj[] {
  const byId = new Map(entries.map((entry) => [entryId(entry), entry]));
  const suppressedCalls = new Set<string>();
  for (const entry of entries) {
    if (!isHookNotice(entry) || detailOf(entry)["kind"] !== "hook_completed") continue;
    const detail = detailOf(entry);
    const callId = str(detail["toolCallId"]) ?? "";
    const name = str(detail["hookName"]) ?? "";
    if (detail["scope"] === "pre_tool" && denialReason(byId.get(callId), name) !== null)
      suppressedCalls.add(callId);
  }
  return entries.filter((entry) =>
    isHookNotice(entry)
      ? hookLine(entry, byId) !== null
      : !(entryType(entry) === "effect" && suppressedCalls.has(entryId(entry))),
  );
}
