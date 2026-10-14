//! File search: file names and lines in the sessions' folders
//! that hold every word of the query. Blocking work; the caller runs it on `spawn_blocking`.
//!
//! Which files: in a git working tree, `git ls-files -co --exclude-standard -z` (tracked plus untracked files that are
//! not ignored), through the app's one hardened git runner (`vw_changes::git`, read-only: git lists names and reads
//! no file content, so no filter or driver runs; `GIT_OPTIONAL_LOCKS=0` keeps it from writing the index). Elsewhere,
//! (and when git lists nothing: an empty repository, or a folder an enclosing repository ignores) a walk that skips
//! hidden folders and build/dependency folders ([`SKIP_DIRS`]). Either way: no hidden file or folder, no symbolic links
//! and no file under a linked folder (a link could lead outside the folder), no file over [`MAX_FILE`], no file whose first 8 KiB hold a NUL byte.
//!
//! Budgets: [`MAX_FILES`] files read, [`MAX_BYTES`] read in all, [`DEADLINE`]; past one of them the answer says
//! `truncated`. Hits: names first, then lines in folder and file order, [`COUNT_CAP`] counted.
//!
//! Depends on: `search_text`, vw-changes (`git`). Called by: `search` (`search files`).
//! Invariants: only paths under a searched folder are read (git's names are joined to the folder and checked to stay
//! inside it lexically, with no `..` and no absolute part); nothing is written.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use crate::search_text::{Query, find, snippet};
use crate::search_view::FileHit;

/// Largest file read.
const MAX_FILE: u64 = 1 << 20;
/// Most files read per search.
const MAX_FILES: u32 = 20_000;
/// Most bytes read per search.
const MAX_BYTES: u64 = 200 << 20;
/// Longest a search may take.
const DEADLINE: Duration = Duration::from_secs(4);
/// Hits counted at most.
const COUNT_CAP: u32 = 1000;
/// Folders a walk never enters (dependencies, builds, caches); hidden folders are skipped too.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "venv",
    "__pycache__",
    "coverage",
    "vendor",
];

/// One folder to search and the session to open its files in.
#[derive(Debug, Clone)]
pub(crate) struct Folder {
    /// The folder (absolute).
    pub path: PathBuf,
    /// A session working in it.
    pub session_id: Option<String>,
}

/// What a search found.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub hits: Vec<FileHit>,
    pub total: u32,
    pub scanned: u32,
    pub truncated: bool,
}

/// The relative paths of the files to search in `folder`.
fn candidates(folder: &Path) -> Vec<String> {
    let listed = vw_changes::git::Call::new(folder, vw_changes::git::Index::User)
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .timeout(Duration::from_secs(3))
        .max_output(32 << 20)
        .output();
    match listed {
        Ok(out) if out.ok() => {
            let names: Vec<String> = out
                .stdout
                .split(|b| *b == 0)
                .filter(|name| !name.is_empty())
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .collect();
            // Nothing listed: an empty repository, or a folder an enclosing repository ignores (a session folder
            // inside a dotfiles home, say); its files are still the session's, so walk it.
            if names.is_empty() {
                walk(folder)
            } else {
                names
            }
        }
        _ => walk(folder),
    }
}

/// A plain walk (no git): every file under `folder`, hidden and [`SKIP_DIRS`] folders skipped, links not followed.
fn walk(folder: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(folder.join(&rel)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let child = rel.join(&name);
            if kind.is_dir() {
                if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(child);
                }
            } else if kind.is_file() {
                out.push(child.to_string_lossy().replace('\\', "/"));
            }
            if out.len() >= MAX_FILES as usize * 2 {
                return out;
            }
        }
    }
    out.sort();
    out
}

/// True when `rel` is a plain relative path (no `..`, no root, no prefix).
fn plain_relative(rel: &str) -> bool {
    Path::new(rel)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Searches `folders` for `query`; at most `limit` hits are returned.
pub(crate) fn search(folders: &[Folder], query: &Query, limit: usize) -> Found {
    let started = Instant::now();
    let mut found = Found::default();
    let mut bytes: u64 = 0;
    let mut names: Vec<FileHit> = Vec::new();
    let mut lines: Vec<FileHit> = Vec::new();
    'folders: for folder in folders {
        for rel in candidates(&folder.path) {
            if found.scanned >= MAX_FILES || bytes >= MAX_BYTES || started.elapsed() >= DEADLINE {
                found.truncated = true;
                break 'folders;
            }
            // Hidden files and folders are skipped in git's list too, like the walk and the Files view's default
            // scope (a tracked `.env` could otherwise be searched).
            if !plain_relative(&rel) || rel.split('/').any(|part| part.starts_with('.')) {
                continue;
            }
            let hit = |line: Option<u32>, text: String| FileHit {
                folder: folder.path.display().to_string(),
                path: rel.clone(),
                line,
                text,
                session_id: folder.session_id.clone(),
            };
            if find(&rel, query).is_some() {
                found.total = found.total.saturating_add(1).min(COUNT_CAP);
                names.push(hit(None, rel.clone()));
            }
            let path = folder.path.join(&rel);
            // No links (they may point outside the folder), only regular files under the size cap.
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() > MAX_FILE {
                continue;
            }
            // Nor a file under a linked folder: git lists names from its index, and a tracked folder replaced by a
            // link would lead outside, including through a symlinked parent.
            if vw_changes::has_symlinked_parent(&folder.path, &path) {
                continue;
            }
            let Ok(content) = std::fs::read(&path) else {
                continue;
            };
            found.scanned += 1;
            bytes += content.len() as u64;
            if content[..content.len().min(8192)].contains(&0) {
                continue;
            }
            let text = String::from_utf8_lossy(&content);
            for (index, line) in text.lines().enumerate() {
                let Some((at, _)) = find(line, query) else {
                    continue;
                };
                found.total = found.total.saturating_add(1).min(COUNT_CAP);
                if lines.len() < limit {
                    lines.push(hit(
                        Some(u32::try_from(index + 1).unwrap_or(u32::MAX)),
                        snippet(line, at),
                    ));
                }
            }
        }
    }
    found.hits = names.into_iter().chain(lines).take(limit).collect();
    found
}
