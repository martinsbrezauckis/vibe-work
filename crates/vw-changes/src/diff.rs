//! The working tree against the baseline: which files changed, their hunks, and what is too big or binary
//! to show line by line. Git only compares trees (`diff-tree`, for rename detection) and hands out blobs
//! (`cat-file`); the line diff is ours (`linediff`), so no diff driver, textconv or filter runs.
//!
//! Depends on: `baseline`, `worktree`, `objects`, `linediff`, `parse`, `git`, `model`, `error`. Called by:
//! `review`, vw-core-api (every refresh).
//! Invariants:
//! - Cheap on a big repository: the stat cache skips unchanged files; trees are written and compared only
//!   when something changed.
//! - After a forced `git gc --prune=now` a refresh repairs itself once (baseline objects written back, the
//!   stat cache dropped so every changed file is stored again) instead of failing.

use std::collections::HashMap;

use crate::baseline::Baseline;
use crate::error::ChangesError;
use crate::git::{Call, Index};
use crate::linediff::{hunks, is_binary, split_lines};
use crate::model::{ContentKind, Diff, FileChange, FileStatus, Hunk};
use crate::objects::{Entries, read_blobs};
use crate::parse::{RawEntry, fnv_hex};
use crate::paths::display;

/// Text files with more changed lines than this are listed with counts and hunk count only.
pub const MAX_CHANGED_LINES: u32 = 5_000;
/// Files bigger than this (bytes, either side) are listed without lines (counts and hunk count still real).
pub const MAX_DIFF_BYTES: usize = 4 << 20;
/// Files bigger than this (bytes, either side) are not compared line by line at all (no counts).
pub const MAX_COMPARE_BYTES: usize = 32 << 20;

/// Reads the working tree and diffs it against the baseline.
///
/// # Errors
/// git failures; `BaselineLost` when the baseline cannot be repaired after a forced gc.
pub fn refresh(baseline: &Baseline) -> Result<Diff, ChangesError> {
    Ok(compute(baseline, true)?.0)
}

/// [`refresh`] plus the working tree's entries (keep all uses them). `trust` false reads and hashes every
/// file again (before every keep/revert: an action never relies on the stat cache).
pub(crate) fn compute(baseline: &Baseline, trust: bool) -> Result<(Diff, Entries), ChangesError> {
    match compute_once(baseline, trust) {
        Err(error) if error.is_missing_object() => {
            tracing::info!(%error, "changes: objects missing (gc?), repairing the baseline");
            baseline.ensure_objects()?;
            baseline.forget_stat_cache();
            // Still missing after the repair: the starting point cannot be trusted (fail closed).
            compute_once(baseline, false).map_err(|error| {
                if error.is_missing_object() {
                    ChangesError::BaselineLost(format!(
                        "objects are missing from the repository ({error})"
                    ))
                } else {
                    error
                }
            })
        }
        other => other,
    }
}

