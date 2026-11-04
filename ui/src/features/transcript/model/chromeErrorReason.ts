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
  const message = cleanReason(str(error?.["message"]) ?? "");
  if (message === "") return null;
  const prefix =
    status === "unknown" || uncertain
      ? "Not known"
      : status === "paused" || error?.["code"] === "CONTROL_SUSPENDED"
        ? "Paused"
        : "Not done";
  return `${prefix}: ${message}`;
}
