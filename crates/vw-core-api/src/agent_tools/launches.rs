//! The launch file: for every process started with `process_start`, the command, folder, port and
//! log path the agent asked for, so `process_restart` and `processes_list` work after an app restart too.
//!
//! A companion of the procmon ledger, kept apart on purpose: `ledger-*.json` never stores
//! arguments, while this file holds exactly the commands the agent wrote (already in
//! its transcript). It is user-only, written atomically (new private file, then rename), read at core start,
//! and never served by the Debug API. Entries whose process is gone are dropped by `retain_alive`, with their
//! output logs.
//!
//! Depends on: vw-platform (user-only files), vw-procmon (`ProcessRef`), `serde_json`. Called by: `tools`,
//! `mod` (load at start).
//! Invariant: the lock is never held across an await (plain methods).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use vw_procmon::ProcessRef;

/// One started process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Launch {
    /// The session that started it.
    pub(crate) session_id: String,
    /// Its identity.
    pub(crate) process: ProcessRef,
    /// The command line as the agent wrote it.
    pub(crate) command: String,
    /// The shell selected for this launch (absent in older launch files).
    #[serde(default)]
    pub(crate) shell: Option<String>,
    /// Its working folder.
    pub(crate) cwd: PathBuf,
    /// The leased port given as `PORT`.
    pub(crate) port: Option<u16>,
    /// Its output log.
    pub(crate) log: PathBuf,
    /// Milliseconds since the Unix epoch when it started.
    pub(crate) started_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    launches: Vec<Launch>,
}

/// The store.
pub(crate) struct Launches {
    path: PathBuf,
    entries: Mutex<Vec<Launch>>,
}

impl Launches {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Launch>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Loads `path` (missing or unreadable = empty; an unreadable file is left in place and logged).
    pub(crate) fn load(path: &Path) -> Self {
        let entries = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<File>(&text) {
                Ok(file) => file.launches,
                Err(error) => {
                    tracing::warn!(%error, "launch file unreadable; starting empty");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Self {
            path: path.to_path_buf(),
            entries: Mutex::new(entries),
        }
    }

    /// Every launch.
    pub(crate) fn all(&self) -> Vec<Launch> {
        self.lock().clone()
    }

    /// The launch of `process`, if recorded.
    pub(crate) fn of(&self, process: ProcessRef) -> Option<Launch> {
        self.lock().iter().find(|l| l.process == process).cloned()
    }

    /// Records a launch and writes the file.
    pub(crate) fn insert(&self, launch: Launch) {
        let mut entries = self.lock();
        entries.retain(|l| l.process != launch.process);
        entries.push(launch);
        self.write(&entries);
    }

    /// Forgets `process` (and deletes its output log) and writes the file.
    pub(crate) fn remove(&self, process: ProcessRef) {
        self.retain(|l| l.process != process);
    }

    /// Moves the launches of session `old` to `new` (a hand-off) and writes the file when one moved.
    pub(crate) fn hand_off(&self, old: &str, new: &str) {
        let mut entries = self.lock();
        let mut moved = false;
        for launch in entries.iter_mut().filter(|l| l.session_id == old) {
            new.clone_into(&mut launch.session_id);
            moved = true;
        }
        if moved {
            self.write(&entries);
        }
    }

    /// Keeps only launches whose process `alive` says is still running (the others' logs are deleted).
    pub(crate) fn retain_alive(&self, alive: impl Fn(ProcessRef) -> bool) {
        self.retain(|l| alive(l.process));
    }

    fn retain(&self, keep: impl Fn(&Launch) -> bool) {
        let mut entries = self.lock();
        let (kept, gone): (Vec<Launch>, Vec<Launch>) = entries.drain(..).partition(|l| keep(l));
        *entries = kept;
        if !gone.is_empty() {
            for launch in &gone {
                let _ = std::fs::remove_file(&launch.log);
            }
            self.write(&entries);
        }
    }

    fn write(&self, entries: &[Launch]) {
        let file = File {
            version: 1,
            launches: entries.to_vec(),
        };
        let Ok(text) = serde_json::to_vec_pretty(&file) else {
            return;
        };
        let temp = self.path.with_extension("json.new");
        let _ = std::fs::remove_file(&temp);
        let written = vw_platform::write_private_new(&temp, &text)
            .map_err(|e| e.to_string())
            .and_then(|()| std::fs::rename(&temp, &self.path).map_err(|e| e.to_string()));
        if let Err(error) = written {
            tracing::warn!(%error, "launch file not written");
        }
    }
}
