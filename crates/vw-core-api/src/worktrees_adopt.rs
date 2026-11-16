//! A working copy the agent made for its chat (Vibe's `enter_worktree`) becomes the chat's copy record, so the
//! chat shows it as its copy and its end offers the same inspection and Keep / Throw away choice as a copy made
//! at chat start.
//!
//! Only a copy Vibe made in its own worktree folder, of the repository the chat works in, is taken, and only for a
//! chat without a copy record yet. Removing it later never deletes its branch (`created_branch: false`): the agent
//! may have entered an existing branch, which is the person's.
//!
//! Depends on: `vw-worktrees` (repository facts, records). Called by: `worktrees` (`info`) and `worktrees_remove`
//! (the inspection target).

use std::path::Path;

use vw_worktrees::{Head, Record, Records};

/// The record for `session_id` sitting in `cwd`, when `cwd` is a linked worktree under `vibe_root` and the chat
/// has no record yet. Blocking (reads git).
pub(crate) fn adopt(
    records: &Records,
    session_id: &str,
    cwd: &Path,
    vibe_root: &Path,
) -> Option<Record> {
    if records.get(session_id).is_some() {
        return None;
    }
    let info = vw_worktrees::read(cwd).ok()??;
    if info.worktrees.len() < 2 || info.main_root() == info.root {
        return None;
    }
    let inside = |path: &Path, root: &Path| match (path.canonicalize(), root.canonicalize()) {
        (Ok(path), Ok(root)) => path.starts_with(root),
        _ => false,
    };
    if !inside(&info.root, vibe_root) {
        return None;
    }
    let Head::Branch { name: branch } = &info.head else {
        return None;
    };
    let main = info.worktrees.first()?;
    // A copy another chat's record already names stays that chat's (a new chat opened in it adopts nothing).
    if records
        .all()
        .iter()
        .any(|r| r.branch == *branch && r.repo_root == main.path)
    {
        return None;
    }
    let base_name = main.branch.clone().unwrap_or_else(|| branch.clone());
    let base_commit = info
        .branches
        .iter()
        .find(|b| b.name == base_name)
        .map(|b| b.commit.clone())
        .unwrap_or_default();
    let record = Record {
        session_id: session_id.to_owned(),
        repo_root: main.path.clone(),
        base_name,
        base_commit,
        branch: branch.clone(),
        folder: info.root.file_name()?.to_string_lossy().into_owned(),
        created_branch: false,
        created_at_ms: vw_changes::now_ms(),
        decision: None,
        session_deleted: false,
    };
    records.put(&record).ok()?;
    Some(record)
}
