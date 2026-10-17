//! Durable intent for an allowed own-branch rename; records follow only a verified rename in that copy.
//! Uses hardened Git reads and the records' atomic writer. Failed or unexecuted commands keep the old record.

use super::{Record, Records, WorktreeError, lock_records};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use vw_changes::git::{Call, Index};

#[derive(Serialize, Deserialize)]
struct Rename {
    from: String,
    to: String,
    copy: PathBuf,
    git_dir: PathBuf,
}

fn read(root: &Path, args: &[&str]) -> Option<String> {
    let out = Call::new(root, Index::User)
        .args(args)
        .timeout(Duration::from_secs(5))
        .output()
        .ok()?;
    out.ok()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

impl Records {
    /// Remembers an allowed rename before the shell executes it. The branch changes only once Git confirms it.
    ///
    /// # Errors
    /// Missing or changed copy identity, invalid destination, or a record/intent that cannot be written.
    pub fn note_rename(
        &self,
        session_id: &str,
        to: &str,
        copy: &Path,
    ) -> Result<(), WorktreeError> {
        let _guard = lock_records();
        let record = self
            .get_unlocked(session_id)
            .ok_or_else(|| WorktreeError::Stale("the copy record is missing".into()))?;
        let copy = copy
            .canonicalize()
            .map_err(|e| WorktreeError::io("resolve copy", copy, e))?;
        let git_dir = read(&copy, &["rev-parse", "--absolute-git-dir"])
            .and_then(|s| PathBuf::from(s).canonicalize().ok())
            .ok_or_else(|| WorktreeError::Stale("the copy's Git directory is missing".into()))?;
        if read(&copy, &["symbolic-ref", "-q", "--short", "HEAD"]).as_deref()
            != Some(&record.branch)
            || read(&copy, &["check-ref-format", &format!("refs/heads/{to}")]).is_none()
        {
            return Err(WorktreeError::Stale(
                "the copy branch or rename destination changed".into(),
            ));
        }
        let dir = self.dir.join("renames");
        vw_platform::ensure_private_dir(&dir).map_err(|e| {
            WorktreeError::io(
                "prepare rename folder",
                &dir,
                std::io::Error::other(e.to_string()),
            )
        })?;
        let path = dir.join(format!("{session_id}.json"));
        let bytes = serde_json::to_vec(&Rename {
            from: record.branch,
            to: to.to_owned(),
            copy,
            git_dir,
        })
        .map_err(|e| WorktreeError::io("encode rename", &path, std::io::Error::other(e)))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes).map_err(|e| WorktreeError::io("write rename", &tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| WorktreeError::io("replace rename", &path, e))
    }

    pub(super) fn follow_rename(&self, mut record: Record) -> Result<Record, WorktreeError> {
        let path = self
            .dir
            .join("renames")
            .join(format!("{}.json", record.session_id));
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(record),
            Err(e) => return Err(WorktreeError::io("read rename", &path, e)),
        };
        let rename: Rename = serde_json::from_slice(&bytes)
            .map_err(|e| WorktreeError::io("parse rename", &path, std::io::Error::other(e)))?;
        if record.branch != rename.from || record.decision == Some(super::Decision::Removed) {
            return Ok(record);
        }
        let renamed = read(&rename.copy, &["symbolic-ref", "-q", "--short", "HEAD"]).as_deref()
            == Some(&rename.to);
        if !renamed {
            return Ok(record);
        }
        let same_dir = read(&rename.copy, &["rev-parse", "--absolute-git-dir"])
            .and_then(|s| PathBuf::from(s).canonicalize().ok())
            == Some(rename.git_dir);
        let old = Call::new(&record.repo_root, Index::User)
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{}", rename.from),
            ])
            .timeout(Duration::from_secs(5))
            .output();
        let old_gone = old.is_ok_and(|o| o.code == Some(1));
        let same_copy = crate::repo_info::worktrees(&record.repo_root).is_ok_and(|trees| {
            trees.iter().skip(1).any(|w| {
                w.branch.as_deref() == Some(&rename.to)
                    && w.path.canonicalize().ok().as_deref() == Some(rename.copy.as_path())
            })
        });
        if same_dir && old_gone && same_copy {
            record.branch = rename.to;
            self.put_unlocked(&record)?;
            std::fs::remove_file(&path)
                .map_err(|e| WorktreeError::io("finish rename", &path, e))?;
        }
        Ok(record)
    }
}
