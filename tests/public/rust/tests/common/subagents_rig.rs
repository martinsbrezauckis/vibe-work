//! Disposable fake-Vibe setup and session helpers for Subagents setting tests.
//! Called by setting behavior tests that use the disposable server.

use std::path::{Path, PathBuf};

use super::channel_rig::{Rig, rig};
use super::fake::scratch;
use serde_json::json;
use vw_core_api::{Command, CommandResult, CoreError, SessionToolsResult};

/// A core over the fake with the subagents rule on; the record of temporary chats and the log of requests that
/// carry an `agentConfig` are kept in files of its own.
pub struct Setup {
    pub r: Rig,
    pub sessions_dir: PathBuf,
    pub agent_log: PathBuf,
}

pub async fn setup(name: &str, extra: &[&str]) -> Setup {
    let dir = scratch(&format!("{name}-side-files"));
    let sessions_dir = dir.join("sessions");
    let agent_log = dir.join("agent-config.log");
    let agent_flag = format!("--log-agent-config={}", agent_log.display());
    let mut flags = vec!["--subagents-rule", agent_flag.as_str()];
    flags.extend_from_slice(extra);
    let for_core = sessions_dir.clone();
    let r = rig(name, &flags, move |c| {
        c.sessions_dir = Some(for_core);
    })
    .await;
    Setup {
        r,
        sessions_dir,
        agent_log,
    }
}

pub async fn set_subagents(r: &Rig, enabled: bool) -> Result<SessionToolsResult, CoreError> {
    let command: Command = serde_json::from_value(
        json!({"type": "sessionTools", "request": {"op": "setSubagents", "enabled": enabled}}),
    )
    .unwrap();
    match r.run(command).await? {
        CommandResult::SessionTools { result } => Ok(result),
        other => panic!("{other:?}"),
    }
}

pub async fn temporary_ids(r: &Rig) -> Vec<String> {
    let command: Command =
        serde_json::from_value(json!({"type": "sessionTools", "request": {"op": "temporary"}}))
            .unwrap();
    match r.run(command).await.unwrap() {
        CommandResult::SessionTools {
            result: SessionToolsResult::Temporary { session_ids },
        } => session_ids,
        other => panic!("{other:?}"),
    }
}

pub async fn listed(r: &Rig) -> Vec<String> {
    let command = Command::ListSessions {
        limit: Some(50),
        cursor: None,
        include_archived: Some(true),
    };
    match r.run(command).await.unwrap() {
        CommandResult::Sessions { rows, .. } => {
            let mut ids: Vec<String> = rows
                .into_iter()
                .map(|row| row.session["id"].as_str().unwrap_or_default().to_owned())
                .collect();
            ids.sort();
            ids
        }
        other => panic!("{other:?}"),
    }
}

/// The ids in the temporary record file.
pub fn record(sessions_dir: &Path) -> Vec<String> {
    let bytes = std::fs::read(sessions_dir.join("temporary.json")).expect("the record exists");
    serde_json::from_slice(&bytes).expect("the record is a list")
}

/// The sessions every `method` request named, in order.
pub fn named(r: &Rig, method: &str) -> Vec<String> {
    r.log()
        .into_iter()
        .filter(|(_, m, _)| m == method)
        .map(|(_, _, s)| s)
        .collect()
}

/// Where in the whole request log `method` for `session` first appears.
pub fn position(r: &Rig, method: &str, session: &str) -> usize {
    r.log()
        .iter()
        .position(|(_, m, s)| m == method && s == session)
        .unwrap_or_else(|| panic!("no {method} for {session:?} in {:?}", r.log()))
}

/// Starts a chat of the user's in `folder` and runs one turn in it (so Vibe counts it as an existing chat).
pub async fn users_chat_in(r: &Rig, folder: &str) -> String {
    let started = r
        .run(Command::StartSession {
            agent_config: Some(json!({"cwd": folder})),
        })
        .await
        .unwrap();
    let CommandResult::Session { snapshot, .. } = started else {
        panic!("{started:?}")
    };
    r.full_turn(&snapshot.session_id).await;
    snapshot.session_id
}
