/**
 * Plain speech cleanup and final turn text extraction with zero voice runtime dependencies.
 * Depends on: transcript entry helpers, sessionSummary types. Called by: replyAnnouncements,
 * voice/speechText, voice/voiceChatText, state/voiceChatTurns.
 * Invariants: pure string transforms; model text is untrusted and only ever stripped for speech/screen-reader output.
 */
import { arr, messageText, obj, roleOf, turnIdOf } from "../features/transcript/model/entries";
import type { SessionEntry } from "./sessionSummary";

/**
 * Removes emphasis marks at word edges only (`**bold**`, `_it_`, `~~gone~~`): `snake_case`, `a*b` and `a > b` stay.
 * Shared with narration (`speakable`) and voice chat (`spokenReply`).
 */
export function stripEmphasis(text: string): string {
  return text
    .replace(/(^|[\s(])(\*{1,3}|_{1,3}|~~)(?=\S)/g, "$1")
    .replace(/(\S)(\*{1,3}|_{1,3}|~~)(?=$|[\s.,;:!?)])/g, "$1");
}

/** Narration cleanup; one-time readings name skipped code blocks and keep up to 10,000 characters. */
export function speakable(text: string, limit = 2_000, describeCode = false): string {
  const plain = text
    .replace(/```[^\n`]*\n?([\s\S]*?)```/g, (_block: string, code: string) =>
      describeCode ? ` (code block, ${String(code.trimEnd().split("\n").length)} lines) ` : " ",
    )
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/^[ \t]*(?:#{1,6}[ \t]+|>[ \t]?|[-*+][ \t]+)/gm, "")
    .replace(/^[ \t]*\|.*$/gm, " ");
  return Array.from(stripEmphasis(plain).replace(/\s+/g, " ").trim()).slice(0, limit).join("");
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
