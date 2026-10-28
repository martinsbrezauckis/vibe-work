//! Bounded directory, file-name and text readers for Files. Uses the hardened git runner; no whole-tree read on expand.
use crate::error::{CoreError, ErrorCode};
use crate::explorer_view::{ExplorerEntry, ExplorerHit, ExplorerResult};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use vw_changes::git::{Call, Index};

const SKIP: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "venv",
    ".venv",
    "__pycache__",
];
pub(crate) fn failed() -> CoreError {
    CoreError::new(
        ErrorCode::NotFound,
        "This folder could not be read. Files still work; try reading history again.",
    )
}
pub(crate) fn relative(path: &str) -> bool {
    path.len() <= 4096
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && !path.split(['/', '\\']).any(|p| p == ".git" || p == "..")
}
pub(crate) fn git(root: &Path, args: &[&str], cap: usize) -> Result<Vec<u8>, CoreError> {
    Call::new(root, Index::User)
        .args(args)
        .max_output(cap)
        .timeout(Duration::from_secs(3))
        .run()
        .map_err(|_| failed())
}
fn ignored(root: &Path, names: &[String]) -> Vec<String> {
    let input = names.join("\0") + "\0";
    let mut ignored: Vec<String> = Call::new(root, Index::User)
        .args(["check-ignore", "--no-index", "--stdin", "-z"])
        .stdin(input.as_bytes())
        .timeout(Duration::from_secs(2))
        .max_output(2 << 20)
        .output()
        .ok()
        .map(|o| {
            o.stdout
                .split(|b| *b == 0)
                .filter_map(|b| String::from_utf8(b.to_vec()).ok())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if !ignored.is_empty() {
        // Tracked paths stay visible even after an ignore pattern starts matching them.
        if let Ok(tracked) = git(
            root,
            &[
                "ls-files",
                "--cached",
                "--ignored",
                "--exclude-standard",
                "-z",
            ],
            4 << 20,
        ) {
            let tracked: Vec<_> = tracked
                .split(|b| *b == 0)
                .filter_map(|b| std::str::from_utf8(b).ok())
                .filter(|s| !s.is_empty())
                .collect();
            ignored.retain(|name| {
                !tracked
                    .iter()
                    .any(|path| *path == name || path.starts_with(&format!("{name}/")))
            });
        }
    }
    ignored
}
pub(crate) fn folder(
    root: &Path,
    path: &str,
    show_ignored: bool,
    offset: u32,
) -> Result<ExplorerResult, CoreError> {
    if !path.is_empty() && !relative(path) {
        return Err(failed());
    }
    let full = root.join(path);
    if vw_git::browser::safe_path(root, &full).is_none() {
        return Err(failed());
    }
    let mut entries = Vec::new();
    // Read at most one page plus its continuation. Each returned page is independently bounded.
    let mut iter = std::fs::read_dir(&full).map_err(|_| failed())?;
    let offset = offset.min(1_000_000) as usize;
    for item in iter.by_ref().skip(offset).take(5000) {
        let item = item.map_err(|_| failed())?;
        let kind = item.file_type().map_err(|_| failed())?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name == ".git" || kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
            continue;
        }
        let rel = if path.is_empty() {
            name
        } else {
            format!("{path}/{name}")
        };
        entries.push(ExplorerEntry {
            path: rel,
            directory: kind.is_dir(),
        });
    }
    let next = iter
        .next()
        .is_some()
        .then_some(u32::try_from(offset + 5000).unwrap_or(u32::MAX));
    let ignored = ignored(
        root,
        &entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>(),
    );
    if !show_ignored {
        entries.retain(|e| !ignored.contains(&e.path));
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
    });
    let status = None;
    Ok(ExplorerResult::Folder {
        entries,
        ignored,
        next,
        status,
    })
}
/// Root-relative file paths under `start` (root-relative; empty: the whole folder), bounded in count and time.
fn walk(root: &Path, start: &Path, cancel: Option<&AtomicBool>) -> (Vec<String>, bool) {
    let started = Instant::now();
    let mut todo = vec![start.to_path_buf()];
    let mut paths = Vec::new();
    let mut visited = 0;
    while let Some(rel) = todo.pop() {
        let Ok(items) = std::fs::read_dir(root.join(&rel)) else {
            continue;
        };
        for item in items.flatten() {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return (paths, true);
            }
            visited += 1;
            if visited > 100_000 || started.elapsed() > Duration::from_secs(3) {
                return (paths, true);
            }
            let name = item.file_name().to_string_lossy().into_owned();
            let Ok(kind) = item.file_type() else {
                continue;
            };
            let child = rel.join(&name);
            if kind.is_dir() && !name.starts_with('.') && !SKIP.contains(&name.as_str()) {
                todo.push(child);
            } else if kind.is_file() {
                paths.push(child.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    (paths, false)
}
pub(crate) fn index(root: &Path) -> ExplorerResult {
    let (mut paths, truncated) = match git(
        root,
        &["ls-files", "-co", "--exclude-standard", "-z"],
        32 << 20,
    ) {
        Ok(bytes) => {
            let mut names = bytes
                .split(|b| *b == 0)
                .filter(|b| !b.is_empty())
                .filter_map(|b| String::from_utf8(b.to_vec()).ok())
                .filter(|p| relative(p))
                .take(100_001)
                .collect::<Vec<_>>();
            let truncated = names.len() > 100_000;
            names.truncate(100_000);
            (names, truncated)
        }
        Err(_) => walk(root, Path::new(""), None),
    };
    paths.sort();
    paths.dedup();
    ExplorerResult::Index { paths, truncated }
}
/// Text search, inside the root-relative folder `inside` when given (checked like `folder`'s path: a path that
/// leaves the root or passes a symlink is refused).
pub(crate) fn search(
    root: &Path,
    query: &str,
    inside: Option<&str>,
    cancel: Option<&AtomicBool>,
) -> Result<ExplorerResult, CoreError> {
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Ok(ExplorerResult::Done {});
    }
    let inside = inside.filter(|p| !p.is_empty()).unwrap_or("");
    if !inside.is_empty()
        && (!relative(inside) || vw_git::browser::safe_path(root, &root.join(inside)).is_none())
    {
        return Err(failed());
    }
    if query.trim().is_empty() || query.len() > 512 {
        return Ok(ExplorerResult::Search {
            hits: Vec::new(),
            truncated: false,
        });
    }
    if let Ok(repo) = git(root, &["rev-parse", "--is-inside-work-tree"], 64)
        && repo.starts_with(b"true")
    {
        return git_search(root, query, inside, cancel);
    }
    let (paths, mut limited) = walk(root, Path::new(inside), cancel);
    let started = Instant::now();
    let mut hits = Vec::new();
    let mut bytes = 0;
    let needle = query.to_lowercase();
    for path in paths {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Ok(ExplorerResult::Done {});
        }
        if started.elapsed() > Duration::from_secs(3) || bytes > 32 << 20 {
            limited = true;
            break;
        }
        let full = root.join(&path);
        if vw_changes::has_symlinked_parent(root, &full) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&full) else {
            continue;
        };
        if !meta.is_file() || meta.len() > 1 << 20 {
            continue;
        }
        let Ok(content) = std::fs::read(&full) else {
            continue;
        };
        bytes += content.len();
        if content.contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&content);
        for (line, text) in text.lines().enumerate() {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Ok(ExplorerResult::Done {});
            }
            if text.to_lowercase().contains(&needle) {
                if hits.len() == 500 {
                    return Ok(ExplorerResult::Search {
                        hits,
                        truncated: true,
                    });
                }
                hits.push(ExplorerHit {
                    path: path.clone(),
                    line: u32::try_from(line + 1).unwrap_or(u32::MAX),
                    text: text.chars().take(300).collect(),
                });
            }
        }
    }
    Ok(ExplorerResult::Search {
        hits,
        truncated: limited,
    })
}

