/**
 * Synthetic history entries for the transcript tests, shaped like Vibe 2.26.0's
 * `PublicHistoryEntry` (all values invented). Each helper returns
 * plain JSON, as the core passes it.
 */
import type { JsonValue } from "../../../../ui/src/protocol/core";

type J = Record<string, JsonValue>;

let clock = 1_000_000;
/** A rising timestamp. */
export const tick = (): number => (clock += 1000);

const base = (
  id: string,
  type: string,
  turnId: string | null,
  extra: J = {},
): J => {
  const at = tick();
  return {
    id,
    sessionId: "s1",
    turnId,
    inputEntryId: null,
    createdAt: at,
    updatedAt: at,
    generationStatus: "completed",
    relatedEntryId: null,
    type,
    ...extra,
  };
};

export const user = (
  id: string,
  turnId: string | null,
  text: string,
  extra: J = {},
): J =>
  base(id, "message", turnId, {
    role: "user",
    content: [{ type: "text", text }],
    source: "turn_start",
    userDisplayContent: null,
    ...extra,
  });

export const assistant = (
  id: string,
  turnId: string | null,
  text: string,
  extra: J = {},
): J =>
  base(id, "message", turnId, {
    role: "assistant",
    content: [{ type: "text", text }],
    ...extra,
  });

export const reasoning = (id: string, turnId: string | null, text: string): J =>
  base(id, "reasoning", turnId, { text, summary: [] });

const display = (
  verb: string,
  message: string,
  statusText: string,
  extra: J = {},
): J => ({
  summary: `${verb} ${message}`,
  content: null,
  suffix: "",
  verb,
  message,
  settledVerb: `${verb}d`,
  settledMessage: message,
  statusText,
  ...extra,
});

/** An effect of `kind` in `status` (completed ones carry a settled display). */
export function effect(
  id: string,
  turnId: string | null,
  kind: string,
  status: string,
  options: {
    input?: JsonValue;
    output?: JsonValue;
    outputText?: string;
    extra?: J;
    error?: string;
  } = {},
): J {
  const settled =
    status !== "running" && status !== "pending" && status !== "blocked";
  const state: J = { status, outputText: options.outputText ?? "" };
  if (settled) {
    state["display"] = {
      success: status === "completed",
      verb: status === "completed" ? "Done" : "",
      message: options.error ?? `${kind} result`,
      warnings: [],
      approvalNote: null,
      suffix: "",
    };
    state["durationMs"] = 1500;
    state["approvalSource"] = "bypass";
  }
  if (options.output !== undefined) state["output"] = options.output;
  if (status === "failed")
    state["error"] = {
      message: options.error ?? "it failed",
      code: null,
      details: null,
    };
  if (status === "cancelled") state["reason"] = "client interrupt";
  return base(id, "effect", turnId, {
    title: kind,
    detail: {
      toolName: kind,
      display: display("Run", `${kind}-target`, `Running ${kind}`),
      kind,
      input: options.input ?? null,
      ...(options.extra ?? {}),
    },
    state,
    generationStatus: settled ? "completed" : "in_progress",
  });
}

/** An approval callback for `effectId`. */
export function approval(
  id: string,
  turnId: string | null,
  status: string,
  effectId: string | null,
  extra: J = {},
): J {
  const state: J =
    status === "answered"
      ? {
          status,
          output: {
            type: "approval",
            decision: { type: "approve", pathScope: null },
            feedback: null,
          },
        }
      : status === "open"
        ? { status }
        : { status, reason: "Turn interrupted" };
  return base(id, "callback", turnId, {
    callbackId: `cbid-${id}`,
    title: "Allow bash?",
    relatedEntryId: effectId,
    generationStatus: status === "open" ? "in_progress" : "completed",
    detail: {
      kind: "approval",
      effect: {
        toolName: "bash",
        display: display("Running", "rm -rf build", "Running command", {
          summary: "bash: rm -rf build",
        }),
        kind: "shell",
        input: { command: "rm -rf build" },
      },
      requiredPermissions: [
        {
          scope: "command_pattern",
          invocationPattern: "rm -rf build",
          sessionPattern: "rm *",
          label: "rm *",
          pathScopeRoot: null,
        },
      ],
      choices: [
        "approve",
        "approve_for_session",
        "approve_permanently",
        "deny",
        "cancel_turn",
      ],
      pathScopeChoices: [],
      relatedEntryId: effectId,
      reason: null,
    },
    state,
    ...extra,
  });
}

/** A question callback. */
export function question(
  id: string,
  turnId: string | null,
  status: string,
  multiSelect = false,
  hideOther = false,
): J {
  return base(id, "callback", turnId, {
    callbackId: `cbid-${id}`,
    title: "User input required",
    generationStatus: status === "open" ? "in_progress" : "completed",
    detail: {
      kind: "user_input",
      request: {
        questions: [
          {
            question: "Which colour?",
            header: "Colour",
            options: [
              { label: "Red", description: "warm" },
              { label: "Blue", description: "" },
            ],
            multiSelect,
            hideOther,
          },
        ],
        footerNote: "Pick one",
      },
    },
    state:
      status === "answered"
        ? {
            status,
            output: {
              type: "user_input",
              result: {
                answers: [
                  { question: "Which colour?", answer: "Red", isOther: false },
                ],
                cancelled: false,
              },
            },
          }
        : { status },
  });
}

export const notice = (
  id: string,
  turnId: string | null,
  kind: string,
  level = "info",
  message = `notice ${kind}`,
): J => base(id, "notice", turnId, { level, message, detail: { kind } });

export const checkpoint = (
  id: string,
  kind: string,
  message: string | null = null,
): J => base(id, "checkpoint", null, { kind, message, details: null });

/** A turn record (`PublicTurn`). */
export function turn(
  id: string,
  status: string,
  startedAt: number,
  completedAt: number | null,
  extra: J = {},
): J {
  return {
    id,
    sessionId: "s1",
    status,
    inputEntryId: null,
    startedAt,
    completedAt,
    error: null,
    stopReason: null,
    ...extra,
  };
}
