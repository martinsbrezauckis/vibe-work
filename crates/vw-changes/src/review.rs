//! Keep and revert: per hunk, per file, all. Keep moves the baseline (the user accepts the change); revert
//! changes the working tree back to the baseline (the user rejects it).
//!
//! Depends on: `baseline`, `diff`, `objects`, `linediff`, `files`, `model`, `error`. Called by: vw-core-api.
//! Invariants:
//! - Every action first refreshes and checks that the target is exactly what the user saw (file blob id,
//!   hunk id, or the whole diff's fingerprint). Anything else is `Stale` and nothing is written.
//! - Git never touches the working tree: a hunk is spliced in Rust from the two
//!   blobs the diff was made of, and files are written or deleted by `files`. Keep writes only our baseline
//!   index and objects (`hash-object --no-filters`, `update-index --index-info`).
//! - A revert deletes a file only when the request names it in `delete_paths`; for "all" the named set must
//!   equal the set it would delete. A folder is never deleted.

use std::collections::BTreeSet;

use crate::baseline::Baseline;
use crate::diff::{compute, refresh};
use crate::error::ChangesError;
use crate::files;
use crate::linediff::{splice, split_lines};
use crate::model::{ContentKind, Diff, FileChange, FileStatus, Hunk, Target};
use crate::objects::{Entry, hash_bytes, read_blobs, update_index};

/// Accepts the target: the baseline moves so it no longer shows as a change. Returns the fresh diff.
///
/// # Errors
/// `Stale`, `Refused` (a hunk of a non-text file), git failures.
pub fn keep(baseline: &mut Baseline, target: &Target) -> Result<Diff, ChangesError> {
    // Every action reads and hashes every file again: it never relies on the stat cache.
    let (fresh, _current) = compute(baseline, false)?;
    let repo = baseline.repo().clone();
    let base_index = baseline.dir().base_index();
    match target {
        Target::All { fingerprint } => {
            if *fingerprint != fresh.fingerprint {
                return Err(ChangesError::Stale { path: "*".into() });
            }
            // Review integrity: "keep all" accepts only what the user was shown line by line. Files too
            // large to show, binary files, large untracked files and submodules stay un-kept until they are
            // kept one by one.
            let mut removals = Vec::new();
            let mut upserts = Vec::new();
            for file in fresh.files.iter().filter(|f| shown_in_full(f)) {
                base_update(file, &mut removals, &mut upserts);
            }
            update_index(&repo, &base_index, &removals, &upserts)?;
        }
        Target::File { path, blob } => {
            let file = checked_file(&fresh, path, blob.as_deref())?;
            if file.content == ContentKind::LargeUntracked {
                let mut large = baseline.large().to_vec();
                large.push(crate::worktree::LargeFile {
                    path: file.path.clone(),
                    size: file.size.unwrap_or(0),
                    modified_ms: modified_ms(baseline, &file.raw_path),
                    raw: file.raw_path.clone(),
                });
                baseline.set_large(large);
            } else {
                let mut removals = Vec::new();
                let mut upserts = Vec::new();
                base_update(file, &mut removals, &mut upserts);
                update_index(&repo, &base_index, &removals, &upserts)?;
            }
        }
        Target::Hunk {
            path,
            blob,
            hunk_id,
        } => {
            let file = checked_file(&fresh, path, blob.as_deref())?;
            let hunk = hunk_of(file, hunk_id)?;
            let (base_bytes, work_bytes) = both_sides(baseline, file)?;
            let (old, new) = (split_lines(&base_bytes), split_lines(&work_bytes));
            let (old_at, new_at) = (hunk.old_at as usize, hunk.new_at as usize);
            let moved = splice(
                &old,
                old_at,
                hunk.old_lines as usize,
                &new[new_at..new_at + hunk.new_lines as usize],
            );
            let id = hash_bytes(&repo, &base_index, &moved)?;
            let base_path = file
                .raw_old_path
                .clone()
                .unwrap_or_else(|| file.raw_path.clone());
            let mode = file.base_mode.clone().unwrap_or_else(|| "100644".into());
            update_index(&repo, &base_index, &[], &[(base_path, Entry { mode, id })])?;
        }
    }
    baseline.commit_index()?;
    refresh(baseline)
}

