//! The copy: writes the planned files into the new worktree's folder and nowhere else.
//!
//! Depends on: `safefs` (handle-relative opens), `names`. Called by: vw-core-api after `session/start` made the
//! folder.
//! Invariants:
//! - Every name is validated again here; a hand-built plan cannot reach `..`, `.git`, a drive or a stream.
//! - Source and destination go through [`safefs`]: no link is followed at any level (on Unix the open itself
//!   refuses it, with no gap between a check and the use), a created file never replaces anything that exists
//!   (`O_EXCL`), and permissions are set on the open handle.
//! - Bytes: at most the planned size is read; a file that grew is removed again and listed as changed.
//! - Unix modes: created private (0600), then set to exactly the source's permission bits (setuid, setgid and
//!   sticky are never carried), so a copy is never more open than its source. Windows: the file inherits the
//!   ACL of the worktree folder.
//! - A failure of one file never stops the others; a partly written file is removed.

use std::io::{Read, Write};
use std::path::Path;

use super::names::validate_relative;
use super::safefs::{create_dest, open_source};
use super::{IncludeReport, MAX_BYTES, MAX_FILES, SkipReason};

/// Copies the files of `plan` from `root` (the main checkout) into `dest` (the new worktree's folder).
///
/// `plan` normally comes from [`super::plan`]; it is checked again. Returns the report of what was copied and
/// what was skipped. Never panics and never fails: a problem with the folder itself is `stopped`.
#[must_use]
pub fn apply(root: &Path, dest: &Path, plan: &IncludeReport) -> IncludeReport {
    let mut done = IncludeReport {
        found: plan.found,
        skipped: plan.skipped.clone(),
        skipped_total: plan.skipped_total,
        stopped: plan.stopped.clone(),
        ..IncludeReport::default()
    };
    if plan.files.is_empty() {
        return done;
    }
    if plan.files.len() > MAX_FILES
        || plan.files.iter().map(|f| u64::from(f.size)).sum::<u64>() > MAX_BYTES
    {
        done.stopped = Some("The plan is over the size limits; nothing was copied.".to_owned());
        return done;
    }
    let (Ok(root), Ok(dest)) = (root.canonicalize(), dest.canonicalize()) else {
        done.stopped =
            Some("The new copy's folder was not found, so no files were copied.".to_owned());
        return done;
    };
    if dest == root || (dest.starts_with(&root) && !dest.join(".git").exists()) {
        done.stopped = Some(
            "The new copy's folder is not a separate working tree; nothing was copied.".to_owned(),
        );
        return done;
    }
    for file in &plan.files {
        match copy_one(&root, &dest, &file.path, u64::from(file.size)) {
            Ok(size) => done.add(file.path.clone(), size),
            Err(reason) => done.skip(file.path.clone(), reason),
        }
    }
    done
}

/// One file; `Ok` = bytes written.
fn copy_one(root: &Path, dest: &Path, rel: &str, planned: u64) -> Result<u64, SkipReason> {
    let comps = validate_relative(rel)?;
    let (mut source, meta) = open_source(root, &comps)?;
    if meta.len() > planned || meta.len() > MAX_BYTES {
        return Err(SkipReason::Changed);
    }
    let mut created = create_dest(dest, &comps)?;
    let finished = write_bounded(&mut source, &mut created.file, planned)
        .and_then(|n| set_mode(&created.file, &meta).map(|()| n));
    if finished.is_err() {
        created.remove();
    }
    finished
}

/// Copies at most `planned` bytes; one more byte available means the file grew.
fn write_bounded(
    source: &mut impl Read,
    out: &mut impl Write,
    planned: u64,
) -> Result<u64, SkipReason> {
    let mut bounded = source.take(planned + 1);
    let n = std::io::copy(&mut bounded, out).map_err(|_| SkipReason::Unreadable)?;
    out.flush().map_err(|_| SkipReason::Unreadable)?;
    if n > planned {
        Err(SkipReason::Changed)
    } else {
        Ok(n)
    }
}

/// Sets the source's permission bits on the open copy (Unix, by handle).
#[cfg(unix)]
fn set_mode(file: &std::fs::File, source: &std::fs::Metadata) -> Result<(), SkipReason> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let bits = source.mode() & 0o777;
    file.set_permissions(std::fs::Permissions::from_mode(bits))
        .map_err(|_| SkipReason::Unreadable)
}

/// Nothing to set elsewhere: the copy inherits the folder's ACL .
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)] // same signature as the Unix function, so the caller has one code path
fn set_mode(_file: &std::fs::File, _source: &std::fs::Metadata) -> Result<(), SkipReason> {
    Ok(())
}
