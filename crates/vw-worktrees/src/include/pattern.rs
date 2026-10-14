//! The `.worktreeinclude` pattern matcher: `.gitignore` syntax over repository-relative paths, plus the
//! rule for wholly ignored directories that the same convention documents (a `**/` pattern reaches into one only when
//! the directory itself matches or the first name after `**/` is one of the names in its path).
//!
//! Pure functions on text and path components: no file system, no git, no process. A path is a list of
//! name components (`["config", "secrets.json"]`), never a string with separators, so no input can add or
//! hide a separator.
//!
//! Depends on: nothing of ours. Called by: `plan`. The supported syntax is checked against git itself in the
//! private differential test (`vw_worktrees_include_matcher`).
//! Invariants:
//! - Last matching pattern wins; `!` re-includes a path; a directory that matched includes everything below
//!   it and a file below it cannot be re-included (git's rule: "it is not possible to re-include a file if
//!   a parent directory of that file is excluded").
//! - A pattern with a `/` anywhere but the end is anchored at the repository root; one without matches the
//!   last name at any depth. A trailing `/` makes it directory-only.
//! - Inside a wholly ignored directory a pattern that is *unanchored or starts with `**/`* ("reaching" patterns)
//!   counts only when the directory's own path matches it or the first name after the `**/` matches one of the
//!   names in the directory's path; every other pattern is evaluated normally.
//! - Deviations from git, all on the safe side: a trailing CR is stripped from a line (CRLF files);
//!   character classes support ranges, `!`/`^` negation and the common `[:class:]` names.

/// One name part of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    /// `**` as a whole segment: zero or more directories.
    Globstar,
    /// A glob over one name (`*`, `?`, `[..]`, `\x`, literal characters).
    Glob(Vec<Tok>),
}

/// One element of a name glob.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Lit(char),
    Any,
    One,
    Class { negate: bool, items: Vec<ClassItem> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClassItem {
    Range(char, char),
    Named(&'static str),
}

/// One parsed line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    negate: bool,
    dir_only: bool,
    anchored: bool,
    segs: Vec<Seg>,
}

/// The parsed file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Patterns {
    list: Vec<Pattern>,
    ignore_case: bool,
}

impl Patterns {
    /// Parses the text of a `.worktreeinclude`; blank lines and comments are dropped.
    #[must_use]
    pub fn parse(text: &str, ignore_case: bool) -> Self {
        let list = text.lines().filter_map(parse_line).collect();
        Self { list, ignore_case }
    }

    /// How many patterns there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// True when no line holds a pattern.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Whether the file at `path` is selected. `inside` is `Some(n)` when the first `n` names of `path` are
    /// a wholly ignored directory (the collapsed entry git printed), `None` for a file git listed on its own.
    #[must_use]
    pub fn selects(&self, path: &[&str], inside: Option<usize>) -> bool {
        for len in 1..path.len() {
            if self.last_match(&path[..len], true, inside) == Some(true) {
                return true;
            }
        }
        self.last_match(path, false, inside) == Some(true)
    }

    /// True when any positive pattern could select something below the wholly ignored directory `dir`
    /// (so a walk of it is worth the time). Never false when a pattern could match: it only prunes.
    #[must_use]
    pub fn may_reach(&self, dir: &[&str]) -> bool {
        self.list
            .iter()
            .filter(|p| !p.negate)
            .any(|p| p.may_reach(dir, self.ignore_case))
    }

    /// Result of the last pattern that matches `path` as a directory or file: `Some(true)` selected,
    /// `Some(false)` re-included by `!`, `None` no pattern matched.
    fn last_match(&self, path: &[&str], is_dir: bool, inside: Option<usize>) -> Option<bool> {
        let dir = inside.map(|n| &path[..n.min(path.len())]);
        let mut verdict = None;
        for p in &self.list {
            // A reaching pattern counts below the wholly ignored directory only if its first name is there.
            if let (Some(d), false) = (dir, p.negate)
                && inside.is_some_and(|n| path.len() > n)
                && p.reaching()
                && !p.first_name_in(d, self.ignore_case)
            {
                continue;
            }
            if p.matches(path, is_dir, self.ignore_case) {
                verdict = Some(!p.negate);
            }
        }
        verdict
    }
}

