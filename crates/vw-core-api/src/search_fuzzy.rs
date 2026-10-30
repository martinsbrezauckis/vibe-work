//! Fuzzy ranking of file names for the composer's @ menu: the query's characters must appear in order, ignoring case.
//! Pure functions, no I/O.
//!
//! Depends on: std. Called by: `search_files` (name-only search).
//! Invariants: the same rules as the window's `ui/src/features/composer/model/fuzzy.ts` (which keeps or drops the
//! rows of an answer by them): each matched character scores 1, plus 4 right after the previous match, plus 3 at the
//! start, after a separator or at a lower-to-upper case change, plus 2 when the match starts at the first character;
//! each skipped character between two matches costs 1, at most 3 per gap. A query with a `/` is matched against the
//! whole relative path, any other query against the file name only.

/// Characters after which a match counts as the start of a word.
const SEPARATORS: &[char] = &['/', '\\', '_', '-', '.', ' ', '(', ')', '[', ']'];

/// The first lowercase character of `c` (a character whose lowercase form is longer compares by its first part).
fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// The match score of `query` in `text` (higher is better), or `None` when its characters do not all appear in order.
/// An empty query scores 0.
pub(crate) fn score(text: &str, query: &str) -> Option<i64> {
    let chars: Vec<char> = text.chars().collect();
    let mut total: i64 = 0;
    let mut at = 0usize;
    let mut previous: Option<usize> = None;
    for want in query.chars().map(lower) {
        while at < chars.len() && lower(chars[at]) != want {
            at += 1;
        }
        let current = *chars.get(at)?;
        total += 1;
        match previous {
            Some(p) if at == p + 1 => total += 4,
            Some(p) => total -= i64::try_from((at - p - 1).min(3)).unwrap_or(3),
            None => {}
        }
        let before = at.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let case_step = before.is_some_and(|b| b.is_lowercase() && current.is_uppercase());
        if before.is_none_or(|b| SEPARATORS.contains(&b)) || case_step {
            total += 3;
        }
        if at == 0 && previous.is_none() {
            total += 2;
        }
        previous = Some(at);
        at += 1;
    }
    Some(total)
}

/// The part of `rel` (a `/`-separated relative path) that `query` is matched against.
pub(crate) fn target<'a>(rel: &'a str, query: &str) -> &'a str {
    if query.contains('/') {
        rel
    } else {
        rel.rsplit('/').next().unwrap_or(rel)
    }
}
