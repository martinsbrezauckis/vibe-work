//! The existing path/safety tokenizer: splits a plain command line into segments (simple
//! commands) and words. It only has to be right for plain lists: anything with an expansion, a substitution
//! or grouping can need more checks by its callers. The git guard reads it too (`gitparse::readings`).
//!
//! Here-documents (`<<WORD`, `<<-WORD`, not the `<<<` here-string): the lines after the command line, up to the
//! delimiter line, are still split into segments as before (a shell may run them: `bash <<EOF`), and each such
//! segment names the segment whose `<<` opened it (`Segment::here_doc_of`), so a caller can tell an inline script's
//! input from commands (Python's `// 2` is not the path `//`). A body whose opening command was not
//! kept as a segment stays unmarked (read as commands).
//!
//! Handles single and double quotes, backslash escapes, `$...` and backticks (kept as text), comments, redirections (`> file`, `2>&1`, `&>`: file targets kept separately, never mistaken for arguments) and the
//! operators `;`, `&&`, `||`, `|`, `|&`, `&`, newlines and parentheses. A segment remembers whether it reads
//! the previous one's output (`|`). A brace after a closing parenthesis, or before whitespace,
//! is treated as a PowerShell script block. This is a lexical inference; the caller supplies no shell identity.
//! Braces inside `${...}` or a compact bash brace word remain part of that word.
//!
//! Depends on: std. Called by: `folders` and `safety`; retained to preserve their behavior.
//! Invariant: never panics and never loops forever on any input (every branch consumes at least one char).

use std::iter::Peekable;
use std::str::Chars;

/// One word of a simple command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Word {
    /// The text with quotes removed (expansions kept as written, e.g. `$PID`).
    pub(crate) text: String,
    /// True when the shell would change the word's text: a quote, a backslash escape, `$` or a backtick was in
    /// it (the classifier then cannot trust it as a command name or literal).
    pub(crate) quoted: bool,
    /// True when `$` or a backtick was read as shell syntax rather than single-quoted data.
    pub(crate) expanded: bool,
}

/// One simple command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Segment {
    /// Its words.
    pub(crate) words: Vec<Word>,
    /// File redirection targets, kept separately from command arguments for guards that check paths.
    pub(crate) redirects: Vec<Word>,
    /// True when the previous segment's output is piped into it.
    pub(crate) piped: bool,
    /// True when PowerShell's call operator `&` introduced this command.
    pub(crate) invoked: bool,
    /// Text a `#` comment skipped in this segment (another shell may read it as code).
    pub(crate) comment: String,
    /// A redirection whose target starts with `#` (read as a comment here, maybe not by the shell).
    pub(crate) redirect_into_comment: bool,
    /// For a line of a here-document's body: the index (in the same list) of the segment whose `<<` opened it.
    pub(crate) here_doc_of: Option<usize>,
}

#[derive(Default)]
struct Builder {
    segments: Vec<Segment>,
    current: Segment,
    word: Word,
    started: bool,
    next_piped: bool,
    next_invoked: bool,
    pending_redirect: Option<RedirectKind>,
    /// Here-documents whose bodies start after the current line: delimiter, `<<-` (leading tabs stripped), the
    /// index of the segment that opened it once that segment is kept, whether it may be marked at all, and the
    /// number of the segment that was being read at its `<<`.
    here_docs: Vec<(String, bool, Option<usize>, bool, usize)>,
    /// The number of the segment being read (every finished segment counts, kept or not).
    seq: usize,
    /// The `<(` and `>(` process substitutions read: with any, no body of this input is marked.
    substitutions: usize,
}

#[derive(Clone, Copy)]
enum RedirectKind {
    File,
    /// `<<` (`true`: `<<-`): the word is the delimiter of a here-document.
    Delimiter(bool),
    /// `<<<`: the word is the here-string itself (data, not a file).
    HereString,
}

