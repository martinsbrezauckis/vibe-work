/**
 * What voice chat reads from a session: its
 * finished turns and how each ended, the final answer of a turn, whether it is busy (a running turn or queued
 * items), and the open approvals and questions as short spoken prompts.
 * Depends on: the transcript entry helpers, the composer session model, sessionSummary (types), voiceChatText.
 * Called by: state/voiceChat, tests.
 * Invariants: pure; reads the core model only; a session without state reads as idle with nothing finished.
 */
import type { JsonValue } from "../protocol/core";
import { questionsOf } from "../features/transcript/model/callbacks";
import {
  type Obj,
  arr,
  entryType,
  messageText,
  obj,
  roleOf,
  statusOf,
  str,
  turnIdOf,
} from "../features/transcript/model/entries";
import { queuedTurns, runningTurnId } from "../features/composer/model/session";
import { approvalPrompt, questionPrompt } from "../features/voice/voiceChatText";
import { type SessionEntry, statusType } from "./sessionSummary";

/**
 * Turn statuses that mean "finished" (Vibe `PublicTurnStatus`, generated `common_models_4.ts`), with what voice chat
 * does for each: speak the answer, say it failed, or say nothing (stopped by the user).
 */
const ENDED: Readonly<Record<string, "answer" | "failed" | "quiet">> = {
  completed: "answer",
  failed: "failed",
  interrupted: "quiet",
};

/** A finished turn: its id and how it ended. */
export interface EndedTurn {
  id: string;
  how: "answer" | "failed" | "quiet";
}

/** The finished turns of a session, oldest first. */
export function endedTurns(entry: SessionEntry | undefined): EndedTurn[] {
  return arr(entry?.state?.["turns"]).flatMap((t) => {
    const turn = obj(t);
    const id = str(turn?.["id"] ?? null);
    const how = ENDED[str(turn?.["status"] ?? null) ?? ""];
    return id === null || how === undefined ? [] : [{ id, how }];
  });
}

/** The final answer of `turnId`: its last non-empty assistant message (earlier ones are progress notes). */
export function finalAnswer(entry: SessionEntry | undefined, turnId: string): string {
  let last = "";
  for (const raw of arr(entry?.state?.["history"])) {
    const e = obj(raw);
    if (e === null || turnIdOf(e) !== turnId || roleOf(e) !== "assistant") continue;
    const text = messageText(e);
    if (text.trim() !== "") last = text;
  }
  return last;
}

/** Assistant text of the running turn, including its growing text while the model streams. */
export function runningAnswer(entry: SessionEntry | undefined): string {
  if (entry === undefined) return "";
  const id = runningTurnId(entry.state ?? null, entry.session);
  return id === null ? "" : finalAnswer(entry, id);
}

/** Text messages of a turn in history order, with identity for incremental speech deduplication. */
export function turnAnswers(entry: SessionEntry | undefined, turnId: string) {
  return arr(entry?.state?.["history"]).flatMap((raw) => {
    const e = obj(raw);
    if (e === null || turnIdOf(e) !== turnId || roleOf(e) !== "assistant") return [];
    return [
      {
        id: str(e["id"] ?? null) ?? "",
        text: messageText(e),
        done: e["generationStatus"] === "completed",
      },
    ];
  });
}

/** The turn currently producing text, if any. */
export function voiceTurnId(entry: SessionEntry | undefined): string | null {
  return entry === undefined ? null : runningTurnId(entry.state ?? null, entry.session);
}

/** True while the session runs a turn or has queued turns waiting to start. */
export function sessionBusy(entry: SessionEntry | undefined): boolean {
  if (entry === undefined) return false;
  const state = entry.state ?? null;
  return runningTurnId(state, entry.session) !== null || queuedTurns(state).length > 0;
}

/** True when the session was archived (voice chat then ends: the session is closed). */
export function sessionClosed(entry: SessionEntry): boolean {
  const archivedAt = entry.session["archivedAt"];
  return (
    statusType(entry.session) === "archived" || (archivedAt !== undefined && archivedAt !== null)
  );
}

/** An open approval or question, with the short text the voice says for it. */
export interface OpenPrompt {
  callbackId: string;
  text: string;
}

/** The open callbacks of a session as spoken prompts, in history order. */
export function openPrompts(entry: SessionEntry | undefined): OpenPrompt[] {
  if (entry === undefined) return [];
  const open = new Set(entry.openCallbackIds);
  return arr(entry.state?.["history"]).flatMap((raw: JsonValue) => {
    const e: Obj | null = obj(raw);
    if (e === null || entryType(e) !== "callback") return [];
    const id = str(e["callbackId"] ?? null) ?? str(e["id"] ?? null);
    if (id === null || (!open.has(id) && statusOf(e) !== "open")) return [];
    const question = questionsOf(e).questions[0]?.question ?? "";
    const isQuestion = obj(e["detail"])?.["kind"] === "user_input";
    const text = isQuestion
      ? questionPrompt(question)
      : approvalPrompt(str(e["title"] ?? null) ?? "");
    return [{ callbackId: id, text }];
  });
}