fn compute_once(baseline: &Baseline, trust: bool) -> Result<(Diff, Entries), ChangesError> {
    let base = baseline.entries();
    let scan = baseline.scan(&base, trust)?;
    let removals: Vec<Vec<u8>> = base
        .keys()
        .filter(|p| !scan.entries.contains_key(*p))
        .cloned()
        .collect();
    let upserts: Vec<(Vec<u8>, crate::objects::Entry)> = scan
        .entries
        .iter()
        .filter(|(p, e)| base.get(*p) != Some(*e))
        .map(|(p, e)| (p.clone(), e.clone()))
        .collect();
    // The change list comes from our private baseline entries and our own reading of the working tree,
    // never from tree objects in `.git/objects` (the agent can rewrite those; integrity check).
    let entries = changed_entries(&base, &removals, &upserts);
    let mut files: Vec<FileChange> = if entries.is_empty() {
        Vec::new()
    } else {
        describe(baseline, &entries)?
    };
    // Case-only renames are one file on a case-insensitive file system.
    if files.iter().any(|f| f.status == FileStatus::Renamed)
        && crate::worktree::case_insensitive(&baseline.repo().root)
    {
        for file in &mut files {
            file.case_only = file.status == FileStatus::Renamed
                && file.raw_old_path.as_deref().is_some_and(|old| {
                    String::from_utf8_lossy(old).to_lowercase()
                        == String::from_utf8_lossy(&file.raw_path).to_lowercase()
                });
        }
    }
    for large in &scan.large {
        let known = baseline.large().iter().any(|b| {
            b.path == large.path && b.size == large.size && b.modified_ms == large.modified_ms
        });
        if !known {
            files.push(FileChange {
                path: large.path.clone(),
                old_path: None,
                status: FileStatus::Added,
                content: ContentKind::LargeUntracked,
                added: 0,
                removed: 0,
                old_mode: None,
                new_mode: None,
                blob: None,
                size: Some(large.size),
                hunks: Vec::new(),
                hunk_count: 0,
                turn: None,
                case_only: false,
                raw_path: large.raw.clone(),
                raw_old_path: None,
                base_id: None,
                base_mode: None,
                mode: None,
            });
        }
    }
    let fingerprint = fingerprint(&files);
    Ok((
        Diff {
            files,
            fingerprint,
            notes: scan.notes,
        },
        scan.entries,
    ))
}

