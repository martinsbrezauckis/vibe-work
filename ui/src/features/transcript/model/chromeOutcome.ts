/**
 * What a finished Chrome Control step actually did, in one plain line, as far as Chrome Control's answer says it.
 * Chrome Control often answers "queued" and
 * reports the end later; the line then says that it has not reported yet, never "done". A failure whose reason did
 * not reach Vibe Work (Vibe passes only "The MCP tool returned an error") says so plainly.
 * the step that turns Chrome Control's (or the headless browser's) tools on for the chat gets one line too:
 * they turn on when the reply ends and Vibe continues by itself, they are already on, or it was refused.
 * Pure. Depends on: entries. Called by: StepItem.
 * Invariant: `null` for a step that is not Chrome Control's or still runs.
 * Receipt-owned error reasons use a short sanitised line. From the untrusted page envelope only the error code is
 * normally read as a key into a closed list. Safari's extension-copy line also checks one exact, closed cause value;
 * page messages, URLs and arbitrary details never become an outcome line.
 */
import { TEXT, term } from "../../../vocabulary";
import { type Obj, detailOf, effectView, inputOf, obj, outputOf, outputText, str } from "./entries";
import { chromeErrorReason } from "./chromeErrorReason";
import { SAFARI_EXTENSION_CONFLICT_NOTICE } from "../../settings/chrome/safariConflict";

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
  if (view.live || view.status === "skipped" || view.status === "declined") return null;
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

/** Browser words use the host receipt or requested profile, never the untrusted page payload. */
function browserWords(entry: Obj, text: string): string {
  const input = inputOf(entry);
  const requested =
    detailOf(entry)["toolName"] === "mcp_vw.chrome_session" && input?.["action"] === "open"
      ? str(input["profileId"])
      : null;
  const profile = str(answerOf(entry)?.["profileId"]) ?? requested ?? "";
  const browser = profile.startsWith("edge:")
    ? "Edge"
    : profile.startsWith("firefox:")
      ? "Firefox"
      : profile.startsWith("chrome:") || /^[\da-f]{8}(?:-[\da-f]{4}){3}-[\da-f]{12}$/i.test(profile)
        ? "Chrome"
        : null;
  if (browser !== null) return text.replace(/\bChrome\b/g, browser);
  if (text === term("builtinSkills", "vw:chrome").name) return "Your browser";
  if (text === NO_CHROME) return "Not done: your browser is not connected to Vibe Work right now.";
  return text
    .replace(/\bYour Chrome\b/g, "Your browser")
    .replace(/\bChrome\b/g, "your browser")
    .replace(/^your browser\b/, "Your browser");
}

