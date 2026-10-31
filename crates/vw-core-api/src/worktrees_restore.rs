//! Reconcile a recorded copy after Vibe restores it onto a fresh recovery branch.
//! Uses only the successful resume's cwd and the original repository's Git listing; called by sessions.
//! No new copy or vault authority is inferred, and an adopted recovery branch is never deleted here.
//! A restored copy is a new folder holding only what git kept, so the `.worktreeinclude` files are copied into it
//! again, as at its start, never over a file it already has.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::core::Shared;
use crate::error::CoreError;
use crate::worktrees::{blocking, core_error};

pub(crate) async fn resumed(shared: &Arc<Shared>, state: &Value) -> Result<(), CoreError> {
    let (Some(id), Some(cwd)) = (
        state.pointer("/session/id").and_then(Value::as_str),
        state.pointer("/session/cwd").and_then(Value::as_str),
    ) else {
        return Ok(());
    };
    let Ok(records) = shared.worktrees.records() else {
        return Ok(());
    };
    let Some(record) = records.get(id) else {
        return Ok(());
    };
    if record.decision == Some(vw_worktrees::Decision::Removed) {
        return Ok(());
    }
    let path = PathBuf::from(cwd);
    let root = shared.worktrees.vibe_root().to_path_buf();
    let changed = blocking(move || -> Result<bool, CoreError> {
        if path.file_name().and_then(|n| n.to_str()) != Some(record.folder.as_str())
            || crate::worktrees_remove::placement(&path, &root).is_some()
        {
            return Ok(false);
        }
        let Some(branch) = vw_worktrees::repo_info::worktrees(&record.repo_root)
            .map_err(core_error)?
            .into_iter()
            .skip(1)
            .find(|w| w.path.canonicalize().ok() == path.canonicalize().ok())
            .and_then(|w| w.branch)
            .filter(|b| b != &record.branch)
        else {
            return Ok(false);
        };
        records
            .update(&record.session_id, |r| {
                r.branch = branch;
                r.created_branch = false;
            })
            .map_err(core_error)?;
        restore_includes(&record.repo_root, &path);
        Ok(true)
    })
    .await??;
    if changed {
        shared.emit(crate::CoreEvent::WorktreesChanged {});
    }
    Ok(())
}

/// Applies the repository's current `.worktreeinclude` plan to the restored copy `dest` (`include::apply` creates new
/// files only: a file the restore brought back stays as it is). Problems are logged; the resume itself goes on.
fn restore_includes(root: &std::path::Path, dest: &std::path::Path) {
    if std::fs::symlink_metadata(root.join(vw_worktrees::include::INCLUDE_FILE)).is_err() {
        return;
    }
    let others: Vec<PathBuf> = vw_worktrees::repo_info::worktrees(root)
        .map(|list| list.into_iter().skip(1).map(|w| w.path).collect())
        .unwrap_or_default();
    let plan = vw_worktrees::include::plan(root, &others);
    let done = vw_worktrees::include::apply(root, dest, &plan);
    if let Some(stopped) = done.stopped {
        tracing::warn!(%stopped, "a restored copy did not get its .worktreeinclude files");
    }
}
