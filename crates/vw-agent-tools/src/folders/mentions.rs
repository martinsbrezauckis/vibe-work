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
    if cfg!(windows) {
        // Windows reads `C:\a`, `c:/A` and `C:\a/b` as one folder: every start where the root follows that way.
        for (at, _) in word.char_indices() {
            if starts_with_windows_path(&word[at..], root) {
                out.push(word[at..].to_owned());
            }
        }
    } else {
        for (at, _) in word.match_indices(root) {
            out.push(word[at..].to_owned());
        }
    }
}

/// `text` starts with `root` as Windows compares path text: letters in either case, `/` and `\` alike.
fn starts_with_windows_path(text: &str, root: &str) -> bool {
    let mut text = text.chars();
    root.chars().all(|r| {
        text.next().is_some_and(|t| {
            t == r
                || matches!((t, r), ('/' | '\\', '/' | '\\'))
                || t.to_uppercase().eq(r.to_uppercase())
        })
    })
}

/// `roots` plus, on Windows, each drive root without the verbatim prefix (`\\?\C:\a` as `C:\a`) and in its Git
/// Bash spelling (`/c/a`): the agent's commands run in Git Bash and name paths either way. The scan matches these
/// regardless of case and separator, so `c:/a`, `C:\a/b` and `/C/a` are covered too.
pub(super) fn with_windows_spellings(mut roots: Vec<String>) -> Vec<String> {
    if cfg!(windows) {
        let extra: Vec<String> = roots
            .iter()
            .filter_map(|root| {
                let plain = root
                    .strip_prefix(r"\\?\")
                    .unwrap_or(root)
                    .replace('/', "\\");
                let (drive, rest) = plain.split_once(":\\")?;
                let bash = format!(
                    "/{}/{}",
                    drive.to_ascii_lowercase(),
                    rest.replace('\\', "/")
                );
                (drive.len() == 1).then(|| [plain.clone(), bash])
            })
            .flatten()
            .collect();
        roots.extend(extra);
        let mut seen = std::collections::HashSet::new();
        roots.retain(|root| seen.insert(root.to_lowercase()));
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
