//! Reading the working tree into index entries without letting git read it: we
//! list paths with `git ls-files` (names and `.gitignore` only), read files in Rust (a symlink's target, never
//! through it), and store their bytes with `git hash-object --no-filters`. No repository filter,
//! fsmonitor or hook runs.
//!
//! Depends on: `objects`, `statcache`, `git`, `paths`, `repo`, `error`. Called by: `baseline`, `diff`.
//! Invariants:
//! - Untracked files over [`LARGE_UNTRACKED`] are not stored (reported instead); more than
//!   [`MAX_UNTRACKED`] untracked files switch the review to tracked files only, with a note; the
//!   repository's `status.showUntrackedFiles=no` means tracked files only.
//! - Paths are git's bytes throughout (`-z` lists, byte-keyed entries); a name that cannot be read here is
//!   reported in the notes, never silently left out (integrity check). Paths under a symlinked
//!   folder are not read (git does not track through them either; the symlink itself is listed).
//! - On a case-insensitive file system a name must exist exactly as listed (`ExactNames`), so a case-only
//!   rename shows as a rename.
//! - Submodules (gitlinks) keep their baseline entry: they are reviewed in their own repository.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ChangesError;
use crate::git::{Call, Index};
use crate::ignoresnap::IgnoreSnapshot;
use crate::objects::{Entries, Entry, hash_bytes, hash_files};
use crate::paths;
use crate::repo::Repo;
use crate::statcache::{Stat, StatCache};

/// Untracked files bigger than this are not stored in the repository (bytes).
pub const LARGE_UNTRACKED: u64 = 20 * 1024 * 1024;
/// More untracked files than this and the review covers tracked files only.
pub const MAX_UNTRACKED: usize = 20_000;

/// An untracked file left out for its size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    /// Repository-relative path.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// Modification time (ms since the epoch, 0 when unknown).
    pub modified_ms: u64,
    /// The name as git's bytes (`path` is its shown form).
    #[serde(default)]
    pub raw: Vec<u8>,
}

/// The working tree as index entries.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    /// Set while taking the baseline when there were too many untracked files to review them.
    pub(crate) tracked_only: bool,
    pub(crate) entries: Entries,
    pub(crate) large: Vec<LargeFile>,
    pub(crate) notes: Vec<String>,
}

fn in_scope(repo: &Repo, path: &[u8]) -> bool {
    let scope = repo.scope.as_bytes();
    scope.is_empty()
        || path == scope
        || path
            .strip_prefix(scope)
            .is_some_and(|rest| rest.first() == Some(&b'/'))
}

fn nul_list(out: &[u8]) -> Vec<Vec<u8>> {
    out.split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

/// Untracked, not ignored files in the scope, as git's bytes (names only: `ls-files` reads no file
/// content). Folders git reports (`sub/`, a nested repository) are left out.
fn untracked(repo: &Repo, index: &Path) -> Result<Vec<Vec<u8>>, ChangesError> {
    let out = Call::new(&repo.root, Index::Ours(index))
        .args(["ls-files", "--others", "--exclude-standard", "-z", "--"])
        .args([repo.scope_pathspec()])
        .run()?;
    Ok(nul_list(&out)
        .into_iter()
        .filter(|p| p.last() != Some(&b'/'))
        .collect())
}

/// Ignored paths in the scope, folders collapsed (`dist/`), by today's rules. The watcher uses
/// [`watch_ignored`], which also needs the baseline's rules to agree.
///
/// # Errors
/// git failures.
pub fn ignored(repo: &Repo, index: &Path) -> Result<Vec<String>, ChangesError> {
    let out = Call::new(&repo.root, Index::Ours(index))
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "-z",
            "--",
        ])
        .args([repo.scope_pathspec()])
        .run()?;
    Ok(nul_list(&out)
        .into_iter()
        .map(|p| String::from_utf8_lossy(&p).into_owned())
        .collect())
}

#[cfg(unix)]
fn link_bytes(target: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    target.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn link_bytes(target: &Path) -> Vec<u8> {
    target.to_string_lossy().replace('\\', "/").into_bytes()
}

#[cfg(unix)]
fn file_mode(meta: &std::fs::Metadata, _base: Option<&Entry>) -> &'static str {
    use std::os::unix::fs::PermissionsExt;
    if meta.permissions().mode() & 0o111 != 0 {
        "100755"
    } else {
        "100644"
    }
}

