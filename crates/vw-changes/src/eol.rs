//! Line endings git converts on checkout, so a file `git status` calls unchanged is unchanged here too.
//!
//! git stores text normalized and may check it out with other line endings (`.gitattributes` `text` / `eol`,
//! the older `crlf` attribute, `core.autocrlf`, `core.eol`): a `*.ps1 text eol=crlf` file has CRLF in the
//! working tree and LF in its blob, and `git status` calls it clean. The working tree is hashed byte for byte
//! here (`hash-object --no-filters`, so no repository filter runs); for a tracked file whose bytes differ
//! from its baseline blob, [`git_equal`] repeats git's two built-in conversions in Rust and reports the file
//! unchanged only when both hold:
//! - converting the file into the repository gives exactly the baseline blob (its id), and
//! - checking that blob out again writes exactly the file's bytes.
//!
//! So a removal of an unchanged file loses no byte (a fresh checkout writes it back as it is), and a file
//! `git status` lists (an `eol=lf` file saved with CRLF, mixed line endings in an `eol=crlf` file) is listed.
//!
//! git answers which rules apply (`check-attr` reads the attribute files; `config --get-regexp` reads
//! `core.autocrlf` and `core.eol`, the system file included: Git for Windows keeps `core.autocrlf=true` there).
//! The conversions follow git's `convert.c` (`convert_attrs`, `crlf_to_git`, `crlf_to_worktree`,
//! `output_eol`):
//! - the action: `text` (else the older `crlf`) set = text, unset = binary, `auto` = guess, `input` = text
//!   checked out with LF; an `eol` value makes an unspecified or set file text with that line ending and keeps
//!   `auto` a guess with it; still unspecified: binary when `core.autocrlf` is false, else a guess;
//! - into the repository, text: every CR right before an LF is dropped; a guess: a file with a NUL, a lone
//!   CR or mostly unprintable bytes is left as it is, otherwise every CR is dropped;
//! - out of it, with CRLF line endings: text: every LF without a CR before it gets one; a guess: the same,
//!   unless the blob holds a CR or looks binary.
//!
//! Depends on: `git`, `objects` (`blob_id`), `repo`, `error`. Called by: `worktree::scan` (refreshes only: the
//! scan that takes a baseline records the bytes as they are), `diff::describe` ([`stored_forms`]).
//! Invariants:
//! - Runs no program the repository names: a file with a `filter` driver, `ident` or a
//!   `working-tree-encoding` is never converted (its bytes are compared as they are: listed as changed, the
//!   safe direction for a removal that discards unsaved work).
//! - Any failure (git, a file that is no longer a regular file, one larger than [`MAX_NORMALIZED`]) leaves
//!   the file listed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::ChangesError;
use crate::git::{Call, Index, Profile};
use crate::objects::blob_id;
use crate::repo::Repo;

/// Largest file normalized in memory (bytes); a larger one keeps its byte-for-byte result.
pub(crate) const MAX_NORMALIZED: u64 = 64 << 20;

const ATTRIBUTES: [&str; 6] = [
    "text",
    "crlf",
    "eol",
    "filter",
    "ident",
    "working-tree-encoding",
];

/// A tracked file whose bytes differ from its baseline blob.
pub(crate) struct Candidate {
    /// Repository-relative path, git's bytes.
    pub(crate) rel: Vec<u8>,
    /// The file on disk.
    pub(crate) full: PathBuf,
    /// The baseline blob id it is compared with.
    pub(crate) base_id: String,
}

/// What git does to a file's line endings (`crlf` true: checked out with CRLF).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Stored and checked out byte for byte.
    AsIs,
    /// Text: CRLF becomes LF in the repository.
    Text { crlf: bool },
    /// Guessed: converted only when the content looks like text.
    Guess { crlf: bool },
    /// A conversion this module does not repeat (filter driver, ident, encoding): compared byte for byte.
    Unknown,
}

