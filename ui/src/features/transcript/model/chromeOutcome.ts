/**
 * What a finished Chrome Control step actually did, in one plain line, as far as Chrome Control's answer says it.
 * Chrome Control often answers "queued" and
 * reports the end later; the line then says that it has not reported yet, never "done". A failure whose reason did
 * not reach Vibe Work (Vibe passes only "The MCP tool returned an error") says so plainly.
 * the step that turns Chrome Control's (or the headless browser's) tools on for the chat gets one line too:
 * they turn on when the reply ends and Vibe continues by itself, they are already on, or it was refused.
 * Pure. Depends on: entries. Called by: StepItem.
 * Invariant: no operation names or codes in the words; `null` for a step that is not Chrome Control's or still runs.
 * Invariant: every line is one of this file's fixed lines. An answer's words (an error's message or details, a page's
 * text) never reach it; from the untrusted page envelope only the error code is read, as a key into a closed list.
 */
import { type Obj, detailOf, effectView, obj, outputOf, outputText, str } from "./entries";

const CHROME_TOOL = /^mcp_vw\.chrome_(status|session|execute)$/i;

/** The tools that turn a tool set on for the chat (`on_demand` in the core), and their lines by answer status. */
const TURN_ON: Readonly<Record<string, Readonly<Record<string, string>>>> = {
  "mcp_vw.use_headless_browser": {
    turningOn: "The headless browser turns on for this chat.",
    alreadyOn: "The headless browser is already on in this chat.",
    refused: "Not done: the headless browser plugin is not installed.",
  },
};

/** The line of a finished turn-on step: its answer's status, or a refusal ("Not run: ..."). */
function turnOnLine(entry: Obj, lines: Readonly<Record<string, string>>): string | null {
  const view = effectView(entry);
  if (view.live || view.status === "skipped") return null;
  if (outputText(entry).trim().startsWith("Not run:")) return lines["refused"] ?? null;
  const status = field(answerOf(entry), "status");
  return status !== null && Object.hasOwn(lines, status) ? (lines[status] ?? null) : null;
}

/** `text` as a JSON object, or `null`. */
function jsonObject(text: string): Obj | null {
  const trimmed = text.trim();
  if (!trimmed.startsWith("{")) return null;
  try {
    return obj(JSON.parse(trimmed) as never);
  } catch {
    return null;
  }
}

/**
 * The step's answer as an object. Vibe 2.26.1 stores a tool's result as an envelope: its structured part, else the JSON in its text content. An output that is no
 * envelope is the answer itself; with no output, the step's text when that is JSON.
 */
function answerOf(entry: Obj): Obj | null {
  const output = outputOf(entry);
  if (output !== null) {
    const structured = obj(output["structured_content"]) ?? obj(output["structuredContent"]);
    if (structured !== null) return structured;
    const content = output["content"];
    if (Array.isArray(content)) {
      const text = content.map((part) => str(obj(part)?.["text"]) ?? "").join("");
      return jsonObject(text);
    }
    return output;
  }
  return jsonObject(outputText(entry));
}

/** A field of the answer, or of its `operation`, `result` or `error` object. */
function field(answer: Obj | null, name: string): string | null {
  if (answer === null) return null;
  for (const holder of [
    answer,
    obj(answer["operation"]),
    obj(answer["result"]),
    obj(answer["error"]),
  ]) {
    const value = str(holder?.[name]);
    if (value !== null && value !== "") return value;
  }
  return null;
}

const REFUSED = "Not done: Chrome Control refused it.";
const NO_CHROME = "Not done: no Chrome is connected to Vibe Work right now.";
const DISCONNECTED = "Not done: Chrome was disconnected.";
const PAUSED = "Paused in Chrome. Vibe waits until you press Resume.";
const LOST = "Not known: Chrome Control lost the connection. Check the page in Chrome.";
const LATE = "Not known: Chrome Control did not finish in time. Check the page in Chrome.";
const UNKNOWN =
  "Not known: Chrome Control cannot tell whether this happened. Check the page in Chrome.";
const PAIR = "Waiting for you to pair in Chrome.";
const OTHER_APP = "Chrome shows a different Vibe Work — pair again.";
const STOPPED = "Not done: the task was stopped in Chrome.";
const EXPIRED = "Not done: the request ran out in Chrome.";
const TAB_CLOSED = "Not done: the tab was closed.";
const CHANGED = "Not done: the page changed.";
const BUSY = "Not done: Chrome is busy.";
/** The line for a failure whose code is not in {@link BY_CODE}, or whose reason did not reach Vibe Work. */
const ANY_PROBLEM = "Not done: Chrome Control reported a problem.";