/// Windows has no executable bit: keep what the baseline says (git's `core.fileMode=false`).
#[cfg(not(unix))]
fn file_mode(_meta: &std::fs::Metadata, base: Option<&Entry>) -> &'static str {
    if base.is_some_and(|b| b.mode == "100755") {
        "100755"
    } else {
        "100644"
    }
}

/// On a case-insensitive file system (macOS APFS by default, Windows NTFS), `lstat("readme.txt")` also finds
/// `README.txt`, and git (`core.ignoreCase`, set by `git init` there) does not list `README.txt` as new either:
/// a case-only rename would look like no change. This checks every component of a name as its folder lists it
/// (one listing per folder, cached) and finds the spelling the file system has now ([`ExactNames::case_twin`]),
/// so the scan sees the old name gone and the new one present: a rename. Folder renames (`Docs/` -> `docs/`)
/// are covered the same way. Case only: names that differ by Unicode normalization (APFS also ignores it) are
/// not matched.
struct ExactNames {
    insensitive: bool,
    root: PathBuf,
    listings: std::collections::HashMap<PathBuf, std::collections::HashSet<std::ffi::OsString>>,
}

impl ExactNames {
    fn new(root: &Path) -> Self {
        Self {
            insensitive: case_insensitive(root),
            root: root.to_path_buf(),
            listings: std::collections::HashMap::new(),
        }
    }

    /// The names in `dir` as the file system spells them (empty when it cannot be listed).
    fn listing(&mut self, dir: &Path) -> &std::collections::HashSet<std::ffi::OsString> {
        self.listings.entry(dir.to_path_buf()).or_insert_with(|| {
            std::fs::read_dir(dir)
                .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
                .unwrap_or_default()
        })
    }

    /// The components of `full` below the root (empty when `full` is not under it).
    fn parts(&self, full: &Path) -> Vec<std::ffi::OsString> {
        full.strip_prefix(&self.root)
            .map(|rel| rel.iter().map(std::ffi::OsStr::to_os_string).collect())
            .unwrap_or_default()
    }

    /// True when every component of `full` below the root exists exactly as written (always true on a
    /// case-sensitive file system, where `lstat` already compares exactly).
    fn exists_exactly(&mut self, full: &Path) -> bool {
        if !self.insensitive {
            return true;
        }
        let mut dir = self.root.clone();
        for part in self.parts(full) {
            if !self.listing(&dir).contains(&part) {
                return false;
            }
            dir.push(part);
        }
        true
    }

    /// `full`'s repository-relative path as the file system spells it now, when that differs from how it is
    /// written only by case; `None` when it exists exactly, does not exist, or the file system is
    /// case-sensitive.
    fn case_twin(&mut self, full: &Path) -> Option<Vec<u8>> {
        if !self.insensitive {
            return None;
        }
        let mut dir = self.root.clone();
        let mut spelled: Vec<Vec<u8>> = Vec::new();
        let mut differs = false;
        for part in self.parts(full) {
            let listing = self.listing(&dir);
            let actual = if listing.contains(&part) {
                part
            } else {
                let wanted = part.to_string_lossy().to_lowercase();
                differs = true;
                listing
                    .iter()
                    .find(|name| name.to_string_lossy().to_lowercase() == wanted)?
                    .clone()
            };
            spelled.push(paths::os_bytes(&actual));
            dir.push(actual);
        }
        (differs && !spelled.is_empty()).then(|| spelled.join(&b'/'))
    }
}

/// The scan's starting point: entries kept as they are (outside the scope, submodules) and the paths to
/// read (tracked in the scope + untracked, not ignored).
fn gather(
    repo: &Repo,
    base: &Entries,
    list_index: &Path,
    rules: Option<&IgnoreSnapshot>,
) -> Result<(Scan, Vec<Vec<u8>>), ChangesError> {
    let mut scan = Scan::default();
    let mut candidates: Vec<Vec<u8>> = Vec::new();
    for (path, entry) in base {
        if !in_scope(repo, path) || entry.mode == "160000" {
            scan.entries.insert(path.clone(), entry.clone());
        } else {
            candidates.push(path.clone());
        }
    }
    if repo.tracked_only {
        scan.notes.push("New files are not shown: this repository hides untracked files (status.showUntrackedFiles=no).".into());
        return Ok((scan, candidates));
    }
    let mut found = untracked(repo, list_index)?;
    let Some(rules) = rules else {
        // Taking the baseline: today's rules are the baseline's rules. Too many untracked files switch this
        // session to tracked files only (decided here, once; `scan.tracked_only`).
        if found.len() > MAX_UNTRACKED {
            scan.notes.push(format!(
                "{} untracked files: new files are not shown (add them to .gitignore or commit them to review them).",
                found.len()
            ));
            scan.tracked_only = true;
        } else {
            candidates.extend(found);
        }
        return Ok((scan, candidates));
    };
    // After the baseline: new files the rules *now* ignore but the baseline's rules did not (the agent
    // wrote a `.gitignore` line, `.git/info/exclude`, or `core.excludesFile`) are listed all the same. No cap here: a
    // flood of new files makes the review slow, never blind.
    let (revived, note) = revived(repo, list_index, rules)?;
    found.extend(revived);
    scan.notes.extend(note);
    candidates.extend(found);
    Ok((scan, candidates))
}