/// The config keys that decide line endings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Config {
    /// `core.autocrlf`: `Some(true)` = true, `Some(false)` = input, `None` = false.
    autocrlf: Option<bool>,
    /// `core.eol`: `Some(true)` = crlf, `Some(false)` = lf, `None` = native.
    eol_crlf: Option<bool>,
}

impl Config {
    /// git's `text_eol_is_crlf`: the line ending of a `text` or `auto` file without an `eol` attribute.
    fn text_eol_is_crlf(self) -> bool {
        match (self.autocrlf, self.eol_crlf) {
            (Some(true), _) => true,
            (Some(false), _) => false,
            (None, Some(crlf)) => crlf,
            (None, None) => cfg!(windows),
        }
    }
}

/// The candidates git would call unchanged against their baseline blob (by `rel`).
///
/// # Errors
/// git failures of the attribute lookup (the caller then keeps every candidate listed).
pub(crate) fn git_equal(
    repo: &Repo,
    index: &Path,
    candidates: &[Candidate],
) -> Result<HashSet<Vec<u8>>, ChangesError> {
    let mut equal = HashSet::new();
    if candidates.is_empty() {
        return Ok(equal);
    }
    let rels: Vec<&[u8]> = candidates.iter().map(|c| c.rel.as_slice()).collect();
    let attrs = attributes(repo, index, &rels)?;
    let config = config(repo, index);
    for (candidate, values) in candidates.iter().zip(attrs) {
        let action = action(&values, config);
        if matches!(action, Action::AsIs | Action::Unknown) {
            continue;
        }
        let Some(bytes) = read_regular(&candidate.full) else {
            continue;
        };
        if round_trips(&bytes, action, |stored| {
            blob_id(&repo.object_format, stored) == candidate.base_id
        }) {
            equal.insert(candidate.rel.clone());
        }
    }
    Ok(equal)
}

/// For each `(rel, working bytes)` that git converts on the way into the repository, the bytes in that stored
/// form (CRs dropped), keyed by `rel`; a file git stores as it is, or one this module does not convert,
/// is left out. The Changes line diff compares these against the baseline blob, as `git diff` does.
///
/// # Errors
/// git failures of the attribute lookup (the caller then diffs the bytes as they are).
pub(crate) fn stored_forms(
    repo: &Repo,
    index: &Path,
    files: &[(&[u8], &[u8])],
) -> Result<HashMap<Vec<u8>, Vec<u8>>, ChangesError> {
    let mut forms = HashMap::new();
    if files.is_empty() {
        return Ok(forms);
    }
    let rels: Vec<&[u8]> = files.iter().map(|(rel, _)| *rel).collect();
    let attrs = attributes(repo, index, &rels)?;
    let config = config(repo, index);
    for ((rel, bytes), values) in files.iter().zip(attrs) {
        if let Some(stored) = to_git(bytes, action(&values, config)) {
            forms.insert(rel.to_vec(), stored);
        }
    }
    Ok(forms)
}

/// True when `bytes` go into the repository as a blob `is_base` accepts and that blob checks out as `bytes`.
fn round_trips(bytes: &[u8], action: Action, is_base: impl FnOnce(&[u8]) -> bool) -> bool {
    let Some(stored) = to_git(bytes, action) else {
        return false;
    };
    is_base(&stored) && to_worktree(&stored, action) == bytes
}

/// The file's bytes when it is (still) a regular file within [`MAX_NORMALIZED`].
fn read_regular(full: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::symlink_metadata(full).ok()?;
    if !meta.is_file() || meta.len() > MAX_NORMALIZED {
        return None;
    }
    let bytes = std::fs::read(full).ok()?;
    (u64::try_from(bytes.len()).ok()? <= MAX_NORMALIZED).then_some(bytes)
}

