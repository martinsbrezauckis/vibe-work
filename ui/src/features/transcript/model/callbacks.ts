/**
 * Callbacks (approvals and questions) as the transcript shows and answers them. The answer objects
 * built here are exactly Vibe's output shapes (`ApprovalCallbackOutput`, `UserInputCallbackOutput`,
 * `vibe/app_server/models.py:340,346`), which the gate validates again before anything reaches Vibe
 * (a malformed answer gets no reply from the server, so it must never be sent).
 * Depends on: entries.ts, protocol/core. Called by: the approval and question cards, the audit item,
 * the `ui/transcript` route.
 * Invariants: the offered decisions are Vibe's own `detail.choices` (all five when Vibe omits the
 * field, its model default) minus "Always allow", and "Allow for this session" only where the core offers it
 * (`offeredDecisions`); a multi-select answer joins the chosen labels with ", " and sets
 * `isOther` when any part is free text (Vibe's own encoding, `vibe/acp/agent.py:254-262`).
 */
import { toolFallback, toolText } from "./toolLabels";
import type { JsonValue } from "../../../protocol/core";
import { term } from "../../../vocabulary";
import { type Obj, arr, detailOf, obj, stateOf, str } from "./entries";

/** Vibe's approval decisions (`ApprovalDecisionType`, `models.py:302`) in display order. */
const DECISIONS = [
  "approve",
  "approve_for_session",
  "approve_permanently",
  "deny",
  "cancel_turn",
] as const;

/** One approval decision. */
export type Decision = (typeof DECISIONS)[number];

/** Past-tense text per decision for the audit row. */
const DECISION_DONE: Record<Decision, string> = {
  approve: "Allowed once",
  approve_for_session: "Allowed for this session",
  approve_permanently: "Always allowed",
  deny: "Said No",
  cancel_turn: "Stopped",
};

function isDecision(value: JsonValue | undefined): value is Decision {
  return typeof value === "string" && (DECISIONS as readonly string[]).includes(value);
}

/**
 * The decisions an approval card offers, in Vibe's order: Vibe's own `detail.choices` (all
 * five when Vibe omits the field, its model default) without "Always allow", which is never offered (Vibe would keep
 * a lasting rule in Vibe Work's config for every project), and with "Allow for this session" only when the core's
 * ask offers it (`offers` holds "session": Vibe Work can keep that grant; it sends Vibe a once-only allow). Unknown
 * ask (`offers` undefined): no session answer.
 */
export function offeredDecisions(entry: Obj, offers: readonly string[] | undefined): Decision[] {
  const choices = detailOf(entry)["choices"];
  const vibe = Array.isArray(choices) ? choices.filter(isDecision) : [...DECISIONS];
  return vibe.filter(
    (d) =>
      d !== "approve_permanently" &&
      (d !== "approve_for_session" || offers?.includes("session") === true),
  );
}

/** The gated effect's detail inside an approval callback (`detail.effect`). */
export function approvalEffect(entry: Obj): Obj {
  return obj(detailOf(entry)["effect"]) ?? {};
}

/**
 * The answer for an approval. A path scope only travels with a grant that outlives the call
 * (`approve_for_session`, `approve_permanently`: what Vibe records as a permission); `feedback` only
 * with a deny (Vibe passes it to the model).
 */
export function approvalOutput(decision: Decision, scope?: string, feedback?: string): JsonValue {
  const granting = decision === "approve_for_session" || decision === "approve_permanently";
  return {
    type: "approval",
    decision: { type: decision, ...(granting && scope !== undefined ? { pathScope: scope } : {}) },
    ...(decision === "deny" && feedback !== undefined && feedback.trim() !== ""
      ? { feedback: feedback.trim() }
      : {}),
  };
}

/** One question of a question card. */
export interface Question {
  question: string;
  header: string;
  options: { label: string; description: string }[];
  multiSelect: boolean;
  /** Free text ("Other") is allowed. */
  allowOther: boolean;
}

/** The questions of a `user_input` callback (`UserQuestionRequest`). */
export function questionsOf(entry: Obj): { questions: Question[]; footer: string | null } {
  const request = obj(detailOf(entry)["request"]) ?? {};
  const questions = arr(request["questions"]).flatMap((raw) => {
    const q = obj(raw);
    if (q === null) return [];
    return [
      {
        question: str(q["question"]) ?? "",
        header: str(q["header"]) ?? "",
        options: arr(q["options"]).flatMap((o) => {
          const option = obj(o);
          const label = str(option?.["label"]);
          return label === null ? [] : [{ label, description: str(option?.["description"]) ?? "" }];
        }),
        multiSelect: q["multiSelect"] === true,
        allowOther: q["hideOther"] !== true,
      },
    ];
  });
  return { questions, footer: str(request["footerNote"]) };
}

/** What the user picked for one question. */
export interface Picked {
  labels: readonly string[];
  /** Free text, used when non-empty (and allowed). */
  other: string;
}

