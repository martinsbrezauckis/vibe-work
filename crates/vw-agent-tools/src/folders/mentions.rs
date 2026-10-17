//! Quote-aware absolute mentions for the folder guard, including redirections and concatenated quotes.
//! Depends on std; called before the shell-segment backstop, preserving spaces and Windows separators.

pub(super) fn raw_mentions(text: &str, root: &str) -> Vec<String> {
    if root.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(c, '\'' | '"') && (quote.is_none() || quote == Some(c)) {
            quote = if quote.is_none() { Some(c) } else { None };
        } else if c == '\\' && quote.is_none() && chars.peek().is_some_and(|c| c.is_whitespace()) {
            // Bash's escaped whitespace is part of the path. Ordinary Windows backslashes stay literal.
            if let Some(next) = chars.next() {
                word.push(next);
            }
        } else if quote.is_none()
            && (c.is_whitespace() || matches!(c, ';' | '|' | '&' | ')' | '(' | '<' | '>' | '`'))
        {
            collect(&word, root, &mut out);
            word.clear();
        } else {
            word.push(c);
        }
    }
    collect(&word, root, &mut out);
    out
}

fn collect(word: &str, root: &str, out: &mut Vec<String>) {
    for (at, _) in word.match_indices(root) {
        out.push(word[at..].to_owned());
    }
}
