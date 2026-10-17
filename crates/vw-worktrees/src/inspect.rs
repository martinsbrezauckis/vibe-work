//! What removing a worktree would lose, found without letting git read the working tree. Vibe's
//! own `inspect` says only "uncommitted changes / untracked files" (no names), and Vibe Work wants the names
//! without running any program the repository could name, so it reads the copy itself.
//!
//! A removal discards (listed in [`Inspection::files`], so the user confirms them by name):
//! - tracked changes and new files the ignore rules do not hide: `vw_changes::diff_against_head` (files read in
//!   Rust, hashed with `--no-filters`; a filter repository's file can show as changed when only its filtered
//!   form matches: the safe direction), plus staged-only changes (`git diff-index --cached HEAD`);
//! - **ignored files that are not known build output** (`node_modules/`, `target/`, `dist/`, `*.pyc`,...: the
//!   list is [`BUILD_OUTPUT`]): a `.env` or a notes file someone ignored is work too. Known build output is
//!   only counted;
//! - nested repositories and initialized submodules: their own working trees are not inspected here, so they
//!   are listed as such (fail closed).
//!
//! What a removal must not touch is reported separately:
//! - [`Inspection::unmerged`]: commits on the worktree's `HEAD` or its recorded branch that no other branch or
//!   remote-tracking ref contains. The branch is then kept (never deleted by a removal), so nothing is lost;
//! - [`Inspection::blockers`]: states in which nothing is removed at all: a merge, rebase, cherry-pick, revert or
//!   bisect in progress, a locked worktree, a detached `HEAD` with commits no ref holds (they would be gone),
//!   or anything the reader could not read;
//! - [`Inspection::stashes`]: stash entries made on the branch. They live in the repository's shared stash list
//!   and their commits stay reachable from it, so a removal does not lose them; they are shown, not counted.
//!
//! Every failure is an error (the caller refuses the removal): nothing here guesses "clean".
//!
//! Depends on: vw-changes (`discover`, `diff_against_head`, `SessionDir`, the runner), `error`. Called by:
//! vw-core-api (`inspect`, `remove`).
//! Invariants:
//! - Writes only our scratch folder (removed after each inspection) and, like the Changes tab, loose objects
//!   for changed files (unreachable, pruned by git's normal gc).
//! - [`Inspection::fingerprint`] covers listed paths, kinds, bytes, staged objects, unmerged commits and blockers: a removal names it,
//!   so it acts only on the state the user saw.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vw_changes::git::{Call, Index};
use vw_changes::{FileStatus, SessionDir};

use crate::error::WorktreeError;

#[path = "inspect_content.rs"]
mod content;

const READ: Duration = Duration::from_secs(30);
/// Unmerged commits named (the count is always complete).
pub const COMMITS_SHOWN: usize = 20;

/// Ignored names that are build output (rebuilt by a command), matched against an entry's last segment.
/// Everything else that is ignored counts as work a removal discards.
pub const BUILD_OUTPUT: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".parcel-cache",
    ".cache",
    "coverage",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".venv",
    "venv",
    ".gradle",
    ".DS_Store",
];
/// Ignored file endings that are build output.
const BUILD_SUFFIXES: &[&str] = &[".pyc", ".pyo", ".class", ".o", ".obj"];

static RUN: AtomicU64 = AtomicU64::new(0);

/// What kind of unsaved work a listed path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum LostKind {
    /// A new file.
    Added,
    /// Changed (working tree or index).
    Modified,
    /// Deleted (the removal does not bring it back either: listed so the user sees it).
    Deleted,
    /// Renamed.
    Renamed,
    /// File, symlink or submodule kind changed.
    TypeChanged,
    /// An ignored file or folder that is not known build output.
    Ignored,
    /// A nested repository or an initialized submodule: its own working tree is not inspected.
    Repository,
}

impl From<FileStatus> for LostKind {
    fn from(status: FileStatus) -> Self {
        match status {
            FileStatus::Added => Self::Added,
            FileStatus::Modified => Self::Modified,
            FileStatus::Deleted => Self::Deleted,
            FileStatus::Renamed => Self::Renamed,
            FileStatus::TypeChanged => Self::TypeChanged,
        }
    }
}

