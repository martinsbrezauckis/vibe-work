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

/// `roots` plus, on Windows, each drive root's Git Bash spelling (`C:\a` as `/c/a` and `/C/a`): the agent's commands
/// run in Git Bash and often name paths that way.
pub(super) fn with_bash_spellings(mut roots: Vec<String>) -> Vec<String> {
    if cfg!(windows) {
        let extra: Vec<String> = roots
            .iter()
            .filter_map(|root| {
                let plain = root.strip_prefix(r"\\?\").unwrap_or(root);
                let (drive, rest) = plain.split_once(":\\")?;
                let rest = rest.replace('\\', "/");
                (drive.len() == 1).then(|| {
                    [drive.to_ascii_lowercase(), drive.to_ascii_uppercase()]
                        .map(|d| format!("/{d}/{rest}"))
                })
            })
            .flatten()
            .collect();
        roots.extend(extra);
    }
    roots
}

/// A Git Bash drive path (`/c/a`) as the Windows path it names; other text, and every path off Windows, unchanged.
pub(super) fn native(text: &str) -> std::borrow::Cow<'_, str> {
    #[cfg(windows)]
    if let Some(native) = crate::msys_paths::native_drive(text) {
        return std::borrow::Cow::Owned(native);
    }
    std::borrow::Cow::Borrowed(text)
}
