//! Reconcile a recorded copy after Vibe restores it onto a fresh recovery branch.
//! Uses only the successful resume's cwd and the original repository's Git listing; called by sessions.
//! No new copy or vault authority is inferred, and an adopted recovery branch is never deleted here.

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
        Ok(true)
    })
    .await??;
    if changed {
        shared.emit(crate::CoreEvent::WorktreesChanged {});
    }
    Ok(())
}
