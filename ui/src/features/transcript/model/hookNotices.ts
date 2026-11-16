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
import { TEXT } from "../../../vocabulary";

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

/**
 * The person's line for a call that waited for the paused browser and ended without running, from the app's
 * `vwHeldCall` mark on the call's pre-tool hook notices (`paused_tools_gate::HELD_CALL_FIELD` in vw-core-api). The
 * app records the outcome when the person's Stop or the turn's end ends the call, so the line does not depend on
 * whether the hook's answer reached the runtime before the turn was interrupted.
 */
function heldLine(entry: Obj): string | null {
  if (detailOf(entry)["scope"] !== "pre_tool") return null;
  if (entry["vwHeldCall"] === "stopped") return TEXT.pausedBrowserStoppedCall;
  if (entry["vwHeldCall"] === "turnEnded") return TEXT.pausedBrowserTurnEndedCall;
  return null;
}

/** A start notice of one call's hooks. */
function hookStart(entry: Obj): boolean {
  const kind = detailOf(entry)["kind"];
  return isHookNotice(entry) && (kind === "hook_run_started" || kind === "hook_started");
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

/** The tool a refusal notice names ("Denied tool 'preview_open'"), or "" when it names none. */
function deniedTool(entry: Obj): string {
  return /^Denied tool '([^']+)'$/.exec(str(detailOf(entry)["content"]) ?? "")?.[1] ?? "";
}

/**
 * The refusal names Vibe Work's combined pre-tool hook puts before each reason
 * (`Guard::refusal_name` in vw-agent-tools); a skip without one (the person's own No) is not a refusal.
 */
const GUARD_REASON =
  /^(Safety reminder|Process guard|Folder guard|Git guard|Connector guard|Tool receipt): /;

/**
 * Vibe 3.1's refused step: the call itself, skipped with its input, full tool name and the guard's
 * named reason, under an `effect-<action id>` id that is not the call id. `tool` is the name the
 * refusal notice gave.
 */
function isRefusedNamedStep(entry: Obj, tool: string): boolean {
  const name = str(detailOf(entry)["toolName"]) ?? "";
  return (
    tool !== "" &&
    entryType(entry) === "effect" &&
    statusOf(entry) === "skipped" &&
    inputOf(entry) !== null &&
    (name === tool || name.endsWith(`.${tool}`)) &&
    GUARD_REASON.test(str(stateOf(entry)["reason"]) ?? "")
  );
}

/**
 * The refused step of each pre-tool completion notice, keyed by the notice id. Vibe 2.x names the
 * step by the call id. Vibe 3.0.3 sends a placeholder after the notice: it is paired with the oldest
 * call refused (status "error") before it and not paired yet. Vibe 3.1 sends the skipped call itself
 * after the notice, under another id: it is paired with the oldest such refusal of the same tool.
 * A user message ends the pairing window; a step of any other shape, a step named by a call id, or a
 * step without a refusal before it, is never paired.
 * A held call the app marked (`heldLine`) whose hooks never completed (the runtime cancelled them) maps the
 * first start notice of its hooks to itself: that notice carries the call's line.
 */
