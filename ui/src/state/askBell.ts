/**
 * The bell for asks, by who decided:
 * - your own click (Allow once, for this session, Always, No) shows only the audit row: the waiting entry leaves
 *   the bell and nothing is added;
 * - a Never setting, or an answer the Debug API or the core gave: a Security group line (it happened without your
 *   click), and the waiting entry (if any) leaves;
 * - entries from asks are never removed by the app otherwise: only Dismiss / Clear all.
 * Pure. Depends on: protocol/core types, the AskCard model (`features/ask/model.ts`), the transcript entry helpers.
 * Called by: notifications.ts (`notificationFor`), notifyStore.ts (the waiting entry's fate).
 * Invariant: "by you" means the core recorded `by: person`; a Debug API UI-control click is recorded `debugApi`
 * (the window sends it with the Debug API origin), so it counts as not yours.
 */
import type { Ask, AskRecord, Origin } from "../protocol/core";
import { approvalQuestion, approvalSummary } from "../features/ask/model";
import { approvalEffect } from "../features/transcript/model/callbacks";
import { type Obj, inProgress, obj, statusOf } from "../features/transcript/model/entries";
import type { CoreState } from "./coreModel";
import { sessionTitle } from "./sessionSummary";

/** What a waiting ask is about, for bell words: the session title, the card's question and its summary. */
export interface AskWords {
  session: string;
  question: string;
  summary: string;
  /** The exact command of a command approval, when Vibe sent one (announcements name it). */
  command?: string;
}

/** The callback entry `callbackId` in `sessionId`'s history (or a subagent's open card), if the window holds it. */
export function callbackEntry(core: CoreState, sessionId: string, callbackId: string): Obj | null {
  const history = core.sessions[sessionId]?.state?.["history"];
  if (Array.isArray(history)) {
    for (const item of history) {
      const entry = obj(item);
      if (entry !== null && entry["callbackId"] === callbackId) return entry;
    }
  }
  const card = core.subagentCards[callbackId];
  return card === undefined ? null : obj(card.entry);
}

/** The core answers Vibe's app-tool transport callback automatically; the separate app card asks the person. */
export function toolTransport(core: CoreState, sessionId: string, callbackId: string): boolean {
  const entry = callbackEntry(core, sessionId, callbackId);
  const tool = entry === null ? null : approvalEffect(entry)["toolName"];
  return (
    entry !== null &&
    entry["vwToolAsk"] !== true &&
    typeof tool === "string" &&
    tool.startsWith("mcp_vw.")
  );
}

/** Callback-backed asks include app-tool cards absent from Vibe's transport callback list.
 * Protected changes and Chrome cards already supply their own history identities.
 */
export function cardAsks(core: CoreState, sessionId: string): string[] {
  return Object.values(core.asks)
    .filter(
      (ask) =>
        ask.surface === "card" && ask.session === sessionId && ask.id.startsWith("callback:"),
    )
    .map((ask) => ask.id.slice(9));
}

/** The summary of Chrome Control card `askId` in `sessionId`'s history, if the window holds it. */
function chromeCard(core: CoreState | undefined, sessionId: string, askId: string): string | null {
  const history = core?.sessions[sessionId]?.state?.["history"];
  if (!Array.isArray(history)) return null;
  for (const item of history) {
    const card = obj(obj(item)?.["chromeAsk"]);
    if (card?.["askId"] === askId && typeof card["summary"] === "string") return card["summary"];
  }
  return null;
}

/**
 * The words for ask `record` (falls back to the record's value-free title). `hostSessionId` names the session whose
 * title the words use when it is not the record's own (a subagent's card is shown in its parent's transcript).
 */
export function askWords(
  core: CoreState | undefined,
  record: AskRecord,
  hostSessionId?: string,
): AskWords {
  return askWordsFor(core, record.ask, hostSessionId);
}

