//! Word matching for the search: pure functions, no I/O.
//!
//! A query is split on whitespace into words; a text matches when every word occurs in it, ignoring case (Unicode
//! lowercase per character). The first occurrence of the first word anchors the snippet. A text that holds the
//! whole query as written (words joined by single spaces) is a phrase match, ranked first.
//!
//! Depends on: std. Called by: `search_index` (messages), `search_files` (file lines and names).
//! Invariants: offsets returned are byte offsets into the ORIGINAL text on char boundaries (the lowercase copy may
//! differ in length; a map from lowercase bytes to original bytes keeps them right); a snippet never splits a char.

/// Most words a query may have (more are ignored).
const MAX_WORDS: usize = 8;
/// Characters of context kept before the match in a snippet.
const BEFORE: usize = 50;
/// Longest snippet in characters.
const SNIPPET: usize = 160;

/// A prepared query.
#[derive(Debug, Clone)]
pub(crate) struct Query {
    words: Vec<String>,
    phrase: String,
}

impl Query {
    /// The query's words, lowercased; `None` for an empty query.
    pub(crate) fn new(text: &str) -> Option<Self> {
        let words: Vec<String> = text
            .split_whitespace()
            .take(MAX_WORDS)
            .map(str::to_lowercase)
            .collect();
        if words.is_empty() {
            return None;
        }
        let phrase = words.join(" ");
        Some(Self { words, phrase })
    }
}

/// A lowercase copy of `text` and, for each of its bytes, the byte offset in `text` of the char it came from.
fn lowered(text: &str) -> (String, Vec<usize>) {
    if text.is_ascii() {
        return (text.to_ascii_lowercase(), (0..text.len()).collect());
    }
    let mut out = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.len());
    for (at, c) in text.char_indices() {
        for l in c.to_lowercase() {
            let before = out.len();
            out.push(l);
            map.extend(std::iter::repeat_n(at, out.len() - before));
        }
    }
    (out, map)
}

/// [`find`] for a text whose ASCII lowercase copy the caller keeps (`ascii_lower`, same byte offsets): no lowercase
/// copy is made per search. `None` for `ascii_lower` falls back to [`find`].
pub(crate) fn find_kept(
    text: &str,
    ascii_lower: Option<&str>,
    query: &Query,
) -> Option<(usize, bool)> {
    let Some(lower) = ascii_lower else {
        return find(text, query);
    };
    let mut first: Option<usize> = None;
    for word in &query.words {
        let at = lower.find(word.as_str())?;
        first.get_or_insert(at);
    }
    Some((first.unwrap_or(0), lower.contains(query.phrase.as_str())))
}

/// Where `query` matches `text`: the original byte offset of the first word's first occurrence, and whether the
/// whole query occurs as a phrase. `None` when a word is missing.
pub(crate) fn find(text: &str, query: &Query) -> Option<(usize, bool)> {
    let (lower, map) = lowered(text);
    let mut first: Option<usize> = None;
    for word in &query.words {
        let at = lower.find(word.as_str())?;
        if first.is_none() {
            first = Some(at);
        }
    }
    let phrase = lower.contains(query.phrase.as_str());
    let at = first.and_then(|i| map.get(i).copied()).unwrap_or(0);
    Some((at, phrase))
}

/// The text around byte offset `at` on one line: newlines and tabs become spaces, `…` marks a cut.
pub(crate) fn snippet(text: &str, at: usize) -> String {
    let at = at.min(text.len());
    let start_char = text[..at].chars().count().saturating_sub(BEFORE);
    let chars: Vec<char> = text.chars().collect();
    let end_char = (start_char + SNIPPET).min(chars.len());
    let mut out = String::new();
    if start_char > 0 {
        out.push('…');
    }
    for &c in &chars[start_char..end_char] {
        out.push(if c.is_whitespace() { ' ' } else { c });
    }
    if end_char < chars.len() {
        out.push('…');
    }
    // Runs of spaces from blank lines read badly in one line.
    let mut collapsed = String::with_capacity(out.len());
    let mut space = false;
    for c in out.chars() {
        if c == ' ' {
            if !space {
                collapsed.push(c);
            }
            space = true;
        } else {
            collapsed.push(c);
            space = false;
        }
    }
    collapsed.trim().to_owned()
}