/// The changed paths as diff records, renames paired by identical content (a moved file with edits shows as
/// a deletion plus a new file: both visible). Sorted by path.
fn changed_entries(
    base: &Entries,
    removals: &[Vec<u8>],
    upserts: &[(Vec<u8>, crate::objects::Entry)],
) -> Vec<RawEntry> {
    let zero_id = |like: &str| "0".repeat(like.len());
    let mut added: Vec<RawEntry> = Vec::new();
    let mut out: Vec<RawEntry> = Vec::new();
    for (path, entry) in upserts {
        match base.get(path) {
            Some(old) => {
                // A regular file stays one kind across an executable-bit change; symlink and submodule are
                // other kinds (a change between kinds is `T`).
                fn kind(mode: &str) -> &str {
                    if mode.starts_with("100") {
                        "file"
                    } else {
                        mode
                    }
                }
                out.push(RawEntry {
                    old_mode: old.mode.clone(),
                    new_mode: entry.mode.clone(),
                    old_id: old.id.clone(),
                    new_id: entry.id.clone(),
                    status: if kind(&old.mode) == kind(&entry.mode) {
                        'M'
                    } else {
                        'T'
                    },
                    score: 0,
                    old_path: path.clone(),
                    path: path.clone(),
                });
            }
            None => added.push(RawEntry {
                old_mode: "000000".into(),
                new_mode: entry.mode.clone(),
                old_id: zero_id(&entry.id),
                new_id: entry.id.clone(),
                status: 'A',
                score: 0,
                old_path: path.clone(),
                path: path.clone(),
            }),
        }
    }
    let mut deleted: Vec<RawEntry> = removals
        .iter()
        .filter_map(|path| {
            let old = base.get(path)?;
            Some(RawEntry {
                old_mode: old.mode.clone(),
                new_mode: "000000".into(),
                old_id: old.id.clone(),
                new_id: zero_id(&old.id),
                status: 'D',
                score: 0,
                old_path: path.clone(),
                path: path.clone(),
            })
        })
        .collect();
    for add in added {
        let pair = deleted
            .iter()
            .position(|d| d.old_id == add.new_id && d.old_mode == add.new_mode);
        match pair {
            Some(at) => {
                let gone = deleted.remove(at);
                out.push(RawEntry {
                    old_mode: gone.old_mode,
                    new_mode: add.new_mode,
                    old_id: gone.old_id,
                    new_id: add.new_id,
                    status: 'R',
                    score: 100,
                    old_path: gone.old_path,
                    path: add.path,
                });
            }
            None => out.push(add),
        }
    }
    out.extend(deleted);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn zero(id: &str) -> bool {
    id.bytes().all(|b| b == b'0')
}

/// File changes with their hunks, from `diff-tree` records and the two blobs of each file. Blobs over
/// [`MAX_COMPARE_BYTES`] are not read (their files are listed as too large, not compared); the others are
/// read in batches that stay under the output cap.
fn describe(baseline: &Baseline, entries: &[RawEntry]) -> Result<Vec<FileChange>, ChangesError> {
    let repo = baseline.repo();
    let index = baseline.dir().current_index();
    let mut wanted: Vec<&str> = Vec::new();
    for e in entries {
        for (mode, id) in [(&e.old_mode, &e.old_id), (&e.new_mode, &e.new_id)] {
            if mode != "160000" && !zero(id) {
                wanted.push(id);
            }
        }
    }
    let sizes = blob_sizes(repo, &index, &wanted)?;
    let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
    let mut batch: Vec<&str> = Vec::new();
    let mut batch_bytes = 0_usize;
    for id in wanted {
        let size = sizes.get(id).copied().unwrap_or(0);
        if size > MAX_COMPARE_BYTES || blobs.contains_key(id) {
            continue;
        }
        if batch_bytes + size > crate::git::OUTPUT_CAP / 2 && !batch.is_empty() {
            blobs.extend(read_blobs(repo, &index, &batch)?);
            batch.clear();
            batch_bytes = 0;
        }
        batch.push(id);
        batch_bytes += size;
    }
    if !batch.is_empty() {
        blobs.extend(read_blobs(repo, &index, &batch)?);
    }
    let empty = Vec::new();
    Ok(entries
        .iter()
        .map(|e| {
            let side = |id: &str| -> Option<&[u8]> {
                if zero(id) {
                    Some(empty.as_slice())
                } else {
                    blobs.get(id).map(Vec::as_slice)
                }
            };
            file_of(e, side(&e.old_id), side(&e.new_id))
        })
        .collect())
}

/// Sizes of blobs (`cat-file --batch-check`, which reads no content and runs no filter).
fn blob_sizes(
    repo: &crate::repo::Repo,
    index: &std::path::Path,
    ids: &[&str],
) -> Result<HashMap<String, usize>, ChangesError> {
    let mut sizes = HashMap::new();
    if ids.is_empty() {
        return Ok(sizes);
    }
    let mut request = Vec::new();
    for id in ids {
        request.extend_from_slice(id.as_bytes());
        request.push(b'\n');
    }
    let out = Call::new(&repo.root, Index::Ours(index))
        .args(["cat-file", "--batch-check"])
        .stdin(&request)
        .run()
        .map_err(|e| {
            if e.is_missing_object() {
                e
            } else {
                ChangesError::BaselineLost(format!(
                    "an object in the repository could not be read ({e})"
                ))
            }
        })?;
    for line in String::from_utf8_lossy(&out).lines() {
        let parts: Vec<&str> = line.split(' ').collect();
        if parts.len() == 3 {
            sizes.insert(parts[0].to_owned(), parts[2].parse().unwrap_or(usize::MAX));
        } else if parts.get(1) == Some(&"missing") {
            return Err(ChangesError::Git {
                command: "cat-file".into(),
                message: format!("missing blob {}", parts[0]),
            });
        }
    }
    Ok(sizes)
}

fn file_of(entry: &RawEntry, old: Option<&[u8]>, new: Option<&[u8]>) -> FileChange {
    let status = match entry.status {
        'A' | 'C' => FileStatus::Added,
        'D' => FileStatus::Deleted,
        'R' => FileStatus::Renamed,
        'T' => FileStatus::TypeChanged,
        _ => FileStatus::Modified,
    };
    let absent = |mode: &str| mode == "000000";
    let mode_change =
        !absent(&entry.old_mode) && !absent(&entry.new_mode) && entry.old_mode != entry.new_mode;
    let mut file = FileChange {
        path: display(&entry.path),
        old_path: (entry.status == 'R').then(|| display(&entry.old_path)),
        status,
        content: ContentKind::Text,
        added: 0,
        removed: 0,
        old_mode: mode_change.then(|| entry.old_mode.clone()),
        new_mode: mode_change.then(|| entry.new_mode.clone()),
        blob: (status != FileStatus::Deleted).then(|| entry.new_id.clone()),
        size: None,
        hunks: Vec::new(),
        hunk_count: 0,
        turn: None,
        case_only: false,
        raw_path: entry.path.clone(),
        raw_old_path: (entry.status == 'R').then(|| entry.old_path.clone()),
        base_id: (!zero(&entry.old_id)).then(|| entry.old_id.clone()),
        base_mode: (!absent(&entry.old_mode)).then(|| entry.old_mode.clone()),
        mode: (!absent(&entry.new_mode)).then(|| entry.new_mode.clone()),
    };
    if entry.old_mode == "160000" || entry.new_mode == "160000" {
        file.content = ContentKind::Submodule;
        return file;
    }
    // Not read (bigger than the comparison limit): listed as too large, no counts, never kept by "keep all".
    let (Some(old), Some(new)) = (old, new) else {
        file.content = ContentKind::TooLarge;
        return file;
    };
    if is_binary(old) || is_binary(new) {
        file.content = ContentKind::Binary;
        return file;
    }
    let (old_lines, new_lines) = (split_lines(old), split_lines(new));
    let found = hunks(&old_lines, &new_lines);
    let count = |kind| {
        found
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == kind)
            .count()
    };
    let to_u32 = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    file.added = to_u32(count(crate::model::LineKind::Add));
    file.removed = to_u32(count(crate::model::LineKind::Del));
    file.hunk_count = to_u32(found.len());
    // Review integrity: a capped file keeps its real counts and hunk count, is shown as "too large to show
    // here", and is never accepted by "keep all" (`review::keep`).
    if file.added.saturating_add(file.removed) > MAX_CHANGED_LINES
        || old.len() > MAX_DIFF_BYTES
        || new.len() > MAX_DIFF_BYTES
    {
        file.content = ContentKind::TooLarge;
        return file;
    }
    let path = entry.path.clone();
    file.hunks = found
        .into_iter()
        .map(|h| {
            let old_part: Vec<u8> = old_lines[h.old_at..h.old_at + h.old_len].concat();
            let new_part: Vec<u8> = new_lines[h.new_at..h.new_at + h.new_len].concat();
            let position = format!("{}:{}", h.old_at, h.new_at);
            Hunk {
                id: fnv_hex(&[
                    path.as_slice(),
                    position.as_bytes(),
                    old_part.as_slice(),
                    new_part.as_slice(),
                ]),
                old_start: to_u32(if h.old_len == 0 {
                    h.old_at
                } else {
                    h.old_at + 1
                }),
                old_lines: to_u32(h.old_len),
                new_start: to_u32(if h.new_len == 0 {
                    h.new_at
                } else {
                    h.new_at + 1
                }),
                new_lines: to_u32(h.new_len),
                section: String::new(),
                lines: h.lines,
                old_at: to_u32(h.old_at),
                new_at: to_u32(h.new_at),
            }
        })
        .collect();
    file
}

/// A hash over every file's path, status, content kind and blob id.
fn fingerprint(files: &[FileChange]) -> String {
    let parts: Vec<Vec<u8>> = files
        .iter()
        .map(|f| {
            format!(
                "{}\0{:?}\0{:?}\0{}\0{}",
                f.path,
                f.status,
                f.content,
                f.blob.as_deref().unwrap_or(""),
                f.size.unwrap_or(0)
            )
            .into_bytes()
        })
        .collect();
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    fnv_hex(&refs)
}
