/**
 * Conversation views of Vibe's structured hook notices. The session history stays untouched for
 * diagnostics; only lifecycle/no-op notices and a duplicated skipped call leave the transcript.
 * Contract: tests/public/fixtures/hook-outcomes.json is also checked against Markdown export.
 * Depends on: entries, toolLabels. Called by: build and the hook row. Unknown notices remain visible.
 */
import {
  type Obj,
  detailOf,
  entryId,
  entryType,
  inputOf,
  obj,
  roleOf,
  stateOf,
  statusOf,
  str,
} from "./entries";
import { builtinTool } from "./toolLabels";

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

/** A pre-tool completion of one call (any status: a warning rewrite can still refuse it). */
function preToolCompletion(entry: Obj): boolean {
  const detail = detailOf(entry);
  return (
    isHookNotice(entry) && detail["kind"] === "hook_completed" && detail["scope"] === "pre_tool"
  );
}

/**
 * Vibe 3.0.3's refused step: skipped, with no input and the placeholder name "tool", under its own
 * `effect-<action id>` id, so it is found by order instead of by the call id.
 */
function isRefusedPlaceholder(entry: Obj): boolean {
  const tool = str(detailOf(entry)["toolName"]) ?? "";
  return (
    entryType(entry) === "effect" &&
    statusOf(entry) === "skipped" &&
    inputOf(entry) === null &&
    (tool === "" || tool === "tool")
  );
}

/**
 * The refused step of each pre-tool completion notice, keyed by the notice id. Vibe 2.x names the
 * step by the call id. Vibe 3.0.3 sends a placeholder after the notice: it is paired with the oldest
 * call refused (status "error") before it and not paired yet. A user message ends the pairing window;
 * a step of any other shape, or a placeholder without a refusal before it, is never paired.
 */
export function refusedSteps(entries: readonly Obj[]): Map<string, Obj> {
  const byId = new Map(entries.map((entry) => [entryId(entry), entry]));
  const steps = new Map<string, Obj>();
  // Call id -> its refusal notices, in the order the calls were refused.
  let waiting = new Map<string, string[]>();
  for (const entry of entries) {
    if (roleOf(entry) === "user") waiting = new Map();
    else if (preToolCompletion(entry)) {
      const callId = str(detailOf(entry)["toolCallId"]) ?? "";
      const named = byId.get(callId);
      if (named !== undefined && entryType(named) === "effect") steps.set(entryId(entry), named);
      else if (callId !== "" && detailOf(entry)["status"] === "error")
        waiting.set(callId, [...(waiting.get(callId) ?? []), entryId(entry)]);
    } else if (isRefusedPlaceholder(entry)) {
      const oldest = waiting.entries().next().value;
      if (oldest === undefined) continue;
      waiting.delete(oldest[0]);
      for (const notice of oldest[1]) steps.set(notice, entry);
    }
  }
  return steps;
}

/** The hook's reason on its refused step: 2.x wraps it in markup, 3.0.3 sends it as is. */
function denialReason(effect: Obj | undefined, hookName: string): string | null {
  if (effect === undefined || entryType(effect) !== "effect" || statusOf(effect) !== "skipped")
    return null;
  const reason = str(stateOf(effect)["reason"]) ?? "";
  if (isRefusedPlaceholder(effect)) return reason;
  const tool = str(detailOf(effect)["toolName"]) ?? "";
  const prefix = `<tool_error>Tool '${tool}' was denied by hook '${hookName}': `;
  const suffix = "</tool_error>";
  return reason.startsWith(prefix) && reason.endsWith(suffix)
    ? reason.slice(prefix.length, -suffix.length)
    : null;
}

/** "Not run: <hook> refused running commands." for a built-in tool, else "... the call." */
function refusedLine(name: string, detail: Obj, content: string): string {
  const tool = str(detail["toolName"]) || (/^Denied tool '([^']+)'$/.exec(content)?.[1] ?? "");
  return `Not run: ${name} refused ${builtinTool(tool)?.permission ?? "the call"}.`;
}

/**
 * `null` for lifecycle/no-op, a line for useful outcomes; called only for hook notices. `steps` is
 * `refusedSteps` of the same entries.
 */
export function hookLine(entry: Obj, steps: ReadonlyMap<string, Obj>): HookLine | null {
  const detail = detailOf(entry);
  if (detail["kind"] !== "hook_completed") return null;
  const content = str(detail["content"]) ?? "";
  const status = str(detail["status"]) ?? "";
  const name = str(detail["hookName"]) || "Hook";
  const effect = steps.get(entryId(entry));
  const reason = detail["scope"] === "pre_tool" ? denialReason(effect, name) : null;
  if (detail["scope"] === "pre_tool" && status === "error") {
    if (content.endsWith(" (strict)"))
      return { text: `${name} failed; the call was not run.`, error: true };
    return {
      text:
        reason ||
        (content.startsWith("Denied tool '") ? refusedLine(name, detail, content) : content) ||
        refusedLine(name, detail, content),
      error: true,
    };
  }
  // A successful transport binding/rewrite is diagnostic bookkeeping, not conversation text.
  if (
    reason === null &&
    ((name === "vibe-work tool receipt" && status === "ok") ||
      ((name === "vibe-work safety reminder" ||
        name === "vibe-work tool receipt" ||
        name === "vibe-work folder guard") &&
        detail["scope"] === "pre_tool" &&
        status === "warning" &&
        /^Rewrote tool_input for '[a-z][a-z0-9_]*'$/.test(content)))
  )
    return null;
  // The note that the user allowed a held command is for the agent's result; adding it is not news to the user.
  if (
    name === "vibe-work allowed note" &&
    detail["scope"] === "post_tool" &&
    status === "warning" &&
    /^Appended \d+ chars to tool result$/.test(content)
  )
    return null;
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
export function transcriptEntries(
  entries: readonly Obj[],
  steps: ReadonlyMap<string, Obj> = refusedSteps(entries),
): Obj[] {
  const suppressedCalls = new Set<string>();
  for (const entry of entries) {
    const effect = steps.get(entryId(entry));
    const name = str(detailOf(entry)["hookName"]) ?? "";
    if (effect !== undefined && denialReason(effect, name) !== null)
      suppressedCalls.add(entryId(effect));
  }
  return entries.filter((entry) =>
    (entryType(entry) === "callback" &&
      entry["vwToolAsk"] !== true &&
      str(obj(obj(detailOf(entry)["effect"])?.["input"])?.["_vwReceipt"]) !== null) ||
    // Vibe discards interrupted output from model context but keeps the text the person watched.
    // Other discarded generations (for example a replaced attempt) remain hidden.
    (obj(entry["outcome"])?.["type"] === "discarded" &&
      !(roleOf(entry) === "assistant" && obj(entry["outcome"])?.["cause"] === "interrupt"))
      ? false
      : isHookNotice(entry)
        ? hookLine(entry, steps) !== null
        : !(entryType(entry) === "effect" && suppressedCalls.has(entryId(entry))),
  );
}