/** The `UserAnswer` for one question, or `null` when nothing was picked. */
export function answerFor(question: Question, picked: Picked): JsonValue | null {
  const other = question.allowOther ? picked.other.trim() : "";
  const labels = question.multiSelect ? [...picked.labels] : picked.labels.slice(0, 1);
  const parts = other === "" ? labels : question.multiSelect ? [...labels, other] : [other];
  if (parts.length === 0) return null;
  return { question: question.question, answer: parts.join(", "), isOther: other !== "" };
}

/** The answer for a question card; `null` when a question has no answer yet. */
export function questionOutput(
  questions: readonly Question[],
  picks: readonly Picked[],
): JsonValue | null {
  const answers: JsonValue[] = [];
  for (const [i, question] of questions.entries()) {
    const answer = answerFor(question, picks[i] ?? { labels: [], other: "" });
    if (answer === null) return null;
    answers.push(answer);
  }
  return { type: "user_input", result: { answers, cancelled: false } };
}

/** The "skip" answer for a question card (`cancelled: true`, Vibe's own cancel). */
export function questionCancelOutput(): JsonValue {
  return { type: "user_input", result: { answers: [], cancelled: true } };
}

/** Where an answer this window knows about came from. */
export type AnsweredVia = "window" | "uiControl";

const SOURCE: Record<string, string> = {
  user: "you",
  config: "your settings",
  smart: term("accessModes", "smart-approve").name,
  bypass: term("accessModes", "auto-approve").name,
  never: "the access mode",
};

/** Which client the core says sent an answer (`callbackAnswered` event). */
export type AnswerOrigin = "ui" | "debug-api";

/**
 * Who answered, as a phrase ("by you, in this window"), as far as can be known: what this window sent
 * itself, else the client the core reports (`origin`: an answer through the Debug API's command route
 * is "via the Debug API"), else Vibe's `approvalSource` on the gated effect. Anything else (the CLI,
 * or an answer from before the app started) is "answered outside this window".
 */
function answeredBy(
  related: Obj | null,
  via: AnsweredVia | undefined,
  origin?: AnswerOrigin,
): string {
  if (via === "window") return "by you";
  if (via === "uiControl") return "by the Debug API (UI control)";
  if (origin === "debug-api") return "via the Debug API";
  if (origin === "ui") return "by you";
  const source = str(stateOf(related ?? {})["approvalSource"]);
  if (source !== null && source !== "user") return `by ${SOURCE[source] ?? source}`;
  return "answered outside this window";
}

/** What the core's ask record says about an allowed answer: for this session, and whether a kept grant gave it. */
export interface AskAllowed {
  scope: string;
  by: string;
}

/**
 * The one-line audit text of a resolved callback. `allowed` is the core's ask record outcome when it was an allow:
 * Vibe always receives a once-only allow (Vibe Work keeps session grants itself), so "for this session" and "the
 * earlier answer" come from the record, not from Vibe's entry.
 */
export function auditText(
  entry: Obj,
  related: Obj | null,
  via: AnsweredVia | undefined,
  origin?: AnswerOrigin,
  allowed?: AskAllowed,
): string {
  const state = stateOf(entry);
  const status = str(state["status"]) ?? "";
  const detail = detailOf(entry);
  const tool = str(approvalEffect(entry)["toolName"]) ?? "";
  const summary = toolText(
    tool,
    str(obj(approvalEffect(entry)["display"])?.["summary"]) ??
      str(entry["title"]) ??
      toolFallback(tool),
    str(obj(approvalEffect(entry)["input"])?.["command"]),
  );
  if (status === "cancelled" || status === "expired") {
    const word = status === "cancelled" ? "Closed" : "No answer in time";
    const reason = str(state["reason"]);
    return `${word}: ${summary}${reason !== null && reason !== "" ? ` (${reason})` : ""}`;
  }
  if (status !== "answered") return `${summary}: ${status || "unknown state"}`;
  const output = obj(state["output"]) ?? {};
  if (detail["kind"] === "user_input") {
    const result = obj(output["result"]) ?? {};
    if (result["cancelled"] === true) return `Question skipped · ${answeredBy(null, via, origin)}`;
    const answers = arr(result["answers"])
      .map((a) => str(obj(a)?.["answer"]))
      .filter((a): a is string => a !== null);
    return `Answered: ${answers.join(" · ") || "(empty)"} · ${answeredBy(null, via, origin)}`;
  }
  const decision = obj(output["decision"])?.["type"];
  if (allowed?.scope === "session" && allowed.by === "grant")
    return `${DECISION_DONE.approve_for_session} (your earlier answer): ${summary}`;
  const done =
    allowed?.scope === "session"
      ? DECISION_DONE.approve_for_session
      : isDecision(decision)
        ? DECISION_DONE[decision]
        : "Answered";
  return `${done}: ${summary} · ${answeredBy(related, via, origin)}`;
}