/// One path whose unsaved state goes with the folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LostFile {
    /// Path relative to the worktree root (a folder ends in `/`).
    pub path: String,
    /// What it is.
    pub kind: LostKind,
}

/// A worktree's state before a removal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    /// The folder exists.
    pub exists: bool,
    /// Its branch (short name), `None` when detached or gone.
    pub branch: Option<String>,
    /// Every path whose unsaved state a removal discards, sorted.
    pub files: Vec<LostFile>,
    /// Up to [`COMMITS_SHOWN`] commits (`<id> <subject>`) that no other branch or remote contains.
    pub unmerged: Vec<String>,
    /// How many such commits there are.
    pub unmerged_total: u32,
    /// How many ignored items are known build output (deleted with the folder, not listed).
    pub build_output: u32,
    /// Stash entries made on the branch (kept in the repository's stash list; shown, not lost).
    pub stashes: Vec<String>,
    /// Why nothing may be removed now (plain words); empty when a removal may proceed.
    pub blockers: Vec<String>,
    /// A hash over everything above that a removal acts on.
    pub fingerprint: String,
}

impl Inspection {
    /// The listed paths, sorted.
    #[must_use]
    pub fn paths(&self) -> Vec<String> {
        self.files.iter().map(|f| f.path.clone()).collect()
    }

    /// True when a removal loses no unsaved file (unmerged commits stay on the kept branch).
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.files.is_empty()
    }

    /// The inspection of a folder that does not exist (nothing to remove).
    #[must_use]
    pub fn gone() -> Self {
        let mut gone = Self {
            exists: false,
            branch: None,
            files: Vec::new(),
            unmerged: Vec::new(),
            unmerged_total: 0,
            build_output: 0,
            stashes: Vec::new(),
            blockers: Vec::new(),
            fingerprint: String::new(),
        };
        gone.fingerprint = fingerprint(&gone, &[]);
        gone
    }
}

fn fingerprint(i: &Inspection, content: &[u8]) -> String {
    let mut parts: Vec<Vec<u8>> = vec![
        vec![u8::from(i.exists)],
        i.branch.clone().unwrap_or_default().into_bytes(),
    ];
    for f in &i.files {
        parts.push(format!("{:?}\0{}", f.kind, f.path).into_bytes());
    }
    parts.push(i.unmerged_total.to_string().into_bytes());
    parts.extend(i.unmerged.iter().map(|c| c.clone().into_bytes()));
    parts.extend(i.blockers.iter().map(|b| b.clone().into_bytes()));
    parts.push(content.to_vec());
    let mut hash = blake3::Hasher::new();
    for part in parts {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(&part);
    }
    hash.finalize().to_hex().to_string()
}

fn nul_list(out: &[u8]) -> Vec<String> {
    out.split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(vw_changes::display)
        .collect()
}

