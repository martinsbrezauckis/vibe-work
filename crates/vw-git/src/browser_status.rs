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

#[cfg(test)]
mod tests {
    use super::{parse, read};

    #[test]
    fn status_words_track_upstream_and_counts() {
        let clean =
            parse(b"# branch.head main\0# branch.upstream origin/main\0# branch.ab +0 -0\0");
        assert_eq!(clean.words, "Up to date");
        let unpublished = parse(b"# branch.head topic\0");
        assert_eq!(unpublished.words, "Not published yet");
        let dirty = parse(b"# branch.head topic\0# branch.upstream origin/topic\0# branch.ab +2 -0\0? new.txt\0? second.txt\0? third.txt\0");
        assert_eq!(dirty.words, "3 changed files · 2 commits not sent");
    }

    #[test]
    fn porcelain_paths_survive_spaces_renames_conflicts_and_malformed_entries() {
        let data = b"# branch.head topic\0# branch.upstream origin/topic\0\
            1 .M N... 100644 100644 100644 aaa bbb dir/a b.txt\0\
            2 R. N... 100644 100644 100644 aaa bbb R100 new name.txt\0old name.txt\0\
            u UU N... 100644 100644 100644 100644 aaa bbb ccc conflict.txt\0\
            ? ../escape\0? safe.txt\0? fresh/new.txt\0";
        let status = parse(data);
        assert_eq!(status.changed_files, 3);
        assert_eq!(status.untracked_files, 3);
        assert_eq!(status.conflicts, 1);
        let paths: Vec<_> = status
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.status.as_str()))
            .collect();
        assert_eq!(
            paths,
            [
                ("dir/a b.txt", "changed"),
                ("new name.txt", "renamed"),
                ("conflict.txt", "conflict"),
                ("safe.txt", "untracked"),
                ("fresh/new.txt", "untracked")
            ]
        );
    }

    #[test]
    fn real_status_lists_nested_untracked_files_as_openable_paths() {
        let root = std::env::temp_dir().join(format!(
            "vw-git-browser-status-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let setup = std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(&root)
            .status()
            .unwrap();
        assert!(setup.success());
        vw_changes::prepare_root(&root.join("data")).unwrap();
        std::fs::write(root.join("nested/new.txt"), "new").unwrap();
        let status = read(&root).unwrap();
        assert_eq!(status.untracked_files, 1);
        assert_eq!(status.files[0].path, "nested/new.txt");
        assert!(root.join(&status.files[0].path).is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn large_status_reports_file_list_limit() {
        let mut data = b"# branch.head main\0# branch.upstream origin/main\0".to_vec();
        for i in 0..501 {
            data.extend_from_slice(format!("? new-{i}.txt\0").as_bytes());
        }
        let status = parse(&data);
        assert_eq!(status.untracked_files, 501);
        assert_eq!(status.files.len(), 500);
        assert!(status.words.contains("First 500 files shown"));
    }
}