fn git_search(
    root: &Path,
    query: &str,
    inside: &str,
    cancel: Option<&AtomicBool>,
) -> Result<ExplorerResult, CoreError> {
    let token = AtomicBool::new(false);
    let cancel = cancel.unwrap_or(&token);
    // `:(literal)`: a folder name is a path, never a glob or pathspec magic.
    let pathspec = if inside.is_empty() {
        ".".to_owned()
    } else {
        format!(":(literal){inside}")
    };
    let output = Call::new(root, Index::User)
        .args([
            "grep",
            "-n",
            "-z",
            "-I",
            "--no-textconv",
            "--untracked",
            "--exclude-standard",
            "-F",
            "-i",
            "-e",
            query,
            "--",
            &pathspec,
        ])
        .cancel_when(cancel)
        .take_prefix()
        .max_output(1 << 20)
        .timeout(Duration::from_secs(3))
        .output();
    if cancel.load(Ordering::Relaxed) {
        return Ok(ExplorerResult::Done {});
    }
    let out = output.map_err(|_| failed())?;
    if !matches!(out.code, Some(0 | 1)) {
        return Err(failed());
    }
    let mut hits = Vec::new();
    let mut rest = out.stdout.as_slice();
    while let Some(name_end) = rest.iter().position(|b| *b == 0) {
        let path = String::from_utf8_lossy(&rest[..name_end]).into_owned();
        rest = &rest[name_end + 1..];
        let Some(line_end) = rest.iter().position(|b| *b == 0) else {
            break;
        };
        let line = String::from_utf8_lossy(&rest[..line_end])
            .parse::<u32>()
            .unwrap_or(1);
        rest = &rest[line_end + 1..];
        let Some(text_end) = rest.iter().position(|b| *b == b'\n') else {
            break;
        };
        let text = String::from_utf8_lossy(&rest[..text_end])
            .chars()
            .take(300)
            .collect();
        rest = &rest[text_end + 1..];
        if relative(&path) {
            hits.push(ExplorerHit { path, line, text });
        }
        if hits.len() > 500 {
            break;
        }
    }
    let truncated = hits.len() > 500 || out.stdout.len() >= 1 << 20;
    hits.truncate(500);
    Ok(ExplorerResult::Search { hits, truncated })
}
