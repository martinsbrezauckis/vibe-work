//! Reading a handbook folder: every page file (HTML and Markdown) read for validation, every other file named (for
//! HB-I03), and the folder of a project resolved (`docs/handbook/`, else an existing pre-1.0 `docs/pages/`, §2).
//!
//! Depends on: `page`, `lite`, `paths`, `handbook::format`. Called by: `handbook::validate`, `hub`,
//! `handbook::overview`, `settings`, tests.
//! Invariants:
//! - Reads at most [`MAX_FILES`] files, [`MAX_DEPTH`] folders deep; hidden files and folders and symlinked folders
//!   are skipped; a symlinked file is read only when it resolves inside the folder; at most `html::MAX_INPUT` bytes
//!   of a page are read.
//! - Never writes, creates or renames anything.

use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::page::{PageScan, scan_html};
use crate::paths::{PagePath, is_page_name, segment_allowed};

/// Most files read for one folder.
pub const MAX_FILES: usize = 2000;
/// Deepest folder level read.
pub const MAX_DEPTH: usize = 6;

/// A page file found in the folder.
#[derive(Clone, Debug)]
pub struct Found {
    /// Its path inside the handbook.
    pub path: PagePath,
    /// What it says.
    pub scan: PageScan,
    /// Modification time, ms since the Unix epoch.
    pub modified_ms: u64,
}

/// What a folder holds.
#[derive(Clone, Debug, Default)]
pub struct FolderScan {
    /// Pages, sorted by path.
    pub pages: Vec<Found>,
    /// Other files (paths inside the handbook), sorted.
    pub others: Vec<String>,
    /// More files exist than were read.
    pub truncated: bool,
}

impl FolderScan {
    /// The page at `path`, if read.
    #[must_use]
    pub fn page(&self, path: &str) -> Option<&Found> {
        self.pages
            .binary_search_by(|f| f.path.as_str().cmp(path))
            .ok()
            .and_then(|i| self.pages.get(i))
    }
}

/// Modification time of `meta` in ms since the epoch (0 when unknown).
#[must_use]
pub fn modified_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Reads at most `limit` bytes of `file` (lossy UTF-8) and its full length.
///
/// # Errors
/// The I/O error.
pub fn read_prefix(file: &Path, limit: usize) -> std::io::Result<(String, u64)> {
    let handle = std::fs::File::open(file)?;
    let len = handle.metadata()?.len();
    let mut bytes = Vec::new();
    handle
        .take(u64::try_from(limit).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), len))
}

/// Reads one page file for validation (HTML or Markdown by `rel`'s extension). `rel` is the page's path relative to
/// the handbook folder (`/` separators): a Markdown page's folder can give it a kind (§7). `None` when it cannot be
/// read.
#[must_use]
pub fn read_page(file: &Path, rel: &str) -> Option<PageScan> {
    let markdown = rel.to_ascii_lowercase().ends_with(".md");
    // One byte more than the reader keeps, so a longer page is known to be truncated.
    let (text, len) = read_prefix(file, crate::html::MAX_INPUT + 1).ok()?;
    let mut scan = if markdown {
        crate::lite::scan_markdown_at(&text, rel)
    } else {
        scan_html(&text)
    };
    scan.detail.truncated |= len > crate::html::MAX_INPUT as u64;
    Some(scan)
}

/// True when `file` resolves (symlinks followed) to a place inside `root`.
#[must_use]
pub fn inside(root: &Path, file: &Path) -> bool {
    match (root.canonicalize(), file.canonicalize()) {
        (Ok(root), Ok(file)) => file.starts_with(root),
        _ => false,
    }
}

/// A fingerprint of the folder's listing: every file's path, length and modification time (the same walk and
/// limits as [`scan`], `stat` only, no reads). Equal fingerprints mean a cached validation is still current.
#[must_use]
pub fn fingerprint(root: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    let mut count = 0usize;
    let mut folders = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut names: Vec<_> = entries.filter_map(Result::ok).collect();
        names.sort_by_key(std::fs::DirEntry::file_name);
        for entry in names {
            let name = entry.file_name();
            if !name.to_str().is_some_and(segment_allowed) || count >= MAX_FILES {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                if depth + 1 < MAX_DEPTH {
                    folders.push((entry.path(), depth + 1));
                }
                continue;
            }
            count += 1;
            let followed = std::fs::metadata(entry.path()).unwrap_or(meta);
            entry.path().hash(&mut hasher);
            followed.len().hash(&mut hasher);
            followed
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .hash(&mut hasher);
        }
    }
    count.hash(&mut hasher);
    hasher.finish()
}

/// Every file under `root` (see the module invariants).
#[must_use]
pub fn scan(root: &Path) -> FolderScan {
    let mut out = FolderScan::default();
    let mut seen = 0usize;
    let mut folders = vec![(root.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, prefix, depth)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut names: Vec<_> = entries.filter_map(Result::ok).collect();
        names.sort_by_key(std::fs::DirEntry::file_name);
        for entry in names {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !segment_allowed(&name) {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if meta.is_dir() {
                if depth + 1 < MAX_DEPTH {
                    folders.push((entry.path(), rel, depth + 1));
                }
                continue;
            }
            if !(meta.is_file() || meta.file_type().is_symlink()) {
                continue;
            }
            if seen >= MAX_FILES {
                out.truncated = true;
                break;
            }
            seen += 1;
            // A symlinked file is read only when it stays inside the folder (canonical check).
            if meta.file_type().is_symlink() && !inside(root, &entry.path()) {
                continue;
            }
            let page = is_page_name(&name)
                .then(|| PagePath::parse(&rel).ok())
                .flatten();
            let Some(path) = page else {
                out.others.push(rel);
                continue;
            };
            if let Some(scan) = read_page(&entry.path(), path.as_str()) {
                let meta = std::fs::metadata(entry.path()).unwrap_or(meta);
                out.pages.push(Found {
                    path,
                    scan,
                    modified_ms: modified_ms(&meta),
                });
            }
        }
    }
    out.pages.sort_by(|a, b| a.path.cmp(&b.path));
    out.others.sort();
    out
}