/// Rejects the target: the working tree goes back to the baseline for it. Returns the fresh diff.
///
/// # Errors
/// `Stale`, `Refused`, `ConfirmDeletes` (the request did not name every file it would delete), git and
/// filesystem failures.
pub fn revert(
    baseline: &Baseline,
    target: &Target,
    delete_paths: &[String],
) -> Result<Diff, ChangesError> {
    // Every action reads and hashes every file again: it never relies on the stat cache.
    let (fresh, _) = compute(baseline, false)?;
    let named: BTreeSet<&str> = delete_paths.iter().map(String::as_str).collect();
    let root = baseline.repo().root.clone();
    match target {
        Target::All { fingerprint } => {
            if *fingerprint != fresh.fingerprint {
                return Err(ChangesError::Stale { path: "*".into() });
            }
            let chosen: Vec<&FileChange> = fresh
                .files
                .iter()
                .filter(|f| f.content != ContentKind::Submodule)
                .collect();
            let deletes: BTreeSet<&str> =
                chosen.iter().filter_map(|f| deleted_by_revert(f)).collect();
            if deletes != named {
                return Err(ChangesError::ConfirmDeletes {
                    paths: deletes.iter().map(|p| (*p).to_owned()).collect(),
                });
            }
            // Refusals come before the first write: a revert never stops half done for them.
            for file in &chosen {
                refuse_folder_case(file)?;
            }
            for file in &chosen {
                restore(baseline, file)?;
            }
            for file in chosen.iter().filter(|f| deleted_by_revert(f).is_some()) {
                files::delete(&root, &file.raw_path)?;
            }
        }
        Target::File { path, blob } => {
            let file = checked_file(&fresh, path, blob.as_deref())?;
            if file.content == ContentKind::Submodule {
                return Err(ChangesError::Refused(
                    "a submodule is reviewed in its own repository".into(),
                ));
            }
            if let Some(gone) = deleted_by_revert(file)
                && !named.contains(gone)
            {
                return Err(ChangesError::ConfirmDeletes {
                    paths: vec![gone.to_owned()],
                });
            }
            refuse_folder_case(file)?;
            restore(baseline, file)?;
            if deleted_by_revert(file).is_some() {
                files::delete(&root, &file.raw_path)?;
            }
        }
        Target::Hunk {
            path,
            blob,
            hunk_id,
        } => {
            let file = checked_file(&fresh, path, blob.as_deref())?;
            let hunk = hunk_of(file, hunk_id)?;
            let (base_bytes, work_bytes) = both_sides(baseline, file)?;
            let (old, new) = (split_lines(&base_bytes), split_lines(&work_bytes));
            let (old_at, new_at) = (hunk.old_at as usize, hunk.new_at as usize);
            let back = splice(
                &new,
                new_at,
                hunk.new_lines as usize,
                &old[old_at..old_at + hunk.old_lines as usize],
            );
            let mode = file.mode.clone().unwrap_or_else(|| "100644".into());
            files::write(&root, &file.raw_path, &back, &mode)?;
        }
    }
    refresh(baseline)
}

/// Writes the baseline version of `file` back (nothing for a file the baseline does not have). A case-only
/// rename on a case-insensitive file system is first renamed back: there, writing the old name would write
/// the same file, and deleting the new name afterwards would delete the user's only copy.
fn restore(baseline: &Baseline, file: &FileChange) -> Result<(), ChangesError> {
    if file.case_only
        && let Some(old) = &file.raw_old_path
    {
        files::rename_back(&baseline.repo().root, &file.raw_path, old)?;
    }
    let (Some(id), Some(mode)) = (&file.base_id, &file.base_mode) else {
        return Ok(());
    };
    if file.content == ContentKind::LargeUntracked {
        return Ok(());
    }
    let blobs = read_blobs(
        baseline.repo(),
        &baseline.dir().current_index(),
        &[id.as_str()],
    )?;
    let bytes = blobs
        .get(id)
        .ok_or_else(|| ChangesError::BaselineLost(format!("blob {id} is missing")))?;
    let path = file.raw_old_path.as_deref().unwrap_or(&file.raw_path);
    files::write(&baseline.repo().root, path, bytes, mode)
}

