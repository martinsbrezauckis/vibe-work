//! Names for a session's copy: the branch suggested from the first message, and the worktree folder name
//! Vibe gets with it.
//!
//! The slug follows Vibe 2.26.0's own rules for worktree names (`vibe/core/git/worktree/naming.py`
//! `worktree_name_from_text`: lower case, everything but `a-z0-9` becomes `-`, at most 6 words and 40
//! characters cut on a word boundary, trailing English stop words dropped) and its auto branch prefix
//! `vibe/` (`repository.py:_AUTO_WORKTREE_BRANCH_PREFIX`), so a branch made here looks like one Vibe makes.
//! Instead of Unicode NFKD (not in std) common Latin letters with marks (Latvian included) are folded to
//! their base letter.
//!
//! Depends on: std. Called by: vw-core-api (`suggestName`, `start`), `lib` re-exports.
//! Invariants:
//! - A folder name is one portable path segment: Vibe refuses anything else (`repository.py`
//!   `_is_portable_worktree_name`: `<>:"/\|?*`, a trailing dot or space, Windows device names).
//! - A suggestion never names a branch or folder the caller reports as taken.

/// Prefix of every suggested branch (Vibe's own auto branches use it too).
pub const BRANCH_PREFIX: &str = "vibe/";
/// Longest slug (Vibe's `MAX_WORKTREE_NAME_LENGTH`).
pub const MAX_NAME: usize = 40;
/// Most words taken from the message.
const MAX_WORDS: usize = 6;
/// The slug when the message gives none.
pub const FALLBACK: &str = "session";
/// Suffixes tried before giving up (`-2` .. `-100`).
const MAX_SUFFIX: u32 = 100;

/// Trailing filler words dropped from a name (Vibe's list).
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "at", "for", "in", "is", "it", "me", "my", "of", "on", "the", "to", "with",
];

/// Windows device names a folder may not be called (Vibe's `_RESERVED_WORKTREE_NAMES`).
fn reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem[3..].chars().all(|c| ('1'..='9').contains(&c)))
}

/// A letter with a mark folded to its base letter (`ā` -> `a`); other characters unchanged.
fn fold(c: char) -> char {
    const TABLE: &[(&str, char)] = &[
        ("àáâãäåāăą", 'a'),
        ("çćčĉċ", 'c'),
        ("ďđ", 'd'),
        ("èéêëēĕėęě", 'e'),
        ("ĝğġģ", 'g'),
        ("ĥħ", 'h'),
        ("ìíîïĩīĭįı", 'i'),
        ("ĵ", 'j'),
        ("ķ", 'k'),
        ("ĺļľŀł", 'l'),
        ("ñńņňŉ", 'n'),
        ("òóôõöøōŏő", 'o'),
        ("ŕŗř", 'r'),
        ("śŝşšș", 's'),
        ("ţťŧț", 't'),
        ("ùúûüũūŭůűų", 'u'),
        ("ŵ", 'w'),
        ("ýÿŷ", 'y'),
        ("źżž", 'z'),
    ];
    TABLE
        .iter()
        .find(|(set, _)| set.contains(c))
        .map_or(c, |(_, base)| *base)
}

/// `text` lower-cased, marks folded, every run of other characters one `-`, no leading or trailing `-`.
fn dashed(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase).map(fold) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

/// The slug of `text` (may be empty): Vibe's rule for a name made from a message.
#[must_use]
pub fn slug(text: &str) -> String {
    let words: Vec<String> = dashed(text)
        .split('-')
        .filter(|w| !w.is_empty())
        .take(MAX_WORDS)
        .map(str::to_owned)
        .collect();
    trim(&words.join("-"), 0)
}

/// `value` cut to at most `max` characters, on a word boundary when it has to be cut.
fn cut(value: &str, max: usize) -> String {
    if value.len() <= max {
        value.to_owned()
    } else if value.as_bytes().get(max) == Some(&b'-') {
        value[..max].to_owned()
    } else {
        let head = &value[..max];
        head.rfind('-')
            .map_or_else(|| head.to_owned(), |b| head[..b].to_owned())
    }
}

/// `value` cut to `MAX_NAME - reserved` characters on a word boundary, trailing stop words dropped.
fn trim(value: &str, reserved: usize) -> String {
    let cut = cut(value, MAX_NAME.saturating_sub(reserved));
    let mut words: Vec<&str> = cut.split('-').filter(|w| !w.is_empty()).collect();
    while words.len() > 1 && words.last().is_some_and(|w| STOP_WORDS.contains(w)) {
        words.pop();
    }
    words.join("-")
}

/// True when `name` can be a worktree folder (one portable path segment, Vibe's rule).
#[must_use]
pub fn portable_folder(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && name.chars().all(|c| !c.is_control())
        && !name.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*'])
        && !reserved(name)
}

/// The worktree folder name for a branch the user typed: its last segment made portable (lower case, other
/// characters as `-`, at most 40 characters; unlike [`slug`] no word is dropped, so `vibe/live-a` gets
/// `live-a`), `wt-` in front of a device name, [`FALLBACK`] when nothing is left. For a suggested branch this
/// is the suggestion's own folder.
#[must_use]
pub fn folder_for(branch: &str) -> String {
    let last = branch.rsplit('/').next().unwrap_or(branch);
    let mut name = cut(&dashed(last), MAX_NAME).trim_matches('-').to_owned();
    if name.is_empty() {
        FALLBACK.clone_into(&mut name);
    }
    if reserved(&name) {
        name = format!("wt-{name}");
    }
    name
}

/// A suggested branch and its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// `vibe/<slug>` (with a `-N` suffix when the plain one is taken).
    pub branch: String,
    /// The worktree folder name.
    pub folder: String,
}

/// The first free `vibe/<slug>` for `text`: `taken(branch, folder)` says whether either is in use. `None`
/// after [`MAX_SUFFIX`] tries.
#[must_use]
pub fn suggest(text: &str, taken: impl Fn(&str, &str) -> bool) -> Option<Suggestion> {
    let mut base = slug(text);
    if base.is_empty() {
        FALLBACK.clone_into(&mut base);
    }
    if reserved(&base) {
        base = format!("wt-{base}");
    }
    (1..=MAX_SUFFIX).find_map(|n| {
        let name = if n == 1 {
            base.clone()
        } else {
            let suffix = format!("-{n}");
            format!("{}{suffix}", trim(&base, suffix.len()))
        };
        let branch = format!("{BRANCH_PREFIX}{name}");
        (!taken(&branch, &name)).then_some(Suggestion {
            branch,
            folder: name,
        })
    })
}