fn parse_line(line: &str) -> Option<Pattern> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    // Trailing spaces are dropped unless escaped with a backslash.
    let mut line = line.to_owned();
    while line.ends_with(' ') && !line.ends_with("\\ ") {
        line.pop();
    }
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (negate, rest) = match line.strip_prefix('!') {
        Some(rest) => (true, rest.to_owned()),
        None => (false, line.clone()),
    };
    let (body, dir_only) = match rest.strip_suffix('/') {
        Some(b) => (b.to_owned(), true),
        None => (rest, false),
    };
    if body.is_empty() {
        return None;
    }
    let anchored = body.contains('/');
    let body = body.strip_prefix('/').unwrap_or(&body);
    let segs: Vec<Seg> = body.split('/').map(parse_seg).collect();
    if segs.is_empty() {
        return None;
    }
    Some(Pattern {
        negate,
        dir_only,
        anchored,
        segs,
    })
}

fn parse_seg(seg: &str) -> Seg {
    if seg == "**" {
        return Seg::Globstar;
    }
    let chars: Vec<char> = seg.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                // `**` that is not a whole segment acts as one `*`.
                while chars.get(i + 1) == Some(&'*') {
                    i += 1;
                }
                toks.push(Tok::Any);
            }
            '?' => toks.push(Tok::One),
            '\\' => {
                i += 1;
                toks.push(Tok::Lit(chars.get(i).copied().unwrap_or('\\')));
            }
            '[' => match parse_class(&chars, i) {
                Some((tok, next)) => {
                    toks.push(tok);
                    i = next;
                    continue;
                }
                None => toks.push(Tok::Lit('[')),
            },
            c => toks.push(Tok::Lit(c)),
        }
        i += 1;
    }
    Seg::Glob(toks)
}

/// `[...]` starting at `start`; `None` when it is not closed (then `[` is a plain character, like git).
fn parse_class(chars: &[char], start: usize) -> Option<(Tok, usize)> {
    let mut i = start + 1;
    let negate = matches!(chars.get(i), Some('!' | '^'));
    if negate {
        i += 1;
    }
    let mut items = Vec::new();
    let mut first = true;
    loop {
        let c = *chars.get(i)?;
        if c == ']' && !first {
            return Some((Tok::Class { negate, items }, i + 1));
        }
        first = false;
        if c == '[' && chars.get(i + 1) == Some(&':') {
            let close = (i + 2..chars.len().saturating_sub(1))
                .find(|&j| chars[j] == ':' && chars[j + 1] == ']')?;
            let name: String = chars[i + 2..close].iter().collect();
            items.push(ClassItem::Named(class_name(&name)?));
            i = close + 2;
            continue;
        }
        let lo = if c == '\\' {
            i += 1;
            *chars.get(i)?
        } else {
            c
        };
        if chars.get(i + 1) == Some(&'-') && chars.get(i + 2).is_some_and(|&n| n != ']') {
            let mut hi = chars[i + 2];
            let mut skip = 3;
            if hi == '\\' {
                hi = *chars.get(i + 3)?;
                skip = 4;
            }
            items.push(ClassItem::Range(lo, hi));
            i += skip;
        } else {
            items.push(ClassItem::Range(lo, lo));
            i += 1;
        }
    }
}

fn class_name(name: &str) -> Option<&'static str> {
    [
        "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space",
        "upper", "xdigit",
    ]
    .into_iter()
    .find(|n| *n == name)
}

fn named(name: &str, c: char) -> bool {
    match name {
        "alnum" => c.is_ascii_alphanumeric(),
        "alpha" => c.is_ascii_alphabetic(),
        "blank" => c == ' ' || c == '\t',
        "cntrl" => c.is_ascii_control(),
        "digit" => c.is_ascii_digit(),
        "graph" => c.is_ascii_graphic(),
        "lower" => c.is_ascii_lowercase(),
        "print" => c.is_ascii_graphic() || c == ' ',
        "punct" => c.is_ascii_punctuation(),
        "space" => c.is_ascii_whitespace() || c == '\u{b}',
        "upper" => c.is_ascii_uppercase(),
        "xdigit" => c.is_ascii_hexdigit(),
        _ => false,
    }
}

fn fold(c: char, ignore_case: bool) -> char {
    if ignore_case {
        c.to_lowercase().next().unwrap_or(c)
    } else {
        c
    }
}

