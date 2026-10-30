/**
 * Fuzzy matching for the @ menu: the query's characters must appear in order in the text, ignoring case. Pure; no
 * I/O. Depends on nothing. Called by the mention model for file names and session titles.
 * Invariants: the same rules as the core's file-name ranking (`crates/vw-core-api/src/search_fuzzy.rs`), so the rows
 * the window keeps from a core answer are the ones the core ranked: a character right after the previous match, at
 * the start of the text or after a separator scores higher; an empty query matches everything with score 0.
 */

/** Characters after which a match counts as the start of a word. */
const SEPARATORS = new Set(["/", "\\", "_", "-", ".", " ", "(", ")", "[", "]"]);

/**
 * The match score of `query` in `text`, or `null` when the query's characters do not all appear in order.
 * Higher is better. Each matched character scores 1, plus 4 when it follows the previous match directly, plus 3 at
 * the start of the text, after a separator or at a lower-to-upper case change; each skipped character between two
 * matches costs 1 (at most 3 per gap), and a match starting at the very first character scores 2 more.
 */
export function fuzzyScore(text: string, query: string): number | null {
  if (query === "") return 0;
  const chars = Array.from(text);
  const lower = chars.map((c) => c.toLowerCase());
  const wanted = Array.from(query.toLowerCase());
  let score = 0;
  let at = 0;
  let previous = -1;
  for (const want of wanted) {
    while (at < lower.length && lower[at] !== want) at += 1;
    if (at >= lower.length) return null;
    score += 1;
    if (previous >= 0 && at === previous + 1) score += 4;
    else if (previous >= 0) score -= Math.min(3, at - previous - 1);
    const before = at === 0 ? undefined : chars[at - 1];
    const current = chars[at] ?? "";
    const caseStep =
      before !== undefined &&
      before === before.toLowerCase() &&
      before !== before.toUpperCase() &&
      current !== current.toLowerCase();
    if (before === undefined || SEPARATORS.has(before) || caseStep) score += 3;
    if (at === 0 && previous < 0) score += 2;
    previous = at;
    at += 1;
  }
  return score;
}

/** The part of a relative path a query is matched against: the whole path when the query names a folder (has a `/`), else the name. */
export function fuzzyTarget(path: string, query: string): string {
  return query.includes("/") ? path : path.slice(path.lastIndexOf("/") + 1);
}