/// `check-attr` values of [`ATTRIBUTES`] for every candidate, in candidate order.
fn attributes(repo: &Repo, index: &Path, rels: &[&[u8]]) -> Result<Vec<[String; 6]>, ChangesError> {
    let mut request = Vec::new();
    for rel in rels {
        request.extend_from_slice(rel);
        request.push(0);
    }
    let out = Call::new(&repo.root, Index::Ours(index))
        .args(["check-attr", "-z", "--stdin"])
        .args(ATTRIBUTES)
        .stdin(&request)
        .timeout(Duration::from_secs(60))
        .run()?;
    // Records: "<path>\0<attribute>\0<value>\0", ATTRIBUTES.len() per path, in request order.
    let fields: Vec<&[u8]> = out.split(|b| *b == 0).collect();
    let per_path = ATTRIBUTES.len() * 3;
    if fields.len() < rels.len() * per_path {
        return Err(ChangesError::Parse(
            "check-attr printed fewer records than asked".into(),
        ));
    }
    let mut all = Vec::with_capacity(rels.len());
    for (i, rel) in rels.iter().enumerate() {
        let mut values: [String; 6] = Default::default();
        for record in fields[i * per_path..(i + 1) * per_path].chunks(3) {
            if record[0] != *rel {
                return Err(ChangesError::Parse(
                    "check-attr answered for another path".into(),
                ));
            }
            let name = String::from_utf8_lossy(record[1]);
            if let Some(slot) = ATTRIBUTES.iter().position(|a| *a == name) {
                values[slot] = String::from_utf8_lossy(record[2]).into_owned();
            }
        }
        all.push(values);
    }
    Ok(all)
}

/// `core.autocrlf` and `core.eol` as the user's own git reads them (system, global and repository config;
/// the last value wins). Unreadable or unset = git's defaults (`autocrlf` false, `eol` native).
fn config(repo: &Repo, index: &Path) -> Config {
    let Ok(out) = Call::new(&repo.root, Index::Ours(index))
        .profile(Profile::Worktree)
        .args(["config", "--get-regexp", r"^core\.(autocrlf|eol)$"])
        .timeout(Duration::from_secs(10))
        .run()
    else {
        return Config::default();
    };
    let mut config = Config::default();
    for line in String::from_utf8_lossy(&out).lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        let value = value.trim().to_ascii_lowercase();
        match key.to_ascii_lowercase().as_str() {
            "core.autocrlf" => config.autocrlf = parse_autocrlf(&value),
            "core.eol" => {
                config.eol_crlf = match value.as_str() {
                    "crlf" => Some(true),
                    "lf" => Some(false),
                    _ => None,
                };
            }
            _ => {}
        }
    }
    config
}

/// `core.autocrlf`: `input`, or a git boolean (a number counts as one too).
fn parse_autocrlf(value: &str) -> Option<bool> {
    match value {
        "input" => Some(false),
        "true" | "yes" | "on" => Some(true),
        other => other.parse::<i64>().ok().filter(|n| *n != 0).map(|_| true),
    }
}

/// A `text` or `crlf` attribute value.
#[derive(PartialEq)]
enum Attr {
    Binary,
    Text,
    Input,
    Auto,
}

/// git's decision for one file, from its `check-attr` values (`unspecified`, `set`, `unset` or a value).
fn action(values: &[String; 6], config: Config) -> Action {
    let [text, crlf, eol, filter, ident, encoding] = values;
    let given = |v: &str| v != "unspecified" && v != "unset";
    // A configured filter driver, `ident` or an encoding change the bytes in ways not repeated here.
    if (given(filter) && filter != "set") || ident == "set" || given(encoding) {
        return Action::Unknown;
    }
    let from = |v: &str| match v {
        "set" => Some(Attr::Text),
        "input" => Some(Attr::Input),
        "unset" => Some(Attr::Binary),
        "auto" => Some(Attr::Auto),
        _ => None,
    };
    let attr = from(text).or_else(|| from(crlf));
    let eol = match eol.as_str() {
        "crlf" => Some(true),
        "lf" => Some(false),
        _ => None,
    };
    match (attr, eol) {
        (Some(Attr::Binary), _) => Action::AsIs,
        (Some(Attr::Auto), Some(crlf)) => Action::Guess { crlf },
        (Some(Attr::Auto), None) => Action::Guess {
            crlf: config.text_eol_is_crlf(),
        },
        (_, Some(crlf)) => Action::Text { crlf },
        (Some(Attr::Input), None) => Action::Text { crlf: false },
        (Some(Attr::Text), None) => Action::Text {
            crlf: config.text_eol_is_crlf(),
        },
        (None, None) => match config.autocrlf {
            None => Action::AsIs,
            Some(crlf) => Action::Guess { crlf },
        },
    }
}