/** `askWords` for an open `ask` (the arrival and last-minute announcements). */
export function askWordsFor(
  core: CoreState | undefined,
  ask: Ask,
  hostSessionId?: string,
): AskWords {
  const sessionId = hostSessionId ?? ask.session ?? "";
  const session = sessionTitle(core?.sessions[sessionId]?.session);
  const callbackId = ask.id.startsWith("callback:") ? ask.id.slice(9) : null;
  const entry =
    core === undefined || callbackId === null
      ? null
      : callbackEntry(core, ask.session ?? sessionId, callbackId);
  if (entry !== null && obj(entry["detail"])?.["kind"] === "approval") {
    const command = obj(approvalEffect(entry)["input"])?.["command"];
    return {
      session,
      question: approvalQuestion(entry),
      summary: approvalSummary(entry),
      ...(typeof command === "string" && command !== "" ? { command } : {}),
    };
  }
  if (ask.kind === "user_input")
    return { session, question: "Vibe asks", summary: "Vibe's question" };
  if (ask.kind.startsWith("chrome/ask/")) {
    const card = chromeCard(core, ask.session ?? sessionId, ask.id);
    return {
      session,
      question:
        ask.kind === "chrome/ask/closeTabs" ? "Close your tab?" : "Allow this in your Chrome?",
      summary: card ?? ask.title,
    };
  }
  if (ask.kind === "protected_change")
    return {
      session,
      question: "Allow this change to Vibe's folder?",
      summary: "change to Vibe's folder",
    };
  return { session, question: `Allow ${ask.title}?`, summary: ask.title };
}

/**
 * The waiting entry's words once its ask closed and no record came within a few seconds: the outcome is not known
 * here, so the entry says only that the session stopped waiting, and stays until dismissed (a record that comes
 * later still rewrites or removes it).
 */
export function stoppedWaitingText(core: CoreState, sessionId: string, callbackId: string): string {
  const session = sessionTitle(core.sessions[sessionId]?.session);
  const entry = callbackEntry(core, sessionId, callbackId);
  return entry !== null && obj(entry["detail"])?.["kind"] === "approval"
    ? `“${session}” stopped waiting for your answer to “${approvalQuestion(entry)}” (${approvalSummary(entry)}).`
    : `“${session}” stopped waiting for your answer.`;
}

/** Whether the conversation still presents this callback as an unanswered card. */
export function waitingCardOpen(core: CoreState, sessionId: string, callbackId: string): boolean {
  const entry = callbackEntry(core, sessionId, callbackId);
  return (
    entry !== null && (statusOf(entry) === "open" || (statusOf(entry) === "" && inProgress(entry)))
  );
}

/**
 * The Security line for an outcome that happened without your click, or `null` (your click, a grant's or a guard's
 * earlier answer, a cancellation).
 */
export function securityLine(record: AskRecord, words: AskWords): string | null {
  const outcome = record.outcome;
  switch (outcome.type) {
    case "never":
      return `Not allowed by your Never setting: ${words.summary}`;
    case "allowed":
      return outcome.by === "debugApi" ? `Allowed via the Debug API: ${words.summary}` : null;
    case "refused":
      if (outcome.by === "debugApi") return `Said No via the Debug API: ${words.summary}`;
      if (outcome.by === "core")
        return `Vibe Work did not allow ${words.summary}${outcome.note === null ? "" : ` (${outcome.note})`}`;
      return null;
    default:
      return null;
  }
}

/** Which Security guard name a line carries ("Never setting" or "Permission prompts"). */
export function securityGuard(record: AskRecord): "never" | "ask" {
  return record.outcome.type === "never" ? "never" : "ask";
}

/** What happens to a waiting entry once its ask closed. */
export type WaitingFate =
  /** The record has not arrived yet (or never will): keep the entry and look again. */
  | { kind: "wait" }
  /** Your click, or an outcome the Security group now shows: the entry leaves the bell. */
  | { kind: "forget" };

/**
 * The waiting entry's fate for the closed ask `record`. `origin` is the client the core reported for a
 * callback answer (`callbackAnswered`), used only while the record is not here yet: only this window's own answer
 * lets the entry go without a record.
 */
export function waitingFate(
  record: AskRecord | undefined,
  origin: Origin | undefined,
): WaitingFate {
  if (record === undefined) return origin === "ui" ? { kind: "forget" } : { kind: "wait" };
  return { kind: "forget" };
}
