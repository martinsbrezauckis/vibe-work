//! The access mode (Vibe agent profile) each session runs with, kept by the core so that a reopened
//! session runs in the mode the user chose.
//!
//! Design: Vibe Work keeps each session's access mode in the core and applies it itself on every
//! reopen. Every resume the core sends names the session's selected profile, and the core then
//! switches the session to the profile it should run in (`session/agent/update`, `restore.rs`) and
//! checks Vibe's answer before any command of that session is sent. A reopened session therefore
//! runs with the access mode the user chose; app commands wait for the verified restore. The core
//! resumes sessions on its own (a new channel before a turn, reopening after a crash or after an
//! idle close), so this applies to every reopen.
//!
//! Depends on: `serde_json`, tokio (restore ordering), vw-platform (user-only file). Called by: `pump` (`runtime/updated`), `sessions` (results and start
//! params), `dispatch` (the profile a client's resume names).
//! Invariants:
//! - The remembered profile is the last one Vibe reported for the session (`runtime.activeAgent.name`
//!   in `runtime/updated` or any result carrying a runtime), else the `agentConfig.agent` it was
//!   started or resumed with, else the profile of an accepted `session/agent/update`.
//! - The remembered profiles are kept in memory and, when the app configures `sessions_dir`, in
//!   `<sessions_dir>/access.json` (user-only, written whole on every change), so a session reopened after
//!   the app restarted gets the mode the user chose for it. The app's data folder protects this setting.
//! - Full access (`auto-approve`) the user confirmed for a session is put back on reopen without a new
//!   dialog (`restore.rs`, trusted token for that exact switch). A custom profile name is never selected
//!   by the core: it selects [`FALLBACK_PROFILE`] (`ask`, the stricter mode) instead and the user is told.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use serde_json::Value;
use tracing::warn;

/// The profile the core selects instead of one it may not select on its own.
pub(crate) const FALLBACK_PROFILE: &str = "ask";
/// The record's file name inside `sessions_dir`.
const FILE: &str = "access.json";

/// Sets the profile of a resume, keeping the other configuration keys.
pub(crate) fn set_resume_profile(params: &mut Value, profile: &str) {
    let Some(object) = params.as_object_mut() else {
        return;
    };
    let config = object
        .entry("agentConfig")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !config.is_object() {
        *config = Value::Object(serde_json::Map::new());
    }
    if let Some(config) = config.as_object_mut() {
        config.insert("agent".to_owned(), Value::String(profile.to_owned()));
    }
}

/// Remembered profiles by session id (in memory, mirrored to disk when a folder is configured).
#[derive(Default)]
pub(crate) struct Access {
    agents: Mutex<HashMap<String, String>>,
    restores: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    file: Option<PathBuf>,
}

impl Access {
    /// Orders attachment, verification and runtime reads for one session. Idle locks are pruned;
    /// the channel transport has its own lock for the individual resume/close calls.
    pub(crate) async fn lock_restore(&self, session_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut restores = self.restores.lock().unwrap_or_else(PoisonError::into_inner);
            restores.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = restores.get(session_id).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                restores.insert(session_id.to_owned(), Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }

    /// The record for `sessions_dir` (`None`: memory only), with the profiles an earlier run left in its file.
    /// An unreadable or malformed file counts as empty: those sessions then resolve as before (Vibe's record,
    /// else `ask`), never to a mode nobody chose.
    pub(crate) fn load(sessions_dir: Option<PathBuf>) -> Self {
        let file = sessions_dir.map(|dir| dir.join(FILE));
        let agents = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|bytes| serde_json::from_slice::<HashMap<String, String>>(&bytes).ok())
            .unwrap_or_default();
        Self {
            agents: Mutex::new(agents),
            restores: Mutex::new(HashMap::new()),
            file,
        }
    }

    fn set(&self, session_id: &str, agent: &str) {
        let mut agents = self.agents.lock().unwrap_or_else(PoisonError::into_inner);
        // `runtime/updated` repeats the profile on every update: write only a change.
        if agents.get(session_id).map(String::as_str) != Some(agent) {
            agents.insert(session_id.to_owned(), agent.to_owned());
            self.write(&agents);
        }
    }

    /// Writes the whole record (sorted, so the file is stable). Called with the lock held, so two
    /// writers never interleave. A failed write is logged: the memory copy still serves this run.
    fn write(&self, agents: &HashMap<String, String>) {
        let Some(file) = &self.file else { return };
        let sorted: BTreeMap<&String, &String> = agents.iter().collect();
        let Ok(bytes) = serde_json::to_vec(&sorted) else {
            return;
        };
        if let Some(dir) = file.parent()
            && let Err(error) = vw_platform::ensure_private_dir(dir)
        {
            warn!(%error, "access modes: folder not created");
            return;
        }
        if let Err(error) = vw_platform::replace_private(file, &bytes) {
            warn!(%error, "access modes: record not written");
        }
    }