impl Builder {
    fn finish_word(&mut self) {
        if self.started {
            let word = std::mem::take(&mut self.word);
            if let Some(kind) = self.pending_redirect.take() {
                match kind {
                    RedirectKind::File => self.current.redirects.push(word),
                    RedirectKind::Delimiter(strip) => {
                        // Marked only for a quoted delimiter bash reads like this tokenizer (not `$'..'`, `$".."` or
                        // an expansion), opened by a command whose name came first: `( cat ) <<EOF` or
                        // `{ cat; } <<EOF` redirect a group, `<<EOF cat` names its command later.
                        let exact = word.quoted
                            && !word.expanded
                            && !word.text.contains(['$', '`'])
                            && !self.current.words.is_empty();
                        self.here_docs
                            .push((word.text, strip, None, exact, self.seq));
                    }
                    RedirectKind::HereString => {}
                }
            } else {
                self.current.words.push(word);
            }
        }
        self.word = Word::default();
        self.started = false;
    }

    fn finish_segment(&mut self, piped_next: bool) {
        self.finish_word();
        self.pending_redirect = None;
        let mut seg = std::mem::take(&mut self.current);
        seg.piped = self.next_piped;
        seg.invoked = self.next_invoked;
        self.next_invoked = false;
        if seg.words.is_empty()
            && seg.redirects.is_empty()
            && seg.comment.is_empty()
            && !seg.redirect_into_comment
        {
            // `a | | b` or a leading operator: keep the pipe flag only for a real next command.
            self.next_piped = piped_next && self.next_piped;
        } else {
            // Only the here-documents whose `<<` was read in this very segment belong to it.
            for (_, _, owner, _, seq) in &mut self.here_docs {
                if *seq == self.seq {
                    owner.get_or_insert(self.segments.len());
                }
            }
            self.segments.push(seg);
            self.next_piped = piped_next;
        }
        self.seq += 1;
    }

    /// After a line break: the bodies of the here-documents the finished line opened, in order. Each body ends at
    /// a line equal to its delimiter (leading tabs stripped for `<<-`; a trailing `\r` also ends it, which can only
    /// end a body early, so more is read as commands) or at the end of the input. Only a body with an exact
    /// delimiter, its closing line and plain quoting ([`plain_quotes`]) is marked, nested ones included (fail
    /// closed: anything else is read as commands, as before).
    fn here_doc_bodies(&mut self, chars: &mut Peekable<Chars<'_>>, windows_paths: bool) {
        for (delimiter, strip, owner, exact, _) in std::mem::take(&mut self.here_docs) {
            let mut body = String::new();
            let mut closed = false;
            while chars.peek().is_some() {
                let mut line: String = chars.by_ref().take_while(|c| *c != '\n').collect();
                if strip {
                    line = line.trim_start_matches('\t').to_owned();
                }
                if line == delimiter || line.strip_suffix('\r') == Some(delimiter.as_str()) {
                    closed = true;
                    break;
                }
                body.push_str(&line);
                body.push('\n');
            }
            let base = self.segments.len();
            let trusted = exact && closed && plain_quotes(&body);
            for mut segment in segments_with_windows_paths(&body, windows_paths) {
                segment.here_doc_of = match segment.here_doc_of {
                    _ if !trusted => None,
                    Some(nested) => Some(base + nested),
                    None => owner,
                };
                self.segments.push(segment);
            }
        }
    }

    fn push(&mut self, c: char) {
        self.word.text.push(c);
        self.started = true;
    }
}

/// Splits `input` into simple commands.
pub(crate) fn segments(input: &str) -> Vec<Segment> {
    segments_with_windows_paths(input, false)
}

/// Keep native path backslashes and cmd's literal `#`. The folder guard also checks Bash's interpretation; the git and
/// process guards read it on every OS (one more reading can only add refusals).
pub(crate) fn windows_segments(input: &str) -> Vec<Segment> {
    segments_with_windows_paths(input, true)
}