/// Files ignored now (by today's rules) that the baseline's rules do not ignore.
/// Fail closed: when git cannot answer for the baseline's rules, nothing counts as ignored (every such file
/// is listed) and the returned note says so.
fn revived(
    repo: &Repo,
    index: &Path,
    rules: &IgnoreSnapshot,
) -> Result<(Vec<Vec<u8>>, Option<String>), ChangesError> {
    let out = Call::new(&repo.root, Index::Ours(index))
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "-z",
            "--",
        ])
        .args([repo.scope_pathspec()])
        .run()?;
    let ignored_now = nul_list(&out);
    let mut note = None;
    let mut fail_open = |error: ChangesError| {
        note = Some(format!(
            "The ignore rules from the start of the session could not be checked ({error}): every file git ignores now is listed."
        ));
        std::collections::HashSet::new()
    };
    let (dirs, plain): (Vec<Vec<u8>>, Vec<Vec<u8>>) = ignored_now
        .into_iter()
        .partition(|e| e.last() == Some(&b'/'));
    let dirs: Vec<Vec<u8>> = dirs
        .into_iter()
        .map(|d| d[..d.len() - 1].to_vec())
        .collect();
    let dirs_then = rules.ignored_dirs(&dirs).unwrap_or_else(&mut fail_open);
    let mut files: Vec<Vec<u8>> = Vec::new();
    for dir in dirs.iter().filter(|d| !dirs_then.contains(*d)) {
        files.extend(walk(&repo.root, dir));
    }
    files.extend(plain);
    let files_then = rules.ignored(&files).unwrap_or_else(&mut fail_open);
    files.retain(|f| !files_then.contains(f));
    Ok((files, note))
}

/// Every file under the folder `rel` (repository-relative bytes), symlinks as entries, never following a
/// symlinked folder, never entering a nested repository.
fn walk(root: &Path, rel: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut stack = vec![rel.to_vec()];
    while let Some(dir) = stack.pop() {
        let Ok(full) = paths::inside_bytes(root, &dir) else {
            continue;
        };
        if full.join(".git").exists() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&full) else {
            continue;
        };
        for entry in entries.flatten() {
            let mut child = dir.clone();
            child.push(b'/');
            child.extend_from_slice(&paths::os_bytes(&entry.file_name()));
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(child),
                Ok(_) => out.push(child),
                Err(_) => {}
            }
        }
    }
    out
}

/// The entry of the symlink at `full` (mode 120000, its target's bytes as the blob), from the stat cache when
/// `trust` allows it.
fn link_entry(
    repo: &Repo,
    list_index: &Path,
    cache: &mut StatCache,
    trust: bool,
    full: &Path,
    key: String,
    stat: Stat,
) -> Result<Entry, ChangesError> {
    let target = std::fs::read_link(full).map_err(|e| ChangesError::io("read link", full, e))?;
    let cached = cache
        .hit(&key, &stat, "120000")
        .filter(|_| trust)
        .map(str::to_owned);
    let id = if let Some(id) = cached {
        id
    } else {
        let id = hash_bytes(repo, list_index, &link_bytes(&target))?;
        cache.put(key, stat, "120000".into(), id.clone());
        id
    };
    Ok(Entry {
        mode: "120000".into(),
        id,
    })
}