/** Localizes only built-in browser labels; page titles and other free text stay unchanged. */
export function chromeStepLabel(entry: Obj, text: string): string {
  const tool = str(detailOf(entry)["toolName"]) ?? "";
  return CHROME_TOOL.test(tool) &&
    [term("builtinSkills", "vw:chrome").name, "Read Chrome status"].includes(text)
    ? browserWords(entry, text)
    : text;
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

const REFUSED = "Not done: Chrome refused it.";
const NO_CHROME = "Not done: no Chrome is connected to Vibe Work right now.";
const DISCONNECTED = "Not done: Chrome was disconnected.";
/** The chat's browser session was not active when the call arrived (`CONTROL_SUSPENDED`): never "Paused". */
const NOT_RUNNING = TEXT.browserNotRunningCall;
/** The helper refused the call because the person paused that tab (`PERSON_PAUSED`): it did not run, nothing waits. */
const PAUSED_TAB = TEXT.browserPausedTabCall;
/** The core's refusal of an agent call after the person's Stop in the browser (person_stop.rs `refusal`). */
const STOPPED_BY_PERSON =
  /^Not run: the user pressed Stop in (?:Your )?(Chrome|Edge|Firefox|Safari)\. /u;

/** The person's line for the core's plain-text Stop refusal, in the browser it names; else `null`. */
function stoppedByPerson(entry: Obj): string | null {
  const content = outputOf(entry)?.["content"];
  const text = Array.isArray(content)
    ? content.map((part) => str(obj(part)?.["text"]) ?? "").join("")
    : outputText(entry);
  const browser = STOPPED_BY_PERSON.exec(text.trim())?.[1];
  return browser === undefined ? null : TEXT.browserStoppedCall.replace(/\bChrome\b/gu, browser);
}
const LOST = TEXT.chromeLost;
const LATE = TEXT.chromeLate;
const UNKNOWN = TEXT.chromeUnknown;
const PAIR = "Waiting for you to pair in Chrome.";
const OTHER_APP = "Chrome is paired with another Vibe Work. Pair again in Chrome.";
const STOPPED = "Not done: the task was stopped in Chrome.";
const EXPIRED = "Not done: the request ran out in Chrome.";
const TAB_CLOSED = "Not done: the tab was closed.";
const CHANGED = "Not done: the page changed.";
const BUSY = "Not done: Chrome is busy.";
/** The line for a failure whose code is not in {@link BY_CODE}, or whose reason did not reach Vibe Work. */
const ANY_PROBLEM = TEXT.chromeProblem;

/** Each line and the codes that give it. */
const LINES: readonly (readonly [string, readonly string[]])[] = [
  // The tool's missing-target diagnosis and recovery, without protocol names or page-controlled words.
  ["Not done: No tab selected. Choose a tab or create one first.", ["TARGET_REQUIRED"]],
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
  [NOT_RUNNING, ["CONTROL_SUSPENDED"]],
  [PAUSED_TAB, ["PERSON_PAUSED"]],
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
 * The closed list of codes the window knows: Chrome Control's helper and extension codes  and
 * the core's own (`OWNER_REQUIRED`, `TRANSPORT_LOST`, `BUSY`). Inside the page envelope any other code gives
 * {@link ANY_PROBLEM}.
 */
const BY_CODE: ReadonlyMap<string, string> = new Map(
  LINES.flatMap(([line, codes]) => codes.map((code) => [code, line] as const)),
);

/**
 * The failure line of an answer, or `null`. A helper answer is wrapped by the core (`untrustedPageContent`,
 * content.rs): there only `error.code` is read, never its message or details, which can carry page text, and a code
 * not in the list gives {@link ANY_PROBLEM}. Receipt-owned codes take precedence; their own messages are handled
 * separately by chromeErrorReason. A status of "unknown" never says "Not done" (the effect may have happened).
 */
function failureLine(answer: Obj | null, status: string | null): string | null {
  const page = obj(answer?.["untrustedPageContent"]);
  const code =
    str(obj(answer?.["error"])?.["code"]) ??
    (page === null ? field(answer, "code") : str(obj(page["error"])?.["code"]));
  if (code === null || code === "") return null;
  const line = BY_CODE.get(code) ?? (page === null ? null : ANY_PROBLEM);
  if (line === null) return null;
  return status === "unknown" && !line.startsWith("Not known:") ? UNKNOWN : line;
}

/** The helper's exact Safari world-mismatch code and cause, without its free text or page data. */
function safariExtensionConflict(answer: Obj | null, status: string | null): string | null {
  if (!str(answer?.["profileId"])?.startsWith("safari:")) return null;
  const error = obj(obj(answer?.["untrustedPageContent"])?.["error"]);
  const code = str(error?.["code"]);
  const cause = str(obj(error?.["details"])?.["cause"]);
  if (cause !== "extension_world_mismatch") return null;
  if (status === "failed" && code === "SAFARI_EXTENSION_CONFLICT")
    return SAFARI_EXTENSION_CONFLICT_NOTICE;
  if (status === "unknown" && code === "OUTCOME_UNKNOWN")
    return `Outcome unknown. ${SAFARI_EXTENSION_CONFLICT_NOTICE}`;
  return null;
}

const BY_STATUS: Readonly<Record<string, string>> = {
  queued: TEXT.chromeQueued,
  pending: TEXT.chromeQueued,
  dispatched: TEXT.chromeQueued,
  approval_required: "Waiting for you in Chrome.",
  awaiting_user: "Waiting for you in Chrome.",
  completed: "Done in Chrome.",
  succeeded: "Done in Chrome.",
  cancelled: "Stopped in Chrome.",
  stopped: "Stopped in Chrome.",
  paused: "Paused in Chrome. Vibe waits until you press Resume.",
  denied: "Not done: Chrome refused it.",
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
  // The core's own plain-text refusal after the person's Stop: the person's line, never the agent's instruction.
  const stopped = stoppedByPerson(entry);
  if (stopped !== null) return stopped;
  const batch = answerOf(entry);
  const steps = batch?.["steps"];
  const last = Array.isArray(steps) ? obj(obj(steps.at(-1))?.["result"]) : null;
  // The batch stopped because this step failed; keep that step's reason and uncertainty.
  const answer =
    field(batch, "status") === "stopped" && last !== null && field(last, "status") !== "completed"
      ? last
      : batch;
  const status = field(answer, "status")?.toLowerCase() ?? null;
  const safariConflict = safariExtensionConflict(answer, status);
  if (safariConflict !== null) return safariConflict;
  const code = field(answer, "code") ?? field(obj(answer?.["untrustedPageContent"]), "code");
  // A failed wait reached a result: the condition was not met. An unknown status still means the action's
  // outcome was not established, regardless of the same timeout code.
  if (
    status === "failed" &&
    code === "timeout" &&
    ["page.wait", "page.wait_for"].includes(str(inputOf(entry)?.["action"]) ?? "")
  )
    return "The wait ended: the page did not show what the agent waited for in time.";
  if (status === "failed" && code === "timeout") return browserWords(entry, ANY_PROBLEM);
  const failure = failureLine(answer, status);
  const reason = chromeErrorReason(answer, status, failure?.startsWith("Not known:") ?? false);
  if (reason !== null) return reason;
  if (failure !== null) return browserWords(entry, failure);
  const line =
    status !== null && Object.hasOwn(BY_STATUS, status)
      ? (BY_STATUS[status] ?? null)
      : view.status === "failed"
        ? ANY_PROBLEM
        : null;
  return line === null ? null : browserWords(entry, line);
}