fn is_build_output(entry: &str) -> bool {
    let name = entry
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(entry);
    BUILD_OUTPUT.contains(&name) || BUILD_SUFFIXES.iter().any(|s| name.ends_with(s))
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, WorktreeError> {
    Ok(Call::new(root, Index::User)
        .args(args)
        .timeout(READ)
        .run()?)
}

/// Inspects the worktree at `path` (its root). `record_branch` is the branch Vibe Work created for it (its
/// commits count too, when the worktree moved to another one); `scan_root` a private folder of ours for the
/// reader's temporary index.
///
/// # Errors
/// `NotGit` when the folder is no longer a git working tree; any git or file failure (fail closed).
pub fn inspect(
    path: &Path,
    record_branch: Option<&str>,
    scan_root: &Path,
) -> Result<Inspection, WorktreeError> {
    if !path.is_dir() {
        return Ok(Inspection::gone());
    }
    let Some(mut repo) = vw_changes::discover(path)? else {
        return Err(WorktreeError::NotGit(format!(
            "{} is no longer a git working tree",
            path.display()
        )));
    };
    // The whole worktree, new files included whatever `status.showUntrackedFiles` says.
    repo.scope = String::new();
    repo.tracked_only = false;
    let (root, git_dir) = (repo.root.clone(), repo.git_dir.clone());
    let mut blockers = in_progress(&git_dir);
    let mut files = changed_files(repo, scan_root, &mut blockers)?;
    let staged = git(
        &root,
        &["diff-index", "--cached", "--name-only", "-z", "HEAD", "--"],
    )?;
    for p in nul_list(&staged) {
        if !files.iter().any(|f| f.path == p) {
            files.push(LostFile {
                path: p,
                kind: LostKind::Modified,
            });
        }
    }
    let build_output = ignored(&root, &mut files)?;
    repositories(&root, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    let head = Call::new(&root, Index::User)
        .args(["symbolic-ref", "-q", "--short", "HEAD"])
        .timeout(READ)
        .output()?;
    let branch = head
        .ok()
        .then(|| String::from_utf8_lossy(&head.stdout).trim().to_owned());
    let (unmerged, unmerged_total) = unmerged(&root, branch.as_deref(), record_branch)?;
    if branch.is_none() && unmerged_total > 0 {
        blockers.push(format!(
            "The copy's HEAD is detached and holds {unmerged_total} commit(s) no branch contains: create a branch for them first."
        ));
    }
    let stashes = branch
        .as_deref()
        .or(record_branch)
        .map_or_else(Vec::new, |b| stashes_on(&git_dir, b));
    let mut found = Inspection {
        exists: true,
        branch,
        files,
        unmerged,
        unmerged_total,
        build_output,
        stashes,
        blockers,
        fingerprint: String::new(),
    };
    let bytes = content::fingerprint(&root, &found.files)?;
    // A staged change may differ even when the working file was restored to HEAD.
    let staged_objects = git(
        &root,
        &["diff-index", "--cached", "--raw", "-z", "HEAD", "--"],
    )?;
    let content = [bytes.as_bytes(), staged_objects.as_slice()].concat();
    found.fingerprint = fingerprint(&found, &content);
    Ok(found)
}

/// Tracked changes and new files (the Changes reader); a name it could not read is a blocker.
fn changed_files(
    repo: vw_changes::Repo,
    scan_root: &Path,
    blockers: &mut Vec<String>,
) -> Result<Vec<LostFile>, WorktreeError> {
    vw_changes::prepare_root(scan_root)?;
    let id = format!(
        "inspect-{}-{}-{}",
        vw_changes::fnv_hex(&[repo.root.to_string_lossy().as_bytes()]),
        std::process::id(),
        RUN.fetch_add(1, Ordering::Relaxed)
    );
    let dir = SessionDir::open(scan_root, &id)?;
    let diff = vw_changes::diff_against_head(repo, &dir);
    let _ = dir.remove();
    let diff = diff?;
    // The reader's notes name files it could not read or rules it could not check: never "clean" then.
    blockers.extend(
        diff.notes
            .iter()
            .map(|n| format!("Not fully inspected: {n}")),
    );
    Ok(diff
        .files
        .iter()
        .map(|f| LostFile {
            path: f.path.clone(),
            kind: f.status.into(),
        })
        .collect())
}

/// Lists ignored entries that are not build output into `files`; returns how many are build output.
fn ignored(root: &Path, files: &mut Vec<LostFile>) -> Result<u32, WorktreeError> {
    let out = git(
        root,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "-z",
        ],
    )?;
    let mut build = 0_u32;
    for entry in nul_list(&out) {
        if is_build_output(&entry) {
            build = build.saturating_add(1);
        } else {
            files.push(LostFile {
                path: entry,
                kind: LostKind::Ignored,
            });
        }
    }
    Ok(build)
}

/// Nested repositories (untracked folders git does not descend into) and initialized submodules.
fn repositories(root: &Path, files: &mut Vec<LostFile>) -> Result<(), WorktreeError> {
    let out = git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for entry in nul_list(&out).into_iter().filter(|e| e.ends_with('/')) {
        files.push(LostFile {
            path: entry,
            kind: LostKind::Repository,
        });
    }
    let tree = git(root, &["ls-tree", "-r", "-z", "--full-tree", "HEAD"])?;
    for line in nul_list(&tree) {
        let Some((meta, rel)) = line.split_once('\t') else {
            continue;
        };
        if !meta.starts_with("160000 ") {
            continue;
        }
        let populated = std::fs::read_dir(root.join(rel)).is_ok_and(|mut d| d.next().is_some());
        if populated {
            files.push(LostFile {
                path: format!("{rel}/"),
                kind: LostKind::Repository,
            });
        }
    }
    Ok(())
}

/// Commits on `HEAD` (and on the recorded branch) that no other branch or remote-tracking ref contains.
fn unmerged(
    root: &Path,
    head_branch: Option<&str>,
    record_branch: Option<&str>,
) -> Result<(Vec<String>, u32), WorktreeError> {
    let mut args: Vec<String> = vec!["HEAD".into()];
    let record_exists = record_branch.is_some_and(|b| {
        Call::new(root, Index::User)
            .args(["show-ref", "--verify", "-q", &format!("refs/heads/{b}")])
            .timeout(READ)
            .output()
            .is_ok_and(|o| o.ok())
    });
    if let (true, Some(b)) = (record_exists, record_branch) {
        args.push(format!("refs/heads/{b}"));
    }
    args.push("--not".into());
    for b in [head_branch, record_branch].into_iter().flatten() {
        // `--branches` matches names without `refs/heads/` (git-rev-list: "--exclude").
        args.push(format!("--exclude={b}"));
    }
    args.push("--branches".into());
    args.push("--remotes".into());
    let mut count_args = vec!["rev-list".to_owned(), "--count".to_owned()];
    count_args.extend(args.iter().cloned());
    let total_text = git(
        root,
        &count_args.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    // Unreadable output is an error, never "0 commits" (that would let a removal delete the branch).
    let total: u32 = String::from_utf8_lossy(&total_text)
        .trim()
        .parse()
        .map_err(|_| {
            WorktreeError::Git(vw_changes::ChangesError::Parse(
                "git rev-list --count printed no number".into(),
            ))
        })?;
    let mut list_args = vec![
        "rev-list".to_owned(),
        format!("--max-count={COMMITS_SHOWN}"),
        "--format=%h %s".to_owned(),
    ];
    list_args.extend(args);
    let listed = git(
        root,
        &list_args.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let commits = String::from_utf8_lossy(&listed)
        .lines()
        .filter(|l| !l.starts_with("commit "))
        .map(str::to_owned)
        .collect();
    Ok((commits, total))
}

/// A merge, rebase, cherry-pick, revert or bisect in progress, or a locked worktree.
fn in_progress(git_dir: &Path) -> Vec<String> {
    let states = [
        ("MERGE_HEAD", "a merge is in progress"),
        ("rebase-merge", "a rebase is in progress"),
        ("rebase-apply", "a rebase or patch apply is in progress"),
        ("CHERRY_PICK_HEAD", "a cherry-pick is in progress"),
        ("REVERT_HEAD", "a revert is in progress"),
        ("BISECT_LOG", "a bisect is in progress"),
        ("locked", "the worktree is locked (git worktree lock)"),
    ];
    states
        .iter()
        .filter(|(name, _)| git_dir.join(name).exists())
        .map(|(_, why)| format!("In the copy, {why}."))
        .collect()
}

/// Stash entries made on `branch`, read from the shared stash reflog (no git process: `git stash list` runs
/// `git log`, whose config could run a program).
fn stashes_on(git_dir: &Path, branch: &str) -> Vec<String> {
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .map_or_else(|_| git_dir.to_path_buf(), |c| git_dir.join(c.trim()));
    let Ok(log) = std::fs::read_to_string(common.join("logs/refs/stash")) else {
        return Vec::new();
    };
    let (wip, on) = (format!("WIP on {branch}:"), format!("On {branch}:"));
    log.lines()
        .filter_map(|l| l.split_once('\t').map(|(_, m)| m.trim().to_owned()))
        .filter(|m| m.starts_with(&wip) || m.starts_with(&on))
        .collect()
}
