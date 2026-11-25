/**
 * Prebuilt search index matching and ranking for Settings catalog entries (highlighting: state/highlight.ts).
 * Prebuilds lowercased haystacks once at initialization so queries stay fast with zero allocations.
 * Depends on: `../../state/highlight` (`queryWords`), `./actionLabels` (`holdsWordPrefix`). Called by: `./catalog`.
 */
import { indexOfWord, queryWords, startsWordsOnly } from "../../state/highlight";
import { holdsWordPrefix } from "./actionLabels";

/** Minimal setting shape required for indexing and search filtering. */
export interface IndexableSetting {
  readonly id: string;
  readonly category: string;
  readonly label: string;
  readonly description: string;
  readonly keywords: string;
  readonly synonyms?: readonly string[];
  readonly actions?: readonly string[];
}

/** Precomputed search record built once per entry at module init. */
export interface PreparedSetting<T extends IndexableSetting> {
  readonly entry: T;
  readonly label: string;
  readonly synonymsText: string;
  readonly actions: readonly string[];
  readonly baseHay: string;
}

function holds(text: string, word: string, wordStart = false): boolean {
  if (indexOfWord(text, word, 0, wordStart) !== -1) return true;
  return word.length > 3 && word.endsWith("s") && text.includes(word.slice(0, -1));
}

/** Precomputes lowercase search haystacks once from static catalog entries. */
export function buildPreparedIndex<T extends IndexableSetting>(
  entries: readonly T[],
  categoryLabel: (id: string) => string,
): readonly PreparedSetting<T>[] {
  return entries.map((entry) => {
    const label = entry.label.toLowerCase();
    const synonymsText = (entry.synonyms ?? []).join(" | ").toLowerCase();
    const actions = (entry.actions ?? []).map((a) => a.toLowerCase());
    const baseHay =
      `${label} ${entry.description} ${entry.keywords} ${synonymsText} ${categoryLabel(entry.category)}`.toLowerCase();
    return { entry, label, synonymsText, actions, baseHay };
  });
}

/** Searches and ranks a prepared index using browser-availability gates and single-word/action rules. */
export function filterAndRankSettings<T extends IndexableSetting>(
  prepared: readonly PreparedSetting<T>[],
  query: string,
  chromeAvailable: boolean,
  edgeAvailable: boolean,
  firefoxAvailable: boolean,
  safariAvailable: boolean,
): T[] {
  const words = queryWords(query);
  if (words.length === 0) return [];
  // One or two characters match only where a word starts ("al" is not the inside of "Mistral").
  const wordStart = startsWordsOnly(query);
  const hits = prepared.filter(({ entry: e, actions, baseHay }) => {
    if (!chromeAvailable && (e.id.startsWith("chrome-control") || e.id === "chrome-composer"))
      return false;
    if (!firefoxAvailable && (e.id.startsWith("firefox-control") || e.id === "firefox-composer"))
      return false;
    if (!edgeAvailable && (e.id.startsWith("edge-control") || e.id === "edge-composer"))
      return false;
    if (!safariAvailable && (e.id.startsWith("safari-control") || e.id === "safari-composer"))
      return false;
    return words.every(
      (w) => holds(baseHay, w, wordStart) || actions.some((a) => holdsWordPrefix(a, w)),
    );
  });
  const rank = (item: PreparedSetting<T>): number => {
    if (words.every((w) => holds(item.label, w, wordStart))) return 0;
    if (
      words.every((w) => holds(item.synonymsText, w, wordStart)) ||
      item.actions.some((a) => words.every((w) => holdsWordPrefix(a, w)))
    ) {
      return 1;
    }
    return words.some((w) => holds(item.label, w, wordStart)) ? 2 : 3;
  };
  return hits
    .map((item) => ({ item, r: rank(item) }))
    .sort((a, b) => a.r - b.r)
    .map(({ item }) => item.entry);
}