/** Each line and the codes that give it. */
const LINES: readonly (readonly [string, readonly string[]])[] = [
  [
    REFUSED,
    // Helper (broker) and extension refusals; DENIED is the helper's code for a "denied" status.
    ["FORBIDDEN", "forbidden", "DENIED", "CAPABILITY_REQUIRED", "OWNERSHIP_MISMATCH"],
  ],
  [
    REFUSED,
    [
      "not_owned",
      "out_of_scope",
      "LOCAL_NETWORK",
      "ADDRESS_REFUSED",
      "HTTPS_REQUIRED",
      "SOCKET_GUARD",
    ],
  ],
  [NO_CHROME, ["ADAPTER_UNAVAILABLE", "OWNER_REQUIRED"]],
  [DISCONNECTED, ["ADAPTER_DISCONNECTED", "disconnected"]],
  [PAUSED, ["CONTROL_SUSPENDED"]],
  [LOST, ["TRANSPORT_LOST", "EMPTY_RESPONSE"]],
  [LATE, ["ACTION_TIMEOUT", "timeout", "operation_timeout", "page_tool_timeout"]],
  [
    UNKNOWN,
    [
      "OUTCOME_UNKNOWN",
      "OUTCOME_UNREPORTED",
      "UNKNOWN",
      "RESULT_REFUSED",
      "dialog_outcome_unknown",
    ],
  ],
  [PAIR, ["OWNER_APPROVAL_REQUIRED"]],
  [OTHER_APP, ["OWNER_IDENTITY_MISMATCH", "OWNER_CONFLICT"]],
  [STOPPED, ["STOP_REQUESTED", "CANCEL_REQUESTED", "CANCELLED", "cancelled"]],
  [EXPIRED, ["LEASE_EXPIRED", "CONTEXT_EXPIRED", "request_expired"]],
  [TAB_CLOSED, ["TARGET_CLOSED", "window_closed"]],
  [
    CHANGED,
    [
      "DOCUMENT_CHANGED",
      "SESSION_CHANGED",
      "STALE_REF",
      "STALE_REFERENCE",
      "stale_ref",
      "stale_document",
      "stale_frame",
    ],
  ],
  [BUSY, ["BUSY", "TAB_BUSY", "OPERATION_BUSY", "busy"]],
];

/**
 * The closed list of codes the window knows: Chrome Control's helper and extension codes (Chrome Control 0.1.3) and
 * the core's own (`OWNER_REQUIRED`, `TRANSPORT_LOST`, `BUSY`). Inside the page envelope any other code gives
 * {@link ANY_PROBLEM}.
 */
const BY_CODE: ReadonlyMap<string, string> = new Map(
  LINES.flatMap(([line, codes]) => codes.map((code) => [code, line] as const)),
);

/**
 * The failure line of an answer, or `null`. A helper answer is wrapped by the core (`untrustedPageContent`,
 * content.rs): there only `error.code` is read, never its message or details, which can carry page text, and a code
 * not in the list gives {@link ANY_PROBLEM}. The core's own failures and older answers carry the code outside; there
 * only a known code gives a line. A status of "unknown" never says "Not done" (the effect may have happened).
 */
function failureLine(answer: Obj | null, status: string | null): string | null {
  const page = obj(answer?.["untrustedPageContent"]);
  const code = page === null ? field(answer, "code") : str(obj(page["error"])?.["code"]);
  if (code === null || code === "") return null;
  const line = BY_CODE.get(code) ?? (page === null ? null : ANY_PROBLEM);
  if (line === null) return null;
  return status === "unknown" && !line.startsWith("Not known:") ? UNKNOWN : line;
}

const BY_STATUS: Readonly<Record<string, string>> = {
  queued: "Sent to Chrome. Chrome Control has not reported yet whether it finished.",
  pending: "Sent to Chrome. Chrome Control has not reported yet whether it finished.",
  dispatched: "Sent to Chrome. Chrome Control has not reported yet whether it finished.",
  approval_required: "Waiting for you in Chrome.",
  awaiting_user: "Waiting for you in Chrome.",
  completed: "Done in Chrome.",
  succeeded: "Done in Chrome.",
  cancelled: "Stopped in Chrome.",
  stopped: "Stopped in Chrome.",
  paused: "Paused in Chrome. Vibe waits until you press Resume.",
  denied: "Not done: Chrome Control refused it.",
  expired: "Not done: the request ran out in Chrome.",
};

/** The outcome line of a finished Chrome Control step, or `null`. */
export function chromeOutcome(entry: Obj): string | null {
  const tool = str(detailOf(entry)["toolName"]) ?? "";
  const turnOn = TURN_ON[tool];
  if (turnOn !== undefined) return turnOnLine(entry, turnOn);
  if (!CHROME_TOOL.test(tool)) return null;
  const view = effectView(entry);
  if (view.live || view.status === "skipped") return null;
  const answer = answerOf(entry);
  const status = field(answer, "status")?.toLowerCase() ?? null;
  const failure = failureLine(answer, status);
  if (failure !== null) return failure;
  if (status !== null && Object.hasOwn(BY_STATUS, status)) return BY_STATUS[status] ?? null;
  return view.status === "failed" ? ANY_PROBLEM : null;
}
