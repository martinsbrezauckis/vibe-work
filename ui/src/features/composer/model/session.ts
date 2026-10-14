/**
 * What the composer reads from a session's state: the running turn (steer, queue and stop
 * act on it), the queued turns listed above the composer, and which user messages can anchor a rewind
 * or a branch.
 * Depends on: the transcript's JSON helpers (`model/entries`). Called by: the composer views and stores.
 * Invariants: pure; a session without state reads as idle with an empty queue.
 */
import type { JsonValue } from "../../../protocol/core";
import { type Obj, arr, obj, str } from "../../transcript/model/entries";

/** The id of the turn that is running, or `null` when the session is idle. */
export function runningTurnId(state: Obj | null, session: Obj | undefined): string | null {
  const status = obj(session?.["status"]);
  const active = str(status?.["activeTurnId"] ?? null);
  const type = str(status?.["type"] ?? null);
  if (active !== null && active !== "" && (type === "running" || type === "blocked")) return active;
  // Turns are the record when the session status lags (or was never patched, fake parity).
  const turns = arr(state?.["turns"]);
  for (let i = turns.length - 1; i >= 0; i -= 1) {
    const turn = obj(turns[i]);
    if (turn?.["status"] === "in_progress") return str(turn["id"]);
  }
  return null;
}

/** One queued turn as listed above the composer. */
export interface QueuedTurn {
  id: string;
  /** The text of its user entry (what the user typed). */
  text: string;
  /** Number of context, image and other non-text blocks it carries. */
  extras: number;
  createdAt: number;
}

function entryText(entry: Obj): { text: string; extras: number } {
  let text = "";
  let extras = 0;
  for (const block of arr(entry["content"])) {
    const b = obj(block);
    if (b?.["type"] === "text") text += str(b["text"] ?? null) ?? "";
    else extras += 1;
  }
  return { text, extras };
}

/** The queued turns of a session (`PublicSessionState.turnQueue.items`), oldest first. */
export function queuedTurns(state: Obj | null): QueuedTurn[] {
  const queue = obj(state?.["turnQueue"]);
  return arr(queue?.["items"]).flatMap((item: JsonValue) => {
    const it = obj(item);
    const id = str(it?.["id"] ?? null);
    if (it === null || id === null) return [];
    const user = arr(it["entries"])
      .map(obj)
      .find((e) => e?.["role"] === "user");
    const { text, extras } = user ? entryText(user) : { text: "", extras: 0 };
    const created = it["createdAt"];
    return [{ id, text, extras, createdAt: typeof created === "number" ? created : 0 }];
  });
}

/**
 * Why a user message cannot anchor a rewind or branch, or `null` when it can. Vibe resolves the anchor
 * only by a client-supplied id: ids Vibe minted itself
 * (`user-turn-<hash>-<n>`) fail with `fork_entry_not_found`, and injected context (`source: harness`) is
 * not a user message.
 */
export function anchorProblem(entry: Obj): string | null {
  const id = str(entry["id"] ?? null) ?? "";
  if (id === "") return "This message has no id.";
  if (entry["source"] === "harness") return "Context Vibe added cannot be rewound to.";
  if (id.startsWith("user-turn-")) {
    return "This message was sent before Vibe Work tracked it, so Vibe cannot find it to rewind or branch.";
  }
  return null;
}

/**
 * How the last turn ended when it did not complete (`interrupted` by Stop or a crash; `failed` with
 * Vibe's error), so the composer can offer Retry; `null` when it completed or nothing ran yet.
 */
export function lastTurnProblem(state: Obj | null): "interrupted" | "failed" | null {
  const turns = arr(state?.["turns"]);
  const last = obj(turns[turns.length - 1] ?? null);
  const status = last?.["status"];
  return status === "interrupted" || status === "failed" ? status : null;
}
