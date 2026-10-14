/**
 * Command registry for the palette (Ctrl+K): features register actions, the palette lists and
 * filters them. The fuzzy matching is adapted from ShellX's command palette: substrings rank before
 * subsequences, and ties remain stable.
 * Depends on: nothing. Called by: the shell (registers its actions), feature routes, the palette.
 * Invariants: ids are unique (a duplicate is refused); filtering never reorders equal scores
 * (registration order is kept); an empty query lists everything in registration order.
 */

/** One palette entry. */
export interface PaletteCommand {
  id: string;
  label: string;
  /** Group heading (`Actions`, `Sessions`, ...). */
  section: string;
  /** Extra searchable text (substring match only), e.g. a session's folder and model. */
  keywords?: string;
  /** Muted text on the right when there is no key combo (a session's project). */
  hint?: string;
  /** Combo shown on the right (`Mod+K` style string, formatted by the view). */
  keys?: string;
  run: () => void;
}

/** Scores how well `query` matches `target`: substring 100+, in-order subsequence 50+, else 0. */
export function scoreFuzzy(query: string, target: string): number {
  const q = query.toLowerCase().trim();
  const t = target.toLowerCase();
  if (q === "") return 1;
  if (t.includes(q)) return 100 + (q.length / t.length) * 10;
  let i = 0;
  for (let j = 0; j < t.length && i < q.length; j++) if (t[j] === q[i]) i++;
  return i === q.length ? 50 + (q.length / t.length) * 10 : 0;
}

/** Label match (fuzzy), or a plain substring hit in the keywords (ranked just below a label hit). */
export function scoreItem(query: string, item: PaletteCommand): number {
  const label = scoreFuzzy(query, item.label);
  if (item.keywords === undefined) return label;
  const words = query
    .toLowerCase()
    .split(/\s+/)
    .filter((w) => w !== "");
  const haystack = `${item.label}\n${item.keywords}`.toLowerCase();
  const inKeywords = words.length > 0 && words.every((w) => haystack.includes(w));
  return Math.max(label, inKeywords ? 40 : 0);
}

/** Entries matching `query`, best first, ties in input order. */
export function filterCommands(items: readonly PaletteCommand[], query: string): PaletteCommand[] {
  if (query.trim() === "") return [...items];
  return items
    .map((item, order) => ({ item, order, score: scoreItem(query, item) }))
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score || a.order - b.order)
    .map((x) => x.item);
}

/** Creates a registry. */
export function createCommands() {
  const items = new Map<string, PaletteCommand>();
  return {
    /** Registers a command; returns the function that removes it. Throws on a duplicate id. */
    register(command: PaletteCommand): () => void {
      if (items.has(command.id)) throw new Error(`command ${command.id} is already registered`);
      items.set(command.id, command);
      return () => {
        items.delete(command.id);
      };
    },
    /** All commands in registration order. */
    list(): readonly PaletteCommand[] {
      return [...items.values()];
    },
  };
}

/** The registry the app uses. */
export type Commands = ReturnType<typeof createCommands>;
