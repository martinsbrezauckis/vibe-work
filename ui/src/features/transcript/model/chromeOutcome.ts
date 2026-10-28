/**
 * What a finished Chrome Control step actually did, in one plain line, as far as Chrome Control's answer says it.
 * Chrome Control often answers "queued" and
 * reports the end later; the line then says that it has not reported yet, never "done". A failure whose reason did
 * not reach Vibe Work (Vibe passes only "The MCP tool returned an error") says so plainly.
 * the step that turns Chrome Control's (or the headless browser's) tools on for the chat gets one line too:
 * they turn on when the reply ends and Vibe continues by itself, they are already on, or it was refused.
 * Pure. Depends on: entries. Called by: StepItem.
 * Invariant: no operation names or codes in the words; `null` for a step that is not Chrome Control's or still runs.
 */
import { type Obj, detailOf, effectView, obj, outputOf, outputText, str } from "./entries";

const CHROME_TOOL = /^mcp_vw\.chrome_(status|task|execute)$/i;

/** The tools that turn a tool set on for the chat (`on_demand` in the core), and their lines by answer status. */
const TURN_ON: Readonly<Record<string, Readonly<Record<string, string>>>> = {
  "mcp_vw.use_your_chrome": {
    turningOn: "Chrome Control turns on for this chat.",
    alreadyOn: "Chrome Control is already on in this chat.",
    refused:
      "Not done: Chrome Control is not set up. Set it up in Settings › Browser › Your Chrome.",
  },
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

const BY_CODE: Readonly<Record<string, string>> = {
  POLICY_DENIED: "Not done: Chrome Control refused it.",
  REQUEST_DENIED: "Not done: the request was refused in Chrome.",
  CONTROL_SUSPENDED: "Paused in Chrome. Vibe waits until you press Resume.",
  TRANSPORT_LOST: "Not known: Chrome Control lost the connection. Check the page in Chrome.",
  ACTION_TIMEOUT: "Not known: Chrome Control did not finish in time. Check the page in Chrome.",
  OWNER_APPROVAL_REQUIRED: "Waiting for you to pair in Chrome.",
  OWNER_IDENTITY_MISMATCH: "Chrome shows a different Vibe Work — pair again.",
  OWNER_CONFLICT: "Chrome shows a different Vibe Work — pair again.",
  OUTCOME_UNKNOWN:
    "Not known: Chrome Control cannot tell whether this happened. Check the page in Chrome.",
  PROFILE_NOT_FOUND: "Not done: no Chrome is connected to Vibe Work right now.",
  ADAPTER_DISCONNECTED: "Not done: Chrome was disconnected.",
  ADAPTER_UNAVAILABLE: "Not done: no Chrome is connected to Vibe Work right now.",
  TASK_STOPPED: "Not done: the task was stopped in Chrome.",
};

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
  const code = field(answer, "code");
  if (code !== null && Object.hasOwn(BY_CODE, code)) return BY_CODE[code] ?? null;
  const status = field(answer, "status")?.toLowerCase() ?? null;
  if (status !== null && Object.hasOwn(BY_STATUS, status)) return BY_STATUS[status] ?? null;
  if (view.status === "failed") {
    const message = field(answer, "message");
    return message !== null && message !== "The MCP tool returned an error"
      ? `Not done: ${message}`
      : "Not done: Chrome Control reported a problem.";
  }
  return null;
}