export function refusedSteps(entries: readonly Obj[]): Map<string, Obj> {
  const byId = new Map(entries.map((entry) => [entryId(entry), entry]));
  const steps = new Map<string, Obj>();
  // Marked call id -> the first start notice of its hooks; completed calls leave.
  const heldStarts = new Map<string, Obj>();
  const completed = new Set<string>();
  for (const entry of entries) {
    const callId = str(detailOf(entry)["toolCallId"]) ?? "";
    if (preToolCompletion(entry)) completed.add(callId);
    else if (hookStart(entry) && heldLine(entry) !== null && !heldStarts.has(callId))
      heldStarts.set(callId, entry);
  }
  for (const [callId, start] of heldStarts)
    if (!completed.has(callId)) steps.set(entryId(start), start);
  // Call id -> its refusal notices, in the order the calls were refused.
  let waiting = new Map<string, string[]>();
  // Call id -> the tool its refusal notices name.
  const tools = new Map<string, string>();
  // Steps found by their call id: never paired again by order.
  const named = new Set<string>();
  for (const entry of entries) {
    if (roleOf(entry) === "user") waiting = new Map();
    else if (preToolCompletion(entry)) {
      const callId = str(detailOf(entry)["toolCallId"]) ?? "";
      const step = byId.get(callId);
      if (step !== undefined && entryType(step) === "effect") {
        steps.set(entryId(entry), step);
        named.add(callId);
      } else if (callId !== "" && detailOf(entry)["status"] === "error") {
        waiting.set(callId, [...(waiting.get(callId) ?? []), entryId(entry)]);
        if (!tools.has(callId)) tools.set(callId, deniedTool(entry));
      }
    } else if (isRefusedPlaceholder(entry)) {
      const oldest = waiting.entries().next().value;
      if (oldest === undefined) continue;
      waiting.delete(oldest[0]);
      for (const notice of oldest[1]) steps.set(notice, entry);
    } else if (!named.has(entryId(entry))) {
      const match = [...waiting].find(([call]) => isRefusedNamedStep(entry, tools.get(call) ?? ""));
      if (match === undefined) continue;
      waiting.delete(match[0]);
      for (const notice of match[1]) steps.set(notice, entry);
    }
  }
  return steps;
}

/**
 * The person's own line for a browser call that waited while they paused the browser: the receipt's
 * reason (prefixed by its guard's name) is the agent's instruction, the person sees what happened.
 */
function personReason(reason: string | null): string | null {
  if (reason?.endsWith(TEXT.agentPausedBrowserStopped)) return TEXT.pausedBrowserStoppedCall;
  if (reason?.endsWith(TEXT.agentPausedBrowserTurnEnded)) return TEXT.pausedBrowserTurnEndedCall;
  return reason;
}

/**
 * The hook's reason on its refused step: 2.x wraps it in markup, 3.0.3 sends it as is on a placeholder step, and
 * 3.1 as is on the named call itself. A plain reason on a named call counts only when the notice says the hook
 * refused it (`refused`): another skip of that call (the person's own No in Vibe) is not the hook's.
 */
function denialReason(effect: Obj | undefined, hookName: string, refused: boolean): string | null {
  if (effect === undefined || entryType(effect) !== "effect" || statusOf(effect) !== "skipped")
    return null;
  const reason = str(stateOf(effect)["reason"]) ?? "";
  if (isRefusedPlaceholder(effect)) return reason;
  const tool = str(detailOf(effect)["toolName"]) ?? "";
  const prefix = `<tool_error>Tool '${tool}' was denied by hook '${hookName}': `;
  const suffix = "</tool_error>";
  if (reason.startsWith(prefix) && reason.endsWith(suffix))
    return reason.slice(prefix.length, -suffix.length);
  return refused && reason.trim() !== "" && !reason.startsWith("<tool_error>") ? reason : null;
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
  const held = heldLine(entry);
  // A held call whose hooks the runtime cancelled: its start notice is the only trace of the call.
  if (hookStart(entry))
    return held !== null && steps.get(entryId(entry)) === entry
      ? { text: held, error: true }
      : null;
  if (detail["kind"] !== "hook_completed") return null;
  const content = str(detail["content"]) ?? "";
  const status = str(detail["status"]) ?? "";
  // The person's Stop (or the turn's end) ended the call while it waited: never shown as a hook refusal.
  if (held !== null && status === "error") return { text: held, error: true };
  const name = str(detail["hookName"]) || "Hook";
  const effect = steps.get(entryId(entry));
  const reason =
    detail["scope"] === "pre_tool"
      ? personReason(denialReason(effect, name, status === "error"))
      : null;
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
    const refused = detailOf(entry)["status"] === "error";
    if (effect !== undefined && denialReason(effect, name, refused) !== null)
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