fn segments_with_windows_paths(input: &str, windows_paths: bool) -> Vec<Segment> {
    let mut b = Builder::default();
    let mut chars = input.chars().peekable();
    let mut after_paren = false;
    let mut block_depth = 0usize;
    while let Some(c) = chars.next() {
        if !c.is_whitespace() && c != '{' {
            after_paren = false;
        }
        match c {
            ' ' | '\t' | '\r' => b.finish_word(),
            '\n' => {
                b.finish_segment(false);
                b.here_doc_bodies(&mut chars, windows_paths);
            }
            ';' => b.finish_segment(false),
            '(' => {
                b.substitutions += usize::from(b.pending_redirect.is_some());
                b.finish_segment(false);
            }
            ')' => {
                b.finish_segment(false);
                after_paren = true;
            }
            '{' if after_paren || chars.peek().is_some_and(|n| n.is_whitespace()) => {
                b.finish_segment(false);
                block_depth += 1;
                after_paren = false;
            }
            '}' if block_depth > 0 => {
                b.finish_segment(false);
                block_depth -= 1;
            }
            '}' if !b.started => b.finish_segment(false),
            '&' => match chars.peek() {
                Some('&') => {
                    chars.next();
                    b.finish_segment(false);
                }
                Some('>') => redirect(&mut b, &mut chars, '>'),
                _ => {
                    b.finish_segment(false);
                    b.next_invoked = true;
                }
            },
            '|' => match chars.peek() {
                Some('|') => {
                    chars.next();
                    b.finish_segment(false);
                }
                Some('&') => {
                    chars.next();
                    b.finish_segment(true);
                }
                _ => b.finish_segment(true),
            },
            '>' | '<' => {
                // A word of digits right before is a file descriptor (`2>&1`), not an argument.
                if b.started && b.word.text.chars().all(|d| d.is_ascii_digit()) {
                    b.word = Word::default();
                    b.started = false;
                } else {
                    b.finish_word();
                }
                redirect(&mut b, &mut chars, c);
            }
            '#' if !windows_paths && !b.started => {
                if comment(&mut b, &mut chars) {
                    b.here_doc_bodies(&mut chars, windows_paths);
                }
            }
            '\\' if windows_paths => b.push(c),
            '\\' => match chars.next() {
                Some('\n') | None => {}
                Some(n) => {
                    b.word.quoted = true;
                    b.push(n);
                }
            },
            '\'' => {
                b.started = true;
                b.word.quoted = true;
                for n in chars.by_ref() {
                    if n == '\'' {
                        break;
                    }
                    b.word.text.push(n);
                }
            }
            '"' => double_quoted(&mut b, &mut chars, windows_paths),
            '$' => dollar(&mut b, &mut chars),
            '`' => backtick(&mut b, &mut chars),
            other => b.push(other),
        }
    }
    b.finish_segment(false);
    if b.substitutions > 0 {
        for segment in &mut b.segments {
            segment.here_doc_of = None;
        }
    }
    b.segments
}

/// True when every line of `body` reads the same as shell words and as a script's own quoting: no backslash, no
/// triple quote, an even number of `'` and of `"` on each line. (A `#` comment, a multi-line string or an escaped
/// quote can make the shell reading and Python's differ, so such a body is never marked.)
fn plain_quotes(body: &str) -> bool {
    !body.contains(['\\', '`'])
        && !body.contains("'''")
        && !body.contains("\"\"\"")
        && body
            .lines()
            .all(|line| line.matches('\'').count() % 2 == 0 && line.matches('"').count() % 2 == 0)
}

/// A `#` comment to the end of its line: skipped (kept as the segment's `comment` text), the segment finished.
/// True when a line break ended it (here-document bodies may follow).
fn comment(b: &mut Builder, chars: &mut Peekable<Chars<'_>>) -> bool {
    if b.pending_redirect.is_some() {
        b.current.redirect_into_comment = true;
        b.pending_redirect = None;
    }
    let mut skipped = String::new();
    let mut line_ended = false;
    for n in chars.by_ref() {
        if n == '\n' {
            line_ended = true;
            break;
        }
        skipped.push(n);
    }
    b.current.comment.push_str(&skipped);
    b.finish_segment(false);
    line_ended
}