/// The baseline and current bytes of a text file (from the blobs the diff was made of).
fn both_sides(baseline: &Baseline, file: &FileChange) -> Result<(Vec<u8>, Vec<u8>), ChangesError> {
    let (Some(base), Some(work)) = (&file.base_id, &file.blob) else {
        return Err(ChangesError::Refused(format!(
            "{} has no separate hunks",
            file.path
        )));
    };
    let mut blobs = read_blobs(
        baseline.repo(),
        &baseline.dir().current_index(),
        &[base.as_str(), work.as_str()],
    )?;
    let base_bytes = blobs
        .remove(base)
        .ok_or_else(|| ChangesError::BaselineLost(format!("blob {base} is missing")))?;
    let work_bytes = if work == base {
        base_bytes.clone()
    } else {
        blobs.remove(work).ok_or_else(|| ChangesError::Stale {
            path: file.path.clone(),
        })?
    };
    Ok((base_bytes, work_bytes))
}

/// The file at `path` in `fresh`, if its blob is the one the user saw.
fn checked_file<'a>(
    fresh: &'a Diff,
    path: &str,
    blob: Option<&str>,
) -> Result<&'a FileChange, ChangesError> {
    let file = fresh.file(path).ok_or_else(|| ChangesError::Stale {
        path: path.to_owned(),
    })?;
    if file.blob.as_deref() != blob {
        return Err(ChangesError::Stale {
            path: path.to_owned(),
        });
    }
    Ok(file)
}

/// The hunk with `id`; hunk actions need a text file (not a symlink) that exists on both sides.
fn hunk_of<'a>(file: &'a FileChange, id: &str) -> Result<&'a Hunk, ChangesError> {
    let symlink =
        file.mode.as_deref() == Some("120000") || file.base_mode.as_deref() == Some("120000");
    if file.content != ContentKind::Text
        || symlink
        || !matches!(file.status, FileStatus::Modified | FileStatus::Renamed)
    {
        return Err(ChangesError::Refused(format!(
            "{} has no separate hunks: keep or revert the whole file",
            file.path
        )));
    }
    file.hunks
        .iter()
        .find(|h| h.id == id)
        .ok_or_else(|| ChangesError::Stale {
            path: file.path.clone(),
        })
}

/// The path a revert deletes: an added file, or the new path of a rename (not of a case-only rename on a
/// case-insensitive file system: that is renamed back).
fn deleted_by_revert(file: &FileChange) -> Option<&str> {
    match file.status {
        FileStatus::Added => Some(file.path.as_str()),
        FileStatus::Renamed if !file.case_only => Some(file.path.as_str()),
        _ => None,
    }
}

/// Refuses to revert a case-only rename of a folder (`Docs/a.txt` -> `docs/a.txt`): renaming the file back
/// cannot change its folder's name, and renaming the folder would touch every file in it.
fn refuse_folder_case(file: &FileChange) -> Result<(), ChangesError> {
    let parent = |p: &[u8]| p.iter().rposition(|b| *b == b'/').map(|i| p[..i].to_vec());
    match &file.raw_old_path {
        Some(old) if file.case_only && parent(old) != parent(&file.raw_path) => {
            Err(ChangesError::Refused(format!(
                "{} -> {}: a folder's name changed only in case; rename the folder back in your file manager (the Changes tab never renames folders)",
                crate::paths::display(old),
                file.path
            )))
        }
        _ => Ok(()),
    }
}

/// True when the user saw this file's change in full (a text diff with its hunks, or a mode-only change).
fn shown_in_full(file: &FileChange) -> bool {
    file.content == ContentKind::Text
}

/// The baseline index changes that accept `file` as it is now.
fn base_update(
    file: &FileChange,
    removals: &mut Vec<Vec<u8>>,
    upserts: &mut Vec<(Vec<u8>, Entry)>,
) {
    if let Some(old) = &file.raw_old_path {
        removals.push(old.clone());
    }
    match (&file.blob, &file.mode) {
        (Some(id), Some(mode)) => upserts.push((
            file.raw_path.clone(),
            Entry {
                mode: mode.clone(),
                id: id.clone(),
            },
        )),
        _ => removals.push(file.raw_path.clone()),
    }
}

fn modified_ms(baseline: &Baseline, path: &[u8]) -> u64 {
    crate::paths::inside_bytes(&baseline.repo().root, path)
        .ok()
        .and_then(|full| std::fs::symlink_metadata(full).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