/// The bytes git would store for `bytes` under `action`; `None` when it stores them as they are.
fn to_git(bytes: &[u8], action: Action) -> Option<Vec<u8>> {
    let stats = Stats::of(bytes);
    if bytes.is_empty() || stats.crlf == 0 {
        return None;
    }
    match action {
        Action::Text { .. } => {
            let mut out = Vec::with_capacity(bytes.len());
            for (i, b) in bytes.iter().enumerate() {
                if !(*b == b'\r' && bytes.get(i + 1) == Some(&b'\n')) {
                    out.push(*b);
                }
            }
            Some(out)
        }
        // A guessed text file has no lone CR (that makes it binary), so every CR goes.
        Action::Guess { .. } if !stats.binary() => {
            Some(bytes.iter().copied().filter(|b| *b != b'\r').collect())
        }
        _ => None,
    }
}

/// The bytes git checks out for the blob `stored` under `action`.
fn to_worktree(stored: &[u8], action: Action) -> Vec<u8> {
    let (crlf, guess) = match action {
        Action::Text { crlf } => (crlf, false),
        Action::Guess { crlf } => (crlf, true),
        Action::AsIs | Action::Unknown => (false, false),
    };
    let stats = Stats::of(stored);
    if !crlf
        || stats.lonelf == 0
        || (guess && (stats.lonecr > 0 || stats.crlf > 0 || stats.binary()))
    {
        return stored.to_vec();
    }
    let mut out = Vec::with_capacity(stored.len() + stats.lonelf);
    for (i, b) in stored.iter().enumerate() {
        if *b == b'\n' && (i == 0 || stored[i - 1] != b'\r') {
            out.push(b'\r');
        }
        out.push(*b);
    }
    out
}

/// git's `gather_stats` counts (`convert.c`).
#[derive(Debug, Default)]
struct Stats {
    nul: usize,
    lonecr: usize,
    lonelf: usize,
    crlf: usize,
    printable: usize,
    nonprintable: usize,
}

impl Stats {
    fn of(bytes: &[u8]) -> Self {
        let mut s = Self::default();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            i += 1;
            match c {
                b'\r' if bytes.get(i) == Some(&b'\n') => {
                    s.crlf += 1;
                    i += 1;
                }
                b'\r' => s.lonecr += 1,
                b'\n' => s.lonelf += 1,
                127 => s.nonprintable += 1,
                // BS, HT, ESC and FF count as printable.
                8 | 9 | 27 | 12 => s.printable += 1,
                0 => {
                    s.nul += 1;
                    s.nonprintable += 1;
                }
                c if c < 32 => s.nonprintable += 1,
                _ => s.printable += 1,
            }
        }
        // A trailing Ctrl-Z (DOS end of file) is not counted as unprintable.
        if bytes.last() == Some(&0x1a) {
            s.nonprintable = s.nonprintable.saturating_sub(1);
        }
        s
    }

    /// git's `convert_is_binary`.
    fn binary(&self) -> bool {
        self.lonecr > 0 || self.nul > 0 || (self.printable >> 7) < self.nonprintable
    }
}

#[cfg(test)]
#[path = "eol_tests.rs"]
mod tests;
