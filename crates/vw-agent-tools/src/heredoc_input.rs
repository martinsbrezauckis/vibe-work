//! Here-document bodies that are a Python script's input, not shell commands, for the folder guard (so Python's
//! `(w - side) // 2` in `python3 - <<'EOF'` is division, not the path `//`).
//! Depends on the tokenizer's `Segment::here_doc_of`; called by `folders::check_segments`.
//! Invariants (a security boundary: fail closed): only the body of exactly `python3 - <<'EOF'` (quoted delimiter,
//! closing line, plain quoting, no redirection, pipe or process substitution: the tokenizer's marking and
//! [`python_input`]) counts; in it only an unquoted run of `/` with no string literal beside it stops being a path.
//! Under plain quoting the shell's reading of quotes is Python's, and an unquoted `/` in a Python program that runs
//! is always division (Python has no other unquoted `/`, and a program with a syntax error never starts); a `/` next
//! to a string literal may join a path (`Path.home() / 'x'`) and stays checked. Perl, Ruby, JavaScript and others
//! have `/`-delimited quotes or regexes, and `cat`/`tee` bodies may be run later, so their bodies are read as before.
//! Every other word, redirection and command after the body is checked as before.

use crate::killparse_shell::{Segment, Word};

/// For each segment: true when it is a line of a here-document that is a Python script's input ([`python_input`]).
pub(crate) fn inputs(parsed: &[Segment]) -> Vec<bool> {
    parsed
        .iter()
        .map(|segment| {
            segment
                .here_doc_of
                .is_some_and(|owner| python_input(parsed, owner))
        })
        .collect()
}

/// `words[index]` is a Python division operator (`/`, `//`): an unquoted run of `/` with no string literal beside it.
pub(crate) fn operator(words: &[Word], index: usize) -> bool {
    let word = &words[index];
    let beside_literal = [index.checked_sub(1), Some(index + 1)]
        .into_iter()
        .flatten()
        .filter_map(|i| words.get(i))
        .any(|w| w.quoted);
    !word.quoted && !word.text.is_empty() && word.text.chars().all(|c| c == '/') && !beside_literal
}

/// True when the here-document opened by `parsed[owner]` is the script a Python interpreter reads, in exactly the
/// form `python3 - <<'EOF'` (`python`, `python3.12`, `pypy3`; the `-` may be left out): no other argument, no
/// assignment or wrapper before it, no redirection of its own, and its output goes to no other command (the next
/// segment that is not a here-document line is not piped).
fn python_input(parsed: &[Segment], owner: usize) -> bool {
    let Some(segment) = parsed.get(owner) else {
        return false;
    };
    let (Some(first), rest) = (
        segment.words.first(),
        segment.words.get(1..).unwrap_or_default(),
    ) else {
        return false;
    };
    let python = ["python", "pypy"].iter().any(|p| {
        first
            .text
            .strip_prefix(p)
            .is_some_and(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.'))
    });
    let stdin_script = match rest {
        [] => true,
        [dash] => dash.text == "-" && !dash.quoted,
        _ => false,
    };
    let piped_on = parsed[owner + 1..]
        .iter()
        .find(|next| next.here_doc_of.is_none())
        .is_some_and(|next| next.piped);
    !first.quoted && python && stdin_script && segment.redirects.is_empty() && !piped_on
}