/// Reads the working tree: `base` gives the tracked paths (and keeps everything outside the scope and
/// every submodule as it is), `list_index` holds the same entries for `ls-files --others`. With `trust`
/// false (and on platforms where the cache is not trusted) every file is read and hashed again.
pub(crate) fn scan(
    repo: &Repo,
    base: &Entries,
    list_index: &Path,
    cache: &mut StatCache,
    trust: bool,
    rules: Option<&IgnoreSnapshot>,
) -> Result<Scan, ChangesError> {
    let trust = trust && crate::statcache::trusted();
    let (mut scan, mut candidates) = gather(repo, base, list_index, rules)?;
    let mut exact = ExactNames::new(&repo.root);
    // Every path already on the list, so a case twin found below is added once (ExactNames).
    let mut listed: std::collections::HashSet<Vec<u8>> = candidates.iter().cloned().collect();
    let mut to_hash: Vec<(Vec<u8>, PathBuf, Stat, &'static str)> = Vec::new();
    let mut next = 0;
    while next < candidates.len() {
        let rel = candidates[next].clone();
        next += 1;
        let full = match paths::inside_bytes(&repo.root, &rel) {
            Ok(full) => full,
            Err(error) => {
                // Never silently out of the list: say which name could not be read and why.
                scan.notes.push(format!("Not reviewed: {error}"));
                continue;
            }
        };
        if paths::has_symlinked_parent(&repo.root, &full) {
            continue;
        }
        if !exact.exists_exactly(&full) {
            // Gone under this spelling; a case-only rename leaves it under another one, which git does not
            // list as new on this file system: read that one as a new path (it is the same file's new name).
            if let Some(twin) = exact.case_twin(&full)
                && in_scope(repo, &twin)
                && listed.insert(twin.clone())
            {
                candidates.push(twin);
            }
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&full) else {
            continue;
        };
        let stat = Stat::of(&meta);
        let key = paths::display(&rel);
        let tracked = base.get(&rel);
        if meta.file_type().is_symlink() {
            let entry = link_entry(repo, list_index, cache, trust, &full, key, stat)?;
            scan.entries.insert(rel, entry);
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        if tracked.is_none() && meta.len() > LARGE_UNTRACKED {
            scan.large.push(LargeFile {
                raw: rel.clone(),
                path: key,
                size: meta.len(),
                modified_ms: u64::try_from(stat.mtime_ns / 1_000_000).unwrap_or(u64::MAX),
            });
            continue;
        }
        let mode = file_mode(&meta, tracked);
        match cache.hit(&key, &stat, mode).filter(|_| trust) {
            Some(id) => {
                let id = id.to_owned();
                scan.entries.insert(
                    rel,
                    Entry {
                        mode: mode.into(),
                        id,
                    },
                );
            }
            None => to_hash.push((rel, full, stat, mode)),
        }
    }
    if !to_hash.is_empty() {
        let paths: Vec<PathBuf> = to_hash.iter().map(|t| t.1.clone()).collect();
        let ids = hash_files(repo, list_index, &paths)?;
        for ((rel, _, stat, mode), id) in to_hash.into_iter().zip(ids) {
            cache.put(paths::display(&rel), stat, mode.into(), id.clone());
            scan.entries.insert(
                rel,
                Entry {
                    mode: mode.into(),
                    id,
                },
            );
        }
    }
    let kept: std::collections::HashSet<String> =
        scan.entries.keys().map(|k| paths::display(k)).collect();
    cache.retain(|k| kept.contains(k));
    Ok(scan)
}

/// What the file watcher may drop for a session: paths ignored today *and* by the baseline's rules (a rule
/// the agent added never silences the watcher). Without a stored baseline: today's rules.
///
/// # Errors
/// git failures.
pub fn watch_ignored(
    repo: &Repo,
    session: &crate::store::SessionDir,
) -> Result<Vec<String>, ChangesError> {
    let index = session.current_index();
    let now = ignored(repo, &index)?;
    let Some(rules) = IgnoreSnapshot::load(session) else {
        return Ok(now);
    };
    // Same folder rule as `revived`: folders are asked as folders. On any error nothing is dropped.
    let mut keep = Vec::new();
    for path in now {
        let (bytes, dir) = match path.strip_suffix('/') {
            Some(d) => (d.as_bytes().to_vec(), true),
            None => (path.as_bytes().to_vec(), false),
        };
        let answer = if dir {
            rules.ignored_dirs(std::slice::from_ref(&bytes))
        } else {
            rules.ignored(std::slice::from_ref(&bytes))
        };
        if answer.is_ok_and(|set| set.contains(&bytes)) {
            keep.push(path);
        }
    }
    Ok(keep)
}

/// Whether names in the working tree at `root` compare without case (Windows, macOS by default): `.git`
/// exists in every working tree root (folder or file), its upper-case twin only then. Used by the diff to
/// mark case-only renames (`FileChange::case_only`), which a revert renames back instead of deleting.
pub(crate) fn case_insensitive(root: &Path) -> bool {
    std::fs::symlink_metadata(root.join(".GIT")).is_ok()
        && std::fs::symlink_metadata(root.join(".git")).is_ok()
}
