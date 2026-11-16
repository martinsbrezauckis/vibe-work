/** Prompt recall state and caret decisions; composer wiring calls it before ordinary key handling. */
import type { KeyInfo } from "../features/composer/model/keys";
import type { CoreController } from "./coreStore";
import type { ComposerStore } from "./composerStore";

/** A navigation snapshot, reset by any edit; caret movement within a prompt keeps the snapshot. */
export class PromptHistory {
  private entries: readonly string[] = [];
  private index: number | null = null;
  private recalled = "";

  replace(entries: readonly string[]): void {
    if (this.entries.length === entries.length && this.entries.every((e, i) => e === entries[i]))
      return;
    this.entries = entries;
    this.edit();
  }
  edit(): void {
    this.index = null;
    this.recalled = "";
  }
  key(key: KeyInfo, text: string, start: number, end: number): string | null {
    if (
      key.shiftKey ||
      key.ctrlKey ||
      key.altKey ||
      key.metaKey ||
      key.isComposing ||
      start !== end
    )
      return null;
    if (key.key !== "ArrowUp" && key.key !== "ArrowDown") return null;
    if (this.index !== null && text !== this.recalled) this.edit();
    if (this.index === null && (text !== "" || key.key !== "ArrowUp")) return null;
    const up = key.key === "ArrowUp";
    if (up ? text.slice(0, start).includes("\n") : text.slice(end).includes("\n")) return null;
    const next = (this.index ?? this.entries.length) + (up ? -1 : 1);
    if (next < 0 || next > this.entries.length) return null;
    if (next === this.entries.length) {
      this.edit();
      return "";
    }
    const entry = this.entries[next];
    if (entry === undefined) return null;
    this.index = next;
    this.recalled = entry;
    return entry;
  }
}

/** One prompt search result: the entry's position in the history list (oldest first) and its text. */
export interface HistoryMatch {
  readonly index: number;
  readonly text: string;
}

/** At most this many prompts are listed by a search. */
export const HISTORY_SEARCH_LIMIT = 50;

/**
 * Prompt search (Ctrl+R): the entries that contain every word of `query` anywhere, ignoring case, newest first.
 * A prompt sent several times is listed once, at its newest place. An empty query lists the newest prompts.
 */
export function searchHistory(entries: readonly string[], query: string): HistoryMatch[] {
  const words = query
    .toLocaleLowerCase()
    .split(/\s+/u)
    .filter((word) => word !== "");
  const seen = new Set<string>();
  const matches: HistoryMatch[] = [];
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const text = entries[index];
    if (text === undefined || seen.has(text)) continue;
    seen.add(text);
    const lower = text.toLocaleLowerCase();
    if (!words.every((word) => lower.includes(word))) continue;
    matches.push({ index, text });
    if (matches.length === HISTORY_SEARCH_LIMIT) break;
  }
  return matches;
}

/**
 * Refreshes from the CLI file or records an accepted submission with its chat (the core keeps a temporary chat's
 * prompts out of the file); optional recall never blocks sending.
 */
export async function syncPromptHistory(
  core: CoreController,
  composer: ComposerStore,
  sent?: { text: string; sessionId: string },
): Promise<void> {
  try {
    const answer = await core.run({
      type: "promptHistory",
      ...(sent === undefined ? {} : { text: sent.text, sessionId: sent.sessionId }),
    });
    if (answer.type === "promptHistory") composer.history.set(() => answer.entries);
  } catch {
    /* A missing/unreadable CLI history leaves the current recall list available. */
  }
}
