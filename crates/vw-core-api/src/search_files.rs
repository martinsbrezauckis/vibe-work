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

/// Shared listing of visible file names in `folder`, with dependency/build folders omitted.
pub(crate) fn candidates(folder: &Path) -> Vec<String> {
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
            // Keep standalone session folders that an enclosing repository ignores searchable. An empty
            // list inside the session's repository must stay empty: walking would expose ignored files.
            if names.is_empty()
                && std::fs::symlink_metadata(folder.join(".git")).is_err()
                && vw_changes::git::Call::new(folder, vw_changes::git::Index::User)
                    .args(["check-ignore", "-q", "."])
                    .timeout(Duration::from_secs(3))
                    .max_output(1024)
                    .output()
                    .is_ok_and(|out| out.ok())
            {
                walk(folder)
            } else {
                names
            }
        }
        _ => walk(folder),
    }
    .into_iter()
    .filter(|rel| {
        plain_relative(rel)
            && !rel
                .split('/')
                .any(|part| part.starts_with('.') || SKIP_DIRS.contains(&part))
    })
    .collect()
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

/// A listed name that can be shown: a regular file, not a link, not under a linked folder.
fn listable(folder: &Path, rel: &str) -> bool {
    let path = folder.join(rel);
    std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file())
        && !vw_changes::has_symlinked_parent(folder, &path)
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
            let Some(query) = query else {
                continue;
            };
            let Ok(content) = std::fs::read(&path) else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_output_limit_marks_search_truncated() {
        let root = std::env::temp_dir().join(format!("vw-search-limit-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("page.html"), "\n".repeat(1 << 19)).unwrap();
        let found = search(
            &[Folder {
                path: root.clone(),
                session_id: None,
            }],
            Some(&Query::new("missing").unwrap()),
            20,
            false,
        );
        assert!(found.truncated);
        assert_eq!(found.scanned, 1);
        assert_eq!(found.hits.len(), 0, "{:?}", found.hits);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn html_file_hits_show_visible_words_and_keep_source_lines() {
        let root = std::env::temp_dir().join(format!("vw-search-visible-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let html = "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Where forecasts are cached</title>\n<meta name=\"description\" content=\"Where forecasts are cached.\"><style>forecast { color: red }</style>\n<script>\nconst forecast = '<p>hidden</p>';\n</script></head><body><h1>Where forecasts are cached</h1>\n<p>The forecast should <b>still show</b> when the network drops &amp; returns.</p></body></html>";
        std::fs::write(root.join("decision.html"), html).unwrap();
        std::fs::write(root.join("decision.htm"), html).unwrap();
        std::fs::write(root.join("notes.txt"), "<meta>forecast</meta>\n").unwrap();
        let folders = [Folder {
            path: root.clone(),
            session_id: None,
        }];
        let found = search(&folders, Some(&Query::new("forecast").unwrap()), 20, false);
        for path in ["decision.html", "decision.htm"] {
            let hits: Vec<_> = found.hits.iter().filter(|h| h.path == path).collect();
            assert!(hits.len() >= 2, "{hits:?}");
            assert!(
                hits.iter().all(|h| !h.text.contains('<')
                    && !h.text.contains("width=")
                    && !h.text.contains("description")
                    && !h.text.contains("const ")),
                "{hits:?}"
            );
            assert!(
                hits.iter()
                    .any(|h| h.text.contains("Where forecasts are cached"))
            );
            assert!(
                hits.iter()
                    .any(|h| h.line == Some(6) && h.text.contains("& returns"))
            );
        }
        assert!(
            found
                .hits
                .iter()
                .any(|h| h.path == "notes.txt" && h.text == "<meta>forecast</meta>")
        );
        for query in ["device-width", "color: red", "hidden"] {
            assert!(
                search(&folders, Some(&Query::new(query).unwrap()), 20, false)
                    .hits
                    .is_empty(),
                "{query}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
