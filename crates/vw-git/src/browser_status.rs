//! Porcelain-v2 browser state, through the existing hardened runner and program preflight.
//! Called by the core browser; never fetches, writes the index, or runs repository programs.

use crate::{GitError, programs};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use vw_changes::git::{Call, Index, Profile};

/// A changed path with a plain state label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BrowserFile {
    /// Repository-relative path, with no traversal.
    pub path: String,
    /// Changed, added, deleted, renamed, untracked, or conflict.
    pub status: String,
}
/// One status call's facts, including human-readable state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    /// Current branch, absent when detached.
    pub branch: Option<String>,
    /// Changed tracked paths.
    pub changed_files: u32,
    /// New files, including those inside new directories.
    pub untracked_files: u32,
    /// Unmerged paths.
    pub conflicts: u32,
    /// Commits not sent, as of last fetch.
    pub ahead: u32,
    /// Commits to receive, as of last fetch.
    pub behind: u32,
    /// Upstream ref, absent when not published.
    pub upstream: Option<String>,
    /// Plain words, never raw porcelain.
    pub words: String,
    /// Changed file paths, bounded to the first 500.
    pub files: Vec<BrowserFile>,
}
/// Read working-tree status only after repository-program refusal checks.
///
/// # Errors
/// Returns the program preflight refusal or the hardened runner's git error.
pub fn read(root: &Path) -> Result<BrowserStatus, GitError> {
    programs::check(root, programs::Purpose::Worktree)?;
    let bytes = Call::new(root, Index::User)
        .profile(Profile::Worktree)
        .args([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=all",
        ])
        .timeout(Duration::from_secs(2))
        .run()?;
    Ok(parse(&bytes))
}
/// Parse NUL records; renamed entries consume the following old-path record.
#[must_use]
pub fn parse(bytes: &[u8]) -> BrowserStatus {
    let mut out = BrowserStatus::default();
    let mut records = bytes.split(|b| *b == 0);
    while let Some(record) = records.next() {
        let text = String::from_utf8_lossy(record);
        if let Some(head) = text.strip_prefix("# branch.head ") {
            if head != "(detached)" {
                out.branch = Some(head.into());
            }
            continue;
        }
        if let Some(upstream) = text.strip_prefix("# branch.upstream ") {
            out.upstream = Some(upstream.into());
            continue;
        }
        if let Some(ab) = text.strip_prefix("# branch.ab ") {
            let mut values = ab.split_whitespace();
            out.ahead = values
                .next()
                .and_then(|v| v.strip_prefix('+'))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            out.behind = values
                .next()
                .and_then(|v| v.strip_prefix('-'))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            continue;
        }
        let (path, status) = match record.first() {
            Some(b'?') => {
                out.untracked_files += 1;
                (record.get(2..), "untracked")
            }
            Some(b'u') => {
                out.conflicts += 1;
                out.changed_files += 1;
                (field_after(record, 10), "conflict")
            }
            Some(b'1' | b'2') => {
                out.changed_files += 1;
                let rename = record[0] == b'2';
                let xy = record.split(|b| *b == b' ').nth(1).unwrap_or_default();
                let label = if rename {
                    "renamed"
                } else if xy.contains(&b'D') {
                    "deleted"
                } else if xy.contains(&b'A') {
                    "added"
                } else {
                    "changed"
                };
                let path = field_after(record, if rename { 9 } else { 8 });
                if rename {
                    records.next();
                }
                (path, label)
            }
            _ => continue,
        };
        if let Some(path) = path.filter(|p| safe_relative(p))
            && out.files.len() < 500
        {
            out.files.push(BrowserFile {
                path: vw_changes::display(path),
                status: status.into(),
            });
        }
    }
    out.words = words(&out);
    out
}
fn words(out: &BrowserStatus) -> String {
    let changes = out.changed_files + out.untracked_files;
    let mut words = Vec::new();
    if changes > 0 {
        words.push(format!(
            "{changes} changed {}",
            if changes == 1 { "file" } else { "files" }
        ));
    }
    if out.conflicts > 0 {
        words.push(format!("{} files need conflict resolution", out.conflicts));
    }
    if out.ahead > 0 {
        words.push(format!(
            "{} {} not sent",
            out.ahead,
            if out.ahead == 1 { "commit" } else { "commits" }
        ));
    }
    if out.behind > 0 {
        words.push(format!(
            "{} {} to receive (as of last fetch)",
            out.behind,
            if out.behind == 1 { "commit" } else { "commits" }
        ));
    }
    if out.branch.is_none() {
        words.push("Not on a branch".into());
    } else if out.upstream.is_none() {
        words.push("Not published yet".into());
    } else if words.is_empty() {
        words.push("Up to date".into());
    }
    if changes > u32::try_from(out.files.len()).unwrap_or(u32::MAX) {
        words.push(format!("First {} files shown", out.files.len()));
    }
    words.join(" · ")
}
fn field_after(record: &[u8], spaces: usize) -> Option<&[u8]> {
    let mut from = 0;
    for _ in 0..spaces {
        let offset = record.get(from..)?.iter().position(|b| *b == b' ')?;
        from += offset + 1;
    }
    record.get(from..).filter(|path| !path.is_empty())
}
fn safe_relative(path: &[u8]) -> bool {
    !path.is_empty()
        && !matches!(path[0], b'/' | b'\\')
        && !path.contains(&b':')
        && !path
            .split(|b| matches!(b, b'/' | b'\\'))
            .any(|part| part == b".." || part.is_empty())
}
