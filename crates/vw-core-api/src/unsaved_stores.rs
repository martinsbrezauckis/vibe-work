//! Browser storage of a chat that Vibe never saved.
//!
//! Vibe keeps no file for a chat that never received a message, so after a restart nothing names that chat any more:
//! its per-session website data (cookies, local storage) stays under the preview storage with no route that can reach
//! or clear it. The core therefore records every chat it starts (`started-sessions.json` in the preview storage
//! folder) and, at the next start, asks Vibe for each one: a saved chat only leaves the record, a chat Vibe reports
//! as `not_found` is ended as a deleted chat (its preview data is wiped, leftovers retried by `previews`), and any
//! other answer (no server, a timeout) keeps the record for the next start.
//!
//! Depends on: `previews` (storage folder, session end), the gate (`session/read`). Called by: `plugin_refresh`
//! (chat started), `previews::session_ended` (chat deleted), `core_start` (startup sweep).
//! Invariants:
//! - A record never outlives a confirmed answer: it is removed only after Vibe said saved or not found.
//! - Chats started before this record existed are not known and are not swept.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;
use vw_platform::sync::LockRecover;

use crate::core::{Core, Shared};
use crate::error::CoreError;

const FILE: &str = "started-sessions.json";
/// One answer from Vibe; a chat that takes longer stays recorded for the next start.
const READ_WAIT: Duration = Duration::from_secs(10);
/// Serializes read-modify-write of every record file in this process.
static RECORD: Mutex<()> = Mutex::new(());

fn file(shared: &Shared) -> Option<PathBuf> {
    let hub = shared.preview.as_ref()?;
    Some(hub.pending_wipes.with_file_name(FILE))
}

fn read(file: &Path) -> Vec<String> {
    std::fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write(file: &Path, ids: &[String]) {
    let result = if ids.is_empty() {
        match std::fs::remove_file(file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
            _ => Ok(()),
        }
    } else {
        serde_json::to_vec(ids)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                vw_platform::replace_private(file, &bytes).map_err(|error| error.to_string())
            })
    };
    if let Err(error) = result {
        tracing::warn!(%error, "the record of started chats could not be written");
    }
}

/// Records a chat this core just started (`session/start`).
pub(crate) fn started(shared: &Shared, id: &str) {
    let Some(file) = file(shared) else {
        return;
    };
    let _held = RECORD.lock_or_recover();
    let mut ids = read(&file);
    if !ids.iter().any(|known| known == id) {
        ids.push(id.to_owned());
        write(&file, &ids);
    }
}

/// Drops the record of a chat that was deleted (its preview data went with it).
pub(crate) fn forget(shared: &Shared, id: &str) {
    let Some(file) = file(shared) else {
        return;
    };
    let _held = RECORD.lock_or_recover();
    let mut ids = read(&file);
    let before = ids.len();
    ids.retain(|known| known != id);
    if ids.len() != before {
        write(&file, &ids);
    }
}

/// What Vibe said about a recorded chat.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Vibe has the chat: it is an ordinary chat now.
    Saved,
    /// Vibe has no such chat: it was never saved.
    Unsaved,
    /// No answer that decides it.
    Unknown,
}

fn verdict(read: Result<(), CoreError>) -> Verdict {
    match read {
        Ok(()) => Verdict::Saved,
        Err(error) if error.server_code.as_deref() == Some("not_found") => Verdict::Unsaved,
        Err(_) => Verdict::Unknown,
    }
}

/// At startup: settles every recorded chat (see the module header).
pub(crate) async fn sweep(core: &Core) {
    let shared = &core.shared;
    let Some(file) = file(shared) else {
        return;
    };
    let recorded = {
        let _held = RECORD.lock_or_recover();
        read(&file)
    };
    let mut settled = Vec::new();
    for id in recorded {
        let call = shared
            .gate
            .call("session/read", json!({"sessionId": id}), None);
        let answer = match tokio::time::timeout(READ_WAIT, call).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(CoreError::from_gate("session/read", &error)),
            Err(_) => continue,
        };
        match verdict(answer) {
            Verdict::Saved => settled.push(id),
            Verdict::Unsaved => {
                crate::previews::session_ended(shared, &id, true).await;
                settled.push(id);
            }
            Verdict::Unknown => {}
        }
    }
    if settled.is_empty() {
        return;
    }
    let _held = RECORD.lock_or_recover();
    let mut ids = read(&file);
    ids.retain(|id| !settled.contains(id));
    write(&file, &ids);
}

#[cfg(test)]
#[path = "unsaved_stores_tests.rs"]
mod tests;
