//! Processes Vibe Work starts itself, registered by the code that starts them, so the monitor knows them as a
//! fact instead of inferring them.
//!
//! Depends on: `types`, serde. Called by: the core (it registers each terminal shell with its process
//! group / Windows Job Object from the terminal lifecycle events), the core (`vibe-app-server` itself),
//! `attribution`, `ledger`, tests.
//! Invariants:
//! - An entry is an identity (pid + start token), so a reused pid never matches it.
//! - The lock is never held across an await (plain methods).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::types::ProcessRef;

/// Who started a process, which decides what happens to it when its session ends or the app quits
/// and how it is managed when its session ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessClass {
    /// Vibe Work started it (`vibe-app-server`, our MCP server instances, helpers, our terminals' shells):
    /// stopped on session end, quit and, after a crash, by the next start's sweep.
    Owned,
    /// The agent started it (a Vibe background process, a dev server, anything it spawned): runs until
    /// someone explicitly asks to stop it, also across quit; re-attached to its session after a restart.
    Agent,
}

/// How a registered process can be stopped as a whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ProcessGroup {
    /// Unix process group (the process was started with its own group, so the group is exactly its tree).
    Unix {
        /// Group id.
        pgid: u32,
    },
    /// Windows Job Object (named by the code that owns the handle; the handle itself stays there).
    WindowsJob {
        /// A label for the job (diagnostics only).
        name: String,
    },
    /// No group: only the process (and its start-consistent descendants) is stopped.
    None,
}

/// One registered process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnProcess {
    /// The session it belongs to; `None` for app-wide processes (`vibe-app-server`).
    pub session_id: Option<String>,
    /// Its identity.
    pub process: ProcessRef,
    /// How to stop it as a whole.
    pub group: ProcessGroup,
    /// Who started it.
    pub class: ProcessClass,
    /// What it is ("terminal", "vibe-app-server",...), shown to the user. Never environment values.
    pub label: String,
    /// "Keep running after Vibe Work closes": only an explicit user choice sets it (UI later). Default off.
    pub keep_running: bool,
}

/// Handle returned by [`OwnProcessRegistry::register`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegistrationId(u64);

/// Thread-safe registry of the processes Vibe Work started. Clone it freely (shared inside).
#[derive(Clone, Default)]
pub struct OwnProcessRegistry {
    entries: Arc<Mutex<BTreeMap<RegistrationId, OwnProcess>>>,
    next: Arc<AtomicU64>,
}

impl OwnProcessRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<RegistrationId, OwnProcess>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a process; returns the id to unregister it with.
    #[must_use]
    pub fn register(&self, process: OwnProcess) -> RegistrationId {
        let id = RegistrationId(self.next.fetch_add(1, Ordering::Relaxed));
        self.lock().insert(id, process);
        id
    }

    /// Removes one registration (the process exited or was stopped). Returns it when it existed.
    #[must_use]
    pub fn unregister(&self, id: RegistrationId) -> Option<OwnProcess> {
        self.lock().remove(&id)
    }

    /// Removes every registration whose process is `process` (any registration id).
    pub fn unregister_process(&self, process: ProcessRef) {
        self.lock().retain(|_, p| p.process != process);
    }

    /// Every registration.
    #[must_use]
    pub fn list(&self) -> Vec<OwnProcess> {
        self.lock().values().cloned().collect()
    }

    /// Moves every registration of session `old` to `new` (a session id hand-off). Returns how many.
    #[must_use = "the moved items tell what followed the hand-off"]
    pub fn hand_off(&self, old: &str, new: &str) -> usize {
        let mut moved = 0;
        for process in self.lock().values_mut() {
            if process.session_id.as_deref() == Some(old) {
                process.session_id = Some(new.to_owned());
                moved += 1;
            }
        }
        moved
    }

    /// The registrations of one session.
    #[must_use]
    pub fn for_session(&self, session_id: &str) -> Vec<OwnProcess> {
        self.lock()
            .values()
            .filter(|p| p.session_id.as_deref() == Some(session_id))
            .cloned()
            .collect()
    }
}
