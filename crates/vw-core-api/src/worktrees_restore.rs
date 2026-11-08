//! Restore a recorded copy on explicit unarchive, and reconcile Vibe's recovered branch after a resume.
//! Uses pinned Vibe's Git-only recovery and the original repository's listing; called by dispatch/sessions.
//! No new copy or vault authority is inferred, and an adopted recovery branch is never deleted here.
//! A restored copy is a new folder holding only what git kept, so the `.worktreeinclude` files are copied into it
//! again, as at its start, never over a file it already has.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::core::Shared;
use crate::error::CoreError;
use crate::worktrees::{blocking, core_error};

/// Unarchive restores retained files without opening a session runtime. The caller owns the chat lock.
pub(crate) async fn unarchived(shared: &Arc<Shared>, params: &Value) -> Result<(), CoreError> {
    let Some(id) = params["sessionId"]
        .as_str()
        .filter(|_| params["archived"] == false)
    else {
        return Ok(());
    };
    let Ok(records) = shared.worktrees.records() else {
        return Ok(());
    };
    let Some(record) = records
        .get(id)
        .filter(|r| r.decision != Some(vw_worktrees::Decision::Removed))
    else {
        return Ok(());
    };
    let reply = shared
        .gate
        .call("session/read", serde_json::json!({"sessionId":id}), None)
        .await
        .map_err(|e| CoreError::from_gate("session/read", &e))?;
    let state = &reply.result["state"];
    let Some(cwd) = state["session"]["cwd"].as_str() else {
        return Err(CoreError::bad_result("session/read", "state.session.cwd"));
    };
    let cwd = PathBuf::from(cwd);
    if cwd.is_dir() {
        return Ok(());
    }
    let root = shared.worktrees.vibe_root();
    if cwd.file_name().and_then(|name| name.to_str()) != Some(record.folder.as_str())
        || cwd
            .parent()
            .is_none_or(|parent| crate::worktrees_remove::placement(parent, root).is_some())
    {
        return Err(CoreError::new(
            crate::ErrorCode::Denied,
            "The saved copy is outside its managed folder.",
        ));
    }
    let python = shared.plugin_approvals.vibe_python().ok_or_else(|| {
        CoreError::new(
            crate::ErrorCode::Internal,
            "Vibe's Python is unavailable for restoring this copy.",
        )
    })?;
    let home = shared.instructions.own_home.clone();
    let _held = shared.worktrees.lock.lock().await;
    blocking(move || {
        crate::plugin_identity::run_vibe_python(
            &python,
            include_str!("worktree_recover.py"),
            &[&cwd, &record.repo_root],
            &home,
            std::time::Duration::from_secs(60),
            1024,
        )
    })
    .await?
    .map_err(|error| {
        CoreError::new(
            crate::ErrorCode::Internal,
            format!("The chat's files could not be restored: {error}"),
        )
    })?;
    resumed(shared, state).await
}

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