/// Consumes the rest of a redirection operator (`>`, `>>`, `>&`, `<`, `<<`, `&>`) and marks the next word
/// (its target) to be kept separately from arguments, except here-doc delimiters; `>&1` targets are consumed here.
fn redirect(b: &mut Builder, chars: &mut Peekable<Chars<'_>>, operator: char) {
    let mut more = String::new();
    while let Some(c) = chars.next_if(|c| matches!(c, '>' | '<')) {
        more.push(c);
    }
    // `<<` and `<<-` open a here-document, `<<<` is a here-string; `<>` and `<<>`-like forms are not either.
    let here_doc = operator == '<' && more == "<";
    let here_string = operator == '<' && more.starts_with("<<");
    let strip = here_doc && chars.next_if_eq(&'-').is_some();
    if chars.peek() == Some(&'&') {
        chars.next();
        // `>&1`, `>&-`: the target is attached.
        while matches!(chars.peek(), Some(c) if c.is_ascii_digit() || *c == '-') {
            chars.next();
        }
        return;
    }
    b.pending_redirect = Some(if here_doc {
        RedirectKind::Delimiter(strip)
    } else if here_string {
        RedirectKind::HereString
    } else {
        RedirectKind::File
    });
}

fn double_quoted(b: &mut Builder, chars: &mut Peekable<Chars<'_>>, literal_backslashes: bool) {
    b.started = true;
    b.word.quoted = true;
    while let Some(n) = chars.next() {
        match n {
            '"' => return,
            '\\' if literal_backslashes => b.word.text.push(n),
            '\\' => match chars.next() {
                Some(e @ ('"' | '\\' | '$' | '`')) => b.word.text.push(e),
                Some('\n') | None => {}
                Some(e) => {
                    b.word.text.push('\\');
                    b.word.text.push(e);
                }
            },
            '$' => dollar(b, chars),
            '`' => backtick(b, chars),
            other => b.word.text.push(other),
        }
    }
}

/// Reads up to the parenthesis that closes an already opened one; quotes inside are respected.
fn balanced(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut depth = 1usize;
    let mut text = String::new();
    let mut quote: Option<char> = None;
    while let Some(n) = chars.next() {
        match (quote, n) {
            (Some(q), c) if c == q => quote = None,
            (_, '\\') => {
                text.push('\\');
                if let Some(e) = chars.next() {
                    text.push(e);
                }
                continue;
            }
            (None, '\'' | '"') => quote = Some(n),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth == 0 {
                    return text;
                }
            }
            _ => {}
        }
        text.push(n);
    }
    text
}

fn dollar(b: &mut Builder, chars: &mut Peekable<Chars<'_>>) {
    b.started = true;
    b.word.quoted = true;
    b.word.expanded = true;
    match chars.peek().copied() {
        Some('(') => {
            chars.next();
            if chars.peek() == Some(&'(') {
                chars.next();
                let inner = balanced(chars);
                let _ = chars.next_if_eq(&')');
                b.word.text.push_str("$((");
                b.word.text.push_str(&inner);
                b.word.text.push_str("))");
            } else {
                let inner = balanced(chars);
                b.word.text.push_str("$(");
                b.word.text.push_str(&inner);
                b.word.text.push(')');
            }
        }
        Some('{') => {
            chars.next();
            b.word.text.push_str("${");
            for n in chars.by_ref() {
                b.word.text.push(n);
                if n == '}' {
                    break;
                }
            }
        }
        Some('!') => {
            // `$!`: the shell's own last background job (never another session's process).
            chars.next();
            b.word.text.push_str("$!");
        }
        Some(c)
            if c.is_ascii_alphanumeric()
                || matches!(c, '_' | '?' | '#' | '@' | '*' | '$' | '-') =>
        {
            b.word.text.push('$');
            if c.is_ascii_alphabetic() || c == '_' {
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphanumeric() || n == '_' {
                        b.word.text.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
            } else {
                b.word.text.push(c);
                chars.next();
            }
        }
        _ => b.word.text.push('$'),
    }
}

fn backtick(b: &mut Builder, chars: &mut Peekable<Chars<'_>>) {
    b.started = true;
    b.word.quoted = true;
    b.word.expanded = true;
    // A lone backtick is PowerShell's one-character escape; paired backticks retain the
    // shell substitution text for the path/safety lexer callers.
    if !chars.clone().any(|c| c == '`') {
        if let Some(c) = chars.next() {
            b.word.text.push(c);
        }
        return;
    }
    let mut inner = String::new();
    while let Some(n) = chars.next() {
        match n {
            '`' => break,
            '\\' => {
                if let Some(e) = chars.next() {
                    inner.push(e);
                }
            }
            other => inner.push(other),
        }
    }
    b.word.text.push('`');
    b.word.text.push_str(&inner);
    b.word.text.push('`');
}
