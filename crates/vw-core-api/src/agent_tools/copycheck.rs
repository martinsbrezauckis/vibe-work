//! `copy_check`: the folder guard's question: **one active
//! session per copy.** A session copy (`$VIBE_HOME/worktrees/<repo>/<name>`) is held by the session working in it while
//! that session is ACTIVE; another session's agent is refused there while the holder is active, naming it; once the
//! holder is idle or closed, another session may work there (a later agent, a merge agent, a new session the user
//! starts there) and then holds it while it is active. Never two active sessions in one copy.
//!
//! - **Active** = the session's state in our store says `session.status.type` is `running` or `blocked` (a turn is
//!   running, or waiting for an approval or an answer inside a turn; Vibe 2.26.1 `PublicSessionStatus`, the same fact
//!   the Changes tab reads in `changes_facts.rs`). Idle (`idle`/no turn) or not in the store (closed) = not active.
//! - **Who uses a copy**: sessions in our store whose `cwd` / workspace roots lie in it, sessions whose history holds
//!   a Vibe `worktree` effect at it (`detail.input.path`, `_effect_models.py:213`), the record of the session that
//!   copy was made for (its folder name under a `<repo>-<hash>` folder of that repository), the session that last
//!   took it here (the lock), and the caller itself when the copy holds its own folder (`own`, from the hook payload's
//!   `cwd`: a subagent's own worktree our store does not know).
//! - **The caller**: the hook payload's `parent_session_id` when there is one (filled by Vibe's runtime, not the model:
//!   a subagent works inside its parent's turn and folder, so parent and subagent are one holder), else its
//!   `session_id`.
//! - **Fails closed**: a copy nobody is known to use is refused ("cannot tell which session uses this copy"); the whole
//!   decision (read the holders, take the lock) happens under one mutex, so two sessions racing for an idle copy cannot
//!   both get it.
//!
//! Depends on: the session store, the worktree records, `names` (titles). Called by: `server` (hook credential only).
//! Invariant: answers `{"decision": "allow"}` (and takes the lock for the caller) or `{"decision": "deny", "reason"}`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::Shared;

/// Copy folder -> the session that last took it.
#[derive(Default)]
pub(crate) struct CopyLocks(Mutex<HashMap<PathBuf, String>>);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Question {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    parent_session_id: Option<String>,
    #[serde(default)]
    copy: String,
    /// The copy holds the session's own folder (its `cwd`, from Vibe's hook payload): the caller uses it.
    #[serde(default)]
    own: bool,
}

/// True when `state` says a turn is running or waiting inside a turn.
pub(crate) fn active(state: &Value) -> bool {
    matches!(
        state
            .pointer("/session/status/type")
            .and_then(Value::as_str),
        Some("running" | "blocked")
    )
}

fn key(path: &Path) -> Result<PathBuf, String> {
    vw_agent_tools::folders::resolve(path)
        .map_err(|e| format!("the folder guard could not resolve a session folder: {e}"))
}

/// The sessions our store shows using `copy`, with whether each is active.
fn users_in_store(shared: &Shared, copy: &Path) -> Result<Vec<(String, bool)>, String> {
    let mut users = Vec::new();
    for view in shared.store.views() {
        let state = &view.state;
        let mut folders: Vec<&str> = Vec::new();
        folders.extend(state.pointer("/session/cwd").and_then(Value::as_str));
        folders.extend(
            state
                .pointer("/session/workspaceRoots")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
        for entry in state
            .pointer("/history")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if entry.pointer("/detail/kind").and_then(Value::as_str) == Some("worktree") {
                folders.extend(entry.pointer("/detail/input/path").and_then(Value::as_str));
            }
        }
        let mut uses_copy = false;
        for folder in folders.into_iter().filter(|f| !f.is_empty()) {
            if key(Path::new(folder))?.starts_with(copy) {
                uses_copy = true;
                break;
            }
        }
        if uses_copy {
            users.push((view.session_id.clone(), active(state)));
        }
    }
    Ok(users)
}

/// The session the record made `copy` for (folder name `<name>` under a `<repo dir name>-...` folder).
fn recorded_owner(shared: &Shared, copy: &Path) -> Result<Option<String>, String> {
    if shared.config.changes_dir.is_none() {
        return Ok(None);
    }
    let records = shared
        .worktrees
        .records()
        .map_err(|e| format!("the copy records could not be opened: {e}"))?;
    let Some(name) = copy.file_name().and_then(|n| n.to_str()) else {
        return Ok(None);
    };
    let Some(bucket) = copy
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
    else {
        return Ok(None);
    };
    let all = records
        .all_for_guard()
        .map_err(|e| format!("the copy records could not be read: {e}"))?;
    Ok(all
        .into_iter()
        .find(|r| {
            r.folder == name
                && r.repo_root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|repo| bucket.starts_with(&format!("{repo}-")))
        })
        .map(|r| r.session_id.clone()))
}

fn is_active(shared: &Shared, session: &str) -> bool {
    shared.store.view(session).is_some_and(|v| active(&v.state))
}

/// The one decision: `Ok` takes (or keeps) `copy` for `caller`.
fn decide(
    shared: &Shared,
    locks: &CopyLocks,
    caller: &str,
    copy: &Path,
    own: bool,
) -> Result<(), String> {
    let mut held = locks.0.lock().unwrap_or_else(PoisonError::into_inner);
    let mut users = users_in_store(shared, copy)?;
    if own && !users.iter().any(|(u, _)| u == caller) {
        users.push((caller.to_owned(), false));
    }
    for id in recorded_owner(shared, copy)?
        .into_iter()
        .chain(held.get(copy).cloned())
    {
        if !users.iter().any(|(u, _)| *u == id) {
            users.push((id.clone(), is_active(shared, &id)));
        }
    }
    if users.is_empty() {
        return Err("Vibe Work cannot tell which session uses this copy".to_owned());
    }
    if let Some((busy, _)) = users.iter().find(|(id, on)| *on && id != caller) {
        let title = shared.names.display_title(&shared.store, busy);
        return Err(format!(
            "this copy is in use by the session \"{}\"; it can be used once that session is idle or closed",
            vw_gate::dialog_value(&title, 80)
        ));
    }
    held.insert(copy.to_path_buf(), caller.to_owned());
    Ok(())
}

/// Answers one `copy_check`.
pub(crate) async fn check(shared: &Arc<Shared>, args: Value) -> Result<Value, String> {
    let question: Question =
        serde_json::from_value(args).map_err(|_| "bad copy_check request".to_owned())?;
    let caller = question
        .parent_session_id
        .clone()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| question.session_id.clone());
    if caller.is_empty() || question.copy.is_empty() {
        return Ok(json!({"decision": "deny", "reason": "no session or copy named"}));
    }
    let Some(tools) = shared.agent_tools.clone() else {
        return Ok(json!({"decision": "deny", "reason": "the agent tools are off"}));
    };
    let worker = shared.clone();
    let copy = match key(Path::new(&question.copy)) {
        Ok(copy) => copy,
        Err(reason) => return Ok(json!({"decision": "deny", "reason": reason})),
    };
    let own = question.own;
    let answer = tokio::task::spawn_blocking(move || {
        let verdict = decide(&worker, &tools.copy_locks, &caller, &copy, own);
        tracing::info!(session = %caller, allowed = verdict.is_ok(), "folder guard: copy");
        verdict
    })
    .await
    .map_err(|e| format!("copy_check failed: {e}"))?;
    Ok(match answer {
        Ok(()) => json!({"decision": "allow"}),
        Err(reason) => json!({"decision": "deny", "reason": reason}),
    })
}
