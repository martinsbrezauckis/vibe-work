/**
 * Find in this conversation: which transcript rows hold the words. Pure; the bar
 * (`FindBar.tsx`) scrolls and marks.
 * Depends on: build (`Row`), entries (`messageText`, `entryId`), search/findText. Called by: FindBar.
 * Invariants: only your messages and the agent's answers are searched (the same text the core's message index
 * holds); matches are in row order.
 */
import { findWords, holdsAll } from "../../search/findText";
import type { Row } from "./build";
import { entryId, messageText } from "./entries";

/** One message that holds every word. */
export interface FindMatch {
  /** The row's index in the transcript list. */
  index: number;
  /** The message's entry id (`data-entry-id` in the DOM). */
  entryId: string;
}

/** The matches of `query` in `rows`. */
export function findMatches(rows: readonly Row[], query: string): FindMatch[] {
  const words = findWords(query);
  if (words.length === 0) return [];
  const out: FindMatch[] = [];
  rows.forEach((row, index) => {
    if (row.kind !== "user" && row.kind !== "assistant") return;
    if (holdsAll(messageText(row.entry), words)) out.push({ index, entryId: entryId(row.entry) });
  });
  return out;
}