/// Glob of one name against one name. Iterative with backtracking on `*`: no recursion, no exponential case.
fn glob_matches(toks: &[Tok], name: &str, ignore_case: bool) -> bool {
    let chars: Vec<char> = name.chars().collect();
    let (mut t, mut n) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while n < chars.len() {
        let step = match toks.get(t) {
            Some(Tok::Any) => {
                star = Some((t, n));
                t += 1;
                continue;
            }
            Some(Tok::One) => true,
            Some(Tok::Lit(c)) => fold(*c, ignore_case) == fold(chars[n], ignore_case),
            Some(Tok::Class { negate, items }) => {
                class_has(items, chars[n], ignore_case) != *negate
            }
            None => false,
        };
        if step {
            t += 1;
            n += 1;
        } else if let Some((st, sn)) = star {
            t = st + 1;
            n = sn + 1;
            star = Some((st, sn + 1));
        } else {
            return false;
        }
    }
    toks[t..].iter().all(|t| matches!(t, Tok::Any))
}

fn class_has(items: &[ClassItem], c: char, ignore_case: bool) -> bool {
    let probe = |c: char| {
        items.iter().any(|item| match item {
            ClassItem::Range(lo, hi) => *lo <= c && c <= *hi,
            ClassItem::Named(name) => named(name, c),
        })
    };
    probe(c) || (ignore_case && (probe(fold(c, true)) || c.to_uppercase().any(probe)))
}

impl Pattern {
    /// Unanchored (no `/` but a trailing one) or starting with `**/`: the patterns the wholly-ignored rule
    /// restricts.
    fn reaching(&self) -> bool {
        !self.anchored || self.segs.first() == Some(&Seg::Globstar)
    }

    /// The first name of the pattern after a leading `**/` (the whole pattern when unanchored) matches one of
    /// the names of `dir`.
    fn first_name_in(&self, dir: &[&str], ignore_case: bool) -> bool {
        let first = if self.anchored {
            self.segs.get(1)
        } else {
            self.segs.first()
        };
        match first {
            Some(Seg::Glob(toks)) => dir.iter().any(|d| glob_matches(toks, d, ignore_case)),
            _ => false,
        }
    }

    /// Whether this pattern could select something below the wholly ignored directory `dir`.
    fn may_reach(&self, dir: &[&str], ignore_case: bool) -> bool {
        if self.reaching() {
            // Selects `dir` itself or reaches in through its first name.
            return self.matches(dir, true, ignore_case) || self.first_name_in(dir, ignore_case);
        }
        // Anchored without a leading `**`: its leading names must agree with the directory's.
        for (i, seg) in self.segs.iter().enumerate() {
            let Some(name) = dir.get(i) else { return true };
            match seg {
                Seg::Globstar => return true,
                Seg::Glob(toks) if !glob_matches(toks, name, ignore_case) => return false,
                Seg::Glob(_) => {}
            }
        }
        // The pattern is shorter than the directory path: it matched an ancestor or the directory itself.
        true
    }

    /// Whether the pattern matches `path` exactly (not its ancestors).
    fn matches(&self, path: &[&str], is_dir: bool, ignore_case: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        if !self.anchored {
            return match (self.segs.first(), path.last()) {
                // A lone `**` selects everything.
                (Some(Seg::Globstar), _) => true,
                (Some(Seg::Glob(toks)), Some(last)) => glob_matches(toks, last, ignore_case),
                _ => false,
            };
        }
        segs_match(&self.segs, path, ignore_case)
    }
}

/// Anchored match with `**` segments, iterative backtracking over directories.
fn segs_match(segs: &[Seg], path: &[&str], ignore_case: bool) -> bool {
    let (mut s, mut p) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    loop {
        match segs.get(s) {
            Some(Seg::Globstar) => {
                // A trailing `**` needs at least one name below (`abc/**` does not match `abc`).
                if s + 1 == segs.len() {
                    return p < path.len();
                }
                star = Some((s, p));
                s += 1;
            }
            Some(Seg::Glob(toks)) if p < path.len() && glob_matches(toks, path[p], ignore_case) => {
                s += 1;
                p += 1;
            }
            None if p == path.len() => return true,
            _ => match star {
                Some((ss, sp)) if sp < path.len() => {
                    star = Some((ss, sp + 1));
                    s = ss + 1;
                    p = sp + 1;
                }
                _ => return false,
            },
        }
    }
}
