//! File search: file names and lines in the sessions' folders
//! that hold every word of the query. Blocking work; the caller runs it on `spawn_blocking`.
//!
//! Which files: in a git working tree, `git ls-files -co --exclude-standard -z` (tracked plus untracked files that are
//! not ignored), through the app's one hardened git runner (`vw_changes::git`, read-only: git lists names and reads
//! no file content, so no filter or driver runs; `GIT_OPTIONAL_LOCKS=0` keeps it from writing the index). Elsewhere,
//! and when an enclosing repository ignores the session folder itself, a walk that skips
//! hidden folders and build/dependency folders ([`SKIP_DIRS`]). Either way: no hidden file or folder, no symbolic links
//! and no file under a linked folder (a link could lead outside the folder), no file over [`MAX_FILE`], no file whose first 8 KiB hold a NUL byte.
//!
//! Budgets: [`MAX_FILES`] files read, [`MAX_BYTES`] read in all, [`DEADLINE`]; past one of them the answer says
//! `truncated`. Hits: names first, then lines in folder and file order, [`COUNT_CAP`] counted.
//!
//! Name-only search (the composer's @ menu) reads no content: it ranks every listed name by `search_fuzzy` (best
//! first, then shorter paths, then path order) and checks only the best ones on disk; an empty query lists names in
//! folder and file order.
//!
//! Depends on: `search_text`, vw-changes (`git`). Called by: `search` (`search files`), `files_lookup` (names only).
//! Invariants: only paths under a searched folder are read (git's names are joined to the folder and checked to stay
//! inside it lexically, with no `..` and no absolute part); nothing is written.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use crate::search_fuzzy;
use crate::search_text::{Query, find, snippet};
use crate::search_view::FileHit;

/// Largest file read.
pub(crate) const MAX_FILE: u64 = 1 << 20;
/// Most files read per search.
pub(crate) const MAX_FILES: u32 = 20_000;
/// Most bytes read per search.
pub(crate) const MAX_BYTES: u64 = 200 << 20;
/// Longest a search may take.
const DEADLINE: Duration = Duration::from_secs(4);
/// Hits counted at most.
const COUNT_CAP: u32 = 1000;
/// Folders a walk never enters (dependencies, builds, caches); hidden folders are skipped too.
pub(crate) const SKIP_DIRS: &[&str] = &[
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

/// Shared listing of visible file names in `folder`, with dependency/build folders omitted.
pub(crate) fn candidates(folder: &Path) -> Vec<String> {
    crate::search_walk::candidates(folder, Instant::now() + DEADLINE, &|_| true).0
}

/// True when `rel` is a plain relative path (no `..`, no root, no prefix).
pub(crate) fn plain_relative(rel: &str) -> bool {
    Path::new(rel)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// A listed name that can be shown: a regular file, not a link, not under a linked folder.
fn listable(folder: &Path, rel: &str) -> bool {
    crate::search_walk::open(folder, rel).is_ok()
}

/// Name-only search: no content is read (`scanned` stays 0). Every listed name is ranked by [`search_fuzzy::score`]
/// of the query (its words joined); only the best names are checked on disk, until `limit` are found.
fn names(folders: &[Folder], query: Option<&Query>, limit: usize) -> Found {
    let started = Instant::now();
    let wanted = query.map(Query::compact).unwrap_or_default();
    let mut found = Found::default();
    let mut ranked: Vec<(i64, usize, String)> = Vec::new();
    'folders: for (index, folder) in folders.iter().enumerate() {
        for rel in candidates(&folder.path) {
            if ranked.len() >= MAX_FILES as usize || started.elapsed() >= DEADLINE {
                found.truncated = true;
                break 'folders;
            }
            // Hidden files and folders are skipped in the listing too, like the walk and the Files view's default scope.
            if !plain_relative(&rel) || rel.split('/').any(|part| part.starts_with('.')) {
                continue;
            }
            if let Some(score) = search_fuzzy::score(search_fuzzy::target(&rel, &wanted), &wanted) {
                ranked.push((score, index, rel));
            }
        }
    }
    found.total = u32::try_from(ranked.len())
        .unwrap_or(u32::MAX)
        .min(COUNT_CAP);
    if !wanted.is_empty() {
        // A stable sort: equal scores keep folder and file order after the shorter path.
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.len().cmp(&b.2.len())));
    }
    for (_, index, rel) in ranked {
        if found.hits.len() >= limit || started.elapsed() >= DEADLINE {
            break;
        }
        let Some(folder) = folders.get(index) else {
            continue;
        };
        if listable(&folder.path, &rel) {
            found.hits.push(FileHit {
                folder: folder.path.display().to_string(),
                path: rel.clone(),
                line: None,
                text: rel,
                session_id: folder.session_id.clone(),
            });
        }
    }
    found
}

/// Searches `folders` for `query`; at most `limit` hits are returned.
pub(crate) fn search(
    folders: &[Folder],
    query: Option<&Query>,
    limit: usize,
    names_only: bool,
) -> Found {
    if names_only {
        return names(folders, query, limit);
    }
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
            if query.is_some_and(|query| find(&rel, query).is_some()) {
                found.total = found.total.saturating_add(1).min(COUNT_CAP);
                names.push(hit(None, rel.clone()));
            }
            let Some(query) = query else {
                continue;
            };
            let Some(content) = crate::search_walk::read(&folder.path, &rel) else {
                continue;
            };
            found.scanned += 1;
            bytes += content.len() as u64;
            if content[..content.len().min(8192)].contains(&0) {
                continue;
            }
            let text = String::from_utf8_lossy(&content);
            let visible;
            let source: Box<dyn Iterator<Item = &str>> = if Path::new(&rel)
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm")
                }) {
                visible =
                    crate::search_html::visible_lines(&text, || started.elapsed() >= DEADLINE);
                found.truncated |= visible.truncated;
                Box::new(visible.lines.iter().map(String::as_str))
            } else {
                Box::new(text.lines())
            };
            for (index, line) in source.enumerate() {
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
