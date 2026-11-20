/** Receipt-owned browser diagnostics for chromeOutcome. Never reads page envelopes, details or tool arguments. */
import { type Obj, obj, str } from "./entries";

/** A bounded plain line; React renders the result as text, never markup. */
function cleanReason(raw: string): string {
  const text = raw
    .replace(/\p{Cc}\][^\p{Cc}]*(?:\p{Cc}\\?|$)/gu, "")
    .replace(/\p{Cc}\[[0-?]*[ -/]*[@-~]/gu, "")
    .replace(/[\p{Cc}\p{Cf}]/gu, " ")
    .replace(/\s+/gu, " ")
    .trim()
    .replace(/^(?:(?:Not run|Not done|Not known|Paused):\s*)+/iu, "");
  const chars = Array.from(text);
  return chars.length > 220 ? `${chars.slice(0, 219).join("")}…` : text;
}

/** Only the top-level error is host-owned; its page counterpart is deliberately ignored. */
export function chromeErrorReason(
  answer: Obj | null,
  status: string | null,
  uncertain: boolean,
): string | null {
  const error = obj(answer?.["error"]);
  // Retain the existing fixed missing-tab diagnosis and recovery instruction for older receipts.
  if (error?.["code"] === "TARGET_REQUIRED") return null;
  // `CONTROL_SUSPENDED` means the chat's browser session was not active (stopped, or not started yet); a tab the
  // person paused answers `PERSON_PAUSED`. Their words are for the agent, so the person gets the fixed line.
  const code = error?.["code"];
  if (
    (code === "PERSON_PAUSED" || (code === "CONTROL_SUSPENDED" && status !== "paused")) &&
    status !== "unknown" &&
    !uncertain
  )
    return null;
  const message = cleanReason(str(error?.["message"]) ?? "");
  if (message === "") return null;
  const prefix =
    status === "unknown" || uncertain ? "Not known" : status === "paused" ? "Paused" : "Not done";
  return `${prefix}: ${message}`;
}
