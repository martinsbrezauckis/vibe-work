/**
 * Find-in-place helpers: the same word rule as the core's search (every word, any order, ignoring
 * case), and marking the found words in rendered text with the CSS Custom Highlight API, so the transcript and the
 * File tab never have their React-owned DOM changed.
 * Depends on: nothing. Called by: the transcript's find bar, the File tab's find bar.
 * Invariants: marking only adds `Range`s to the two named highlights (`vw-find`, `vw-find-current`) and clears them
 * on cleanup; where the API is missing (an older engine) nothing is marked and the bars still jump to the match
 * (the match's row or line is outlined by a class instead).
 */

/** The query's words, lowercased (at most 8, like the core). */
export function findWords(query: string): string[] {
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter((w) => w !== "")
    .slice(0, 8);
}

/** True when `text` holds every word. */
export function holdsAll(text: string, words: readonly string[]): boolean {
  if (words.length === 0) return false;
  const lower = text.toLowerCase();
  return words.every((w) => lower.includes(w));
}

/** The CSS Custom Highlight registry, when this engine has it. */
interface HighlightRegistry {
  set(name: string, value: unknown): void;
  delete(name: string): void;
}
function registry(): { highlights: HighlightRegistry; make: (ranges: Range[]) => unknown } | null {
  const css = (globalThis as { CSS?: { highlights?: HighlightRegistry } }).CSS;
  const Ctor = (globalThis as { Highlight?: new (...ranges: Range[]) => unknown }).Highlight;
  if (css?.highlights === undefined || Ctor === undefined) return null;
  return { highlights: css.highlights, make: (ranges) => new Ctor(...ranges) };
}

/** Ranges of every occurrence of every word in the text nodes under `root`. */
function rangesIn(root: Element, words: readonly string[]): Range[] {
  const out: Range[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const lower = (node.nodeValue ?? "").toLowerCase();
    for (const word of words) {
      let at = lower.indexOf(word);
      while (at !== -1 && out.length < 2000) {
        const range = document.createRange();
        range.setStart(node, at);
        range.setEnd(node, at + word.length);
        out.push(range);
        at = lower.indexOf(word, at + word.length);
      }
    }
  }
  return out;
}

/**
 * Marks the words in every element of `all` (dim) and in `current` (strong). Returns the cleanup. Does nothing on an
 * engine without the API.
 */
export function markWords(
  all: readonly Element[],
  current: Element | null,
  words: readonly string[],
): () => void {
  const api = registry();
  if (api === null || words.length === 0) return () => undefined;
  api.highlights.set("vw-find", api.make(all.flatMap((el) => rangesIn(el, words))));
  api.highlights.set("vw-find-current", api.make(current === null ? [] : rangesIn(current, words)));
  return () => {
    api.highlights.delete("vw-find");
    api.highlights.delete("vw-find-current");
  };
}