    /// Records the profile in a runtime snapshot (`runtime/updated` params' `runtime`, or a result's).
    pub(crate) fn observe_runtime(&self, session_id: &str, runtime: &Value) {
        if let Some(name) = runtime.pointer("/activeAgent/name").and_then(Value::as_str) {
            self.set(session_id, name);
        }
    }

    /// Records the profile a call asked for: `agentConfig.agent` of a start, continue or fork,
    /// `agentName` of an accepted `session/agent/update`. Not a resume's: it always names
    /// the resolved profile, which the restore verifies. A runtime Vibe reports later overrides it.
    pub(crate) fn observe_request(&self, method: &str, session_id: &str, params: &Value) {
        let asked = match method {
            "session/start" | "session/continue" | "session/fork" => {
                params.pointer("/agentConfig/agent")
            }
            "session/agent/update" => params.get("agentName"),
            _ => None,
        };
        if let Some(name) = asked.and_then(Value::as_str) {
            self.set(session_id, name);
        }
    }

    /// Session `old` was handed off to `new`: `new` runs with `old`'s profile (Vibe derives the new
    /// session from the old one's settings) until Vibe reports otherwise.
    pub(crate) fn hand_off(&self, old: &str, new: &str) {
        let mut agents = self.agents.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(agent) = agents.get(old).cloned()
            && !agents.contains_key(new)
        {
            agents.insert(new.to_owned(), agent);
            self.write(&agents);
        }
    }

    /// Forgets a deleted session.
    pub(crate) fn forget(&self, session_id: &str) {
        let mut agents = self.agents.lock().unwrap_or_else(PoisonError::into_inner);
        if agents.remove(session_id).is_some() {
            self.write(&agents);
        }
    }

    /// The profile `session_id` ran with, if known.
    pub(crate) fn remembered(&self, session_id: &str) -> Option<String> {
        self.agents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::Access;
    use serde_json::json;

    #[test]
    fn the_profile_vibe_last_reported_wins() {
        let access = Access::default();
        assert_eq!(access.remembered("s"), None);
        access.observe_request(
            "session/start",
            "s",
            &json!({"agentConfig": {"agent": "plan"}}),
        );
        assert_eq!(access.remembered("s").as_deref(), Some("plan"));
        access.observe_runtime("s", &json!({"activeAgent": {"name": "ask"}}));
        assert_eq!(access.remembered("s").as_deref(), Some("ask"));
        access.observe_request(
            "session/agent/update",
            "s",
            &json!({"sessionId": "s", "agentName": "auto-approve"}),
        );
        assert_eq!(access.remembered("s").as_deref(), Some("auto-approve"));
        access.observe_runtime("s", &json!({"config": {}}));
        assert_eq!(
            access.remembered("s").as_deref(),
            Some("auto-approve"),
            "a runtime without an agent changes nothing"
        );
        access.forget("s");
        assert_eq!(access.remembered("s"), None);
    }

    #[test]
    fn the_record_survives_a_restart_and_forgets_deleted_sessions() {
        let dir = std::env::temp_dir().join(format!("vw-access-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = Access::load(Some(dir.clone()));
        first.observe_request(
            "session/agent/update",
            "full",
            &json!({"sessionId": "full", "agentName": "auto-approve"}),
        );
        first.observe_runtime("asks", &json!({"activeAgent": {"name": "ask"}}));
        first.hand_off("full", "full-2");
        let second = Access::load(Some(dir.clone()));
        assert_eq!(second.remembered("full").as_deref(), Some("auto-approve"));
        assert_eq!(second.remembered("full-2").as_deref(), Some("auto-approve"));
        assert_eq!(second.remembered("asks").as_deref(), Some("ask"));
        second.forget("full");
        let third = Access::load(Some(dir.clone()));
        assert_eq!(third.remembered("full"), None);
        assert_eq!(third.remembered("asks").as_deref(), Some("ask"));
        std::fs::write(dir.join("access.json"), b"not json").unwrap();
        assert_eq!(
            Access::load(Some(dir.clone())).remembered("asks"),
            None,
            "a malformed record counts as empty"
        );
        assert_eq!(
            Access::load(None).remembered("asks"),
            None,
            "no folder: memory only"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
