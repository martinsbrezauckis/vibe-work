/**
 * Plain search-term splitting and case-insensitive substring match spans (`queryWords`, `highlightRuns`).
 * Depends on: nothing. Called by: components/Highlight, features/settings/catalog, features/settings/searchIndex.
 */

/** Non-empty lowercase query words separated by whitespace. */
export function queryWords(query: string): string[] {
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter((w) => w !== "");
}

/** A query of one or two characters only matches where a word starts; longer ones match anywhere. */
export function startsWordsOnly(query: string): boolean {
  return query.trim().length <= 2;
}

/** Whether the character before `at` ends a word, so a match at `at` starts one. */
export function startsWordAt(text: string, at: number): boolean {
  return at === 0 || !/[\p{L}\p{N}]/u.test(text.charAt(at - 1));
}

/** Index of the first match of `word` in `text` at or after `from`; with `wordStart` only where a word begins. */
export function indexOfWord(text: string, word: string, from: number, wordStart: boolean): number {
  let at = text.indexOf(word, from);
  while (wordStart && at !== -1 && !startsWordAt(text, at)) at = text.indexOf(word, at + 1);
  return at;
}

/**
 * Splits `text` into plain and matched runs for highlighting (case-insensitive, every query word). Adjacent and
 * overlapping matches merge. With `shortAtWordStart` a query of one or two characters marks only where a word starts.
 */
export function highlightRuns(
  text: string,
  query: string,
  shortAtWordStart = false,
): { text: string; hit: boolean }[] {
  const words = queryWords(query);
  const lower = text.toLowerCase();
  const wordStart = shortAtWordStart && startsWordsOnly(query);
  const marks: boolean[] = new Array<boolean>(text.length).fill(false);
  for (const w of words) {
    let at = indexOfWord(lower, w, 0, wordStart);
    while (at !== -1) {
      for (let i = at; i < at + w.length; i++) marks[i] = true;
      at = indexOfWord(lower, w, at + w.length, wordStart);
    }
  }
  const runs: { text: string; hit: boolean }[] = [];
  for (let i = 0; i < text.length; i++) {
    const hit = marks[i] === true;
    const last = runs[runs.length - 1];
    if (last !== undefined && last.hit === hit) last.text += text.charAt(i);
    else runs.push({ text: text.charAt(i), hit });
  }
  return runs;
}
