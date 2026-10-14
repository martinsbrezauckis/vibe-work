//! Literal command segments shared by the Git and process guardrails.
//! Depends on no shell parser. Called by `gitparse` and `killparse`.
//!
//! This intentionally knows only five separators. Quotes, substitutions, here-documents and
//! scripts are ordinary bytes, not nested command languages.

/// Split at literal `;`, `&&`, `||`, `|` and newlines, then split each piece on whitespace.
#[must_use]
pub fn segments(command: &str) -> Vec<Vec<&str>> {
    let bytes = command.as_bytes();
    let mut start = 0;
    let mut parts = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let step = match bytes[at] {
            b';' | b'\n' | b'|' => {
                if bytes[at] == b'|' && bytes.get(at + 1) == Some(&b'|') {
                    2
                } else {
                    1
                }
            }
            b'&' if bytes.get(at + 1) == Some(&b'&') => 2,
            _ => {
                at += 1;
                continue;
            }
        };
        parts.push(command[start..at].split_whitespace().collect());
        at += step;
        start = at;
    }
    parts.push(command[start..].split_whitespace().collect());
    parts
}

/// The literal executable basename, including a Windows `.exe` suffix.
/// This does not remove quotes or expand variables.
#[must_use]
pub fn executable_name(word: &str) -> &str {
    let basename = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let suffix = basename.get(basename.len().saturating_sub(4)..);
    if suffix.is_some_and(|s| s.eq_ignore_ascii_case(".exe")) {
        &basename[..basename.len() - 4]
    } else {
        basename
    }
}

/// Skip plain assignments and the small set of wrappers that pass through a command name.
/// An option or value consumed by a wrapper is not itself a command.
#[must_use]
pub fn command_words<'a>(mut words: &'a [&'a str]) -> &'a [&'a str] {
    loop {
        words = &words[words.iter().take_while(|w| assignment(w)).count()..];
        let Some((&first, rest)) = words.split_first() else {
            return words;
        };
        let Some(next) = wrapper_args(executable_name(first), rest) else {
            return words;
        };
        words = next;
    }
}

fn wrapper_args<'a>(name: &str, rest: &'a [&'a str]) -> Option<&'a [&'a str]> {
    let name = name.to_ascii_lowercase();
    let at = match name.as_str() {
        "sudo" | "doas" => sudo_options(rest),
        "env" => env_options(rest),
        "nohup" => usize::from(rest.first() == Some(&"--")),
        "command" => rest
            .iter()
            .take_while(|w| matches!(**w, "-p" | "--"))
            .count(),
        "exec" => exec_options(rest),
        "time" => time_options(rest),
        "timeout" => timeout_options(rest).saturating_add(1),
        "nice" if rest.first() == Some(&"-n") => 2,
        "nice" => usize::from(rest.first().is_some_and(|w| w.starts_with('-'))),
        _ => return None,
    };
    Some(&rest[at.min(rest.len())..])
}

fn sudo_options(words: &[&str]) -> usize {
    let mut at = 0;
    while at < words.len() && words[at].starts_with('-') {
        let value = matches!(
            words[at],
            "-u" | "-g" | "-h" | "-p" | "-C" | "--user" | "--group" | "--host" | "--prompt"
        );
        at += if value { 2 } else { 1 };
    }
    at
}

fn env_options(words: &[&str]) -> usize {
    let mut at = 0;
    while at < words.len() {
        if assignment(words[at])
            || matches!(words[at], "-i" | "--ignore-environment" | "-0" | "--null")
        {
            at += 1;
        } else if matches!(
            words[at],
            "-u" | "--unset" | "-C" | "--chdir" | "-S" | "--split-string"
        ) {
            at += 2;
        } else if words[at].starts_with('-') {
            at += 1;
        } else {
            break;
        }
    }
    at
}

fn exec_options(words: &[&str]) -> usize {
    let mut at = 0;
    while at < words.len() {
        match words[at] {
            "-a" => at += 2,
            "-c" | "-l" | "--" => at += 1,
            _ => break,
        }
    }
    at
}

fn time_options(words: &[&str]) -> usize {
    let mut at = 0;
    while at < words.len() {
        match words[at] {
            "-f" | "--format" | "-o" | "--output" => at += 2,
            "-p" | "--portability" | "-a" | "--append" | "-v" | "--verbose" | "--" => at += 1,
            _ => break,
        }
    }
    at
}

fn timeout_options(words: &[&str]) -> usize {
    let mut at = 0;
    while at < words.len() {
        match words[at] {
            "-s" | "--signal" | "-k" | "--kill-after" => at += 2,
            "--foreground" | "--preserve-status" | "--verbose" | "--" => at += 1,
            arg if arg.starts_with("--signal=") || arg.starts_with("--kill-after=") => at += 1,
            _ => break,
        }
    }
    // The first non-option word is the duration, followed by the command.
    at
}

fn assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
