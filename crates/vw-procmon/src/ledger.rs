//! The process ledger on disk (scope addition): every process this app knows to belong to
//! a session, so nothing it or the agent started is ever ownerless, also after a crash.
//!
//! One user-only JSON file per running app instance, `ledger-<pid>-<start token>.json`, in a user-only
//! directory (vw-platform `private_file`). An instance writes only its own file (atomic: new private temp
//! file, then rename). The next start reads the files of instances that are no longer alive (the startup
//! sweep, `lifecycle`); a file of a live instance (two windows of the app) is never touched.
//!
//! Depends on: vw-platform (`ensure_private_dir`, `write_private_new`), `serde_json`, `registry`, `types`.
//! Called by: `monitor`, `lifecycle`.
//! Invariants:
//! - Never stores arguments or environment values: an entry names the executable (from the OS-reported
//!   path, never argv[0]) and
//!   counts the arguments (security: argv redaction by patterns cannot be complete).
//! - A corrupt file is reported and left in place, never deleted.
//! - The lock is never held across an await (plain methods).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::error::LedgerError;
use crate::registry::{ProcessClass, ProcessGroup};
use crate::types::ProcessRef;

/// Ledger format version.
pub const LEDGER_VERSION: u32 = 2;
const PREFIX: &str = "ledger-";

/// One process in the ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
    /// Identity.
    pub process: ProcessRef,
    /// How to stop it as a whole.
    pub group: ProcessGroup,
    /// Owning session (`None` for app-wide processes such as `vibe-app-server`).
    pub session_id: Option<String>,
    /// Who started it.
    pub class: ProcessClass,
    /// Executable name (basename of the OS-reported executable path; `None` when unreadable). Arguments are
    /// never stored (they may hold secrets).
    pub name: Option<String>,
    /// Number of arguments after argv[0].
    pub arg_count: usize,
    /// "Keep running after Vibe Work closes" (explicit user choice only; default off).
    pub keep_running: bool,
    /// Ports it listened on when last seen.
    pub ports: Vec<u16>,
}

/// One instance's file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerFile {
    /// [`LEDGER_VERSION`].
    pub version: u32,
    /// The app instance that wrote it.
    pub instance: ProcessRef,
    /// Its processes.
    pub entries: Vec<LedgerEntry>,
}

/// A ledger file of another (earlier) instance, as found at start.
#[derive(Clone, Debug)]
pub struct ForeignLedger {
    /// Where it is.
    pub path: PathBuf,
    /// Its content.
    pub file: LedgerFile,
}

/// This instance's ledger.
pub struct Ledger {
    dir: PathBuf,
    path: PathBuf,
    instance: ProcessRef,
    entries: Mutex<BTreeMap<(u32, u64), LedgerEntry>>,
    written: Mutex<Option<String>>,
}

fn io(path: &Path, source: std::io::Error) -> LedgerError {
    LedgerError::Io {
        path: path.display().to_string(),
        source,
    }
}

impl Ledger {
    /// Opens (creates) the ledger directory as user-only and prepares this instance's file name. Nothing is
    /// written until the first [`Ledger::replace`].
    ///
    /// # Errors
    /// [`LedgerError`] when the directory cannot be created or made user-only.
    pub fn open(dir: &Path, instance: ProcessRef) -> Result<Self, LedgerError> {
        vw_platform::ensure_private_dir(dir)?;
        let path = dir.join(format!(
            "{PREFIX}{}-{}.json",
            instance.pid, instance.start.0
        ));
        Ok(Self {
            dir: dir.to_path_buf(),
            path,
            instance,
            entries: Mutex::new(BTreeMap::new()),
            written: Mutex::new(None),
        })
    }

    /// This instance's file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current entries.
    #[must_use]
    pub fn entries(&self) -> Vec<LedgerEntry> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// Replaces the whole entry set and writes the file when it changed.
    ///
    /// # Errors
    /// [`LedgerError`] when the file cannot be written (the in-memory set is updated anyway).
    pub fn replace(&self, entries: Vec<LedgerEntry>) -> Result<(), LedgerError> {
        let map: BTreeMap<(u32, u64), LedgerEntry> = entries
            .into_iter()
            .map(|e| ((e.process.pid, e.process.start.0), e))
            .collect();
        *self.entries.lock().unwrap_or_else(PoisonError::into_inner) = map;
        self.write()
    }

    fn write(&self) -> Result<(), LedgerError> {
        let file = LedgerFile {
            version: LEDGER_VERSION,
            instance: self.instance,
            entries: self.entries(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(|e| LedgerError::Corrupt {
            path: self.path.display().to_string(),
            reason: e.to_string(),
        })?;
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if written.as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        let temp = self.path.with_extension("json.new");
        // A temp file left by a crash mid-write is ours (same instance name): replace it.
        let _ = fs::remove_file(&temp);
        vw_platform::write_private_new(&temp, text.as_bytes())?;
        fs::rename(&temp, &self.path).map_err(|e| io(&self.path, e))?;
        *written = Some(text);
        Ok(())
    }

    /// Removes this instance's file (clean quit with nothing left to remember).
    ///
    /// # Errors
    /// [`LedgerError::Io`] when the file exists and cannot be removed.
    pub fn remove_file(&self) -> Result<(), LedgerError> {
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(&self.path, e)),
        }
        *self.written.lock().unwrap_or_else(PoisonError::into_inner) = None;
        Ok(())
    }

    /// Ledger files of other instances in the directory, with the corrupt ones reported separately.
    ///
    /// # Errors
    /// [`LedgerError::Io`] when the directory cannot be listed.
    pub fn others(&self) -> Result<(Vec<ForeignLedger>, Vec<LedgerError>), LedgerError> {
        let mut found = Vec::new();
        let mut corrupt = Vec::new();
        for entry in fs::read_dir(&self.dir)
            .map_err(|e| io(&self.dir, e))?
            .flatten()
        {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_json = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"));
            if !name.starts_with(PREFIX) || !is_json || path == self.path {
                continue;
            }
            match fs::read_to_string(&path)
                .map_err(|e| io(&path, e))
                .and_then(|text| {
                    serde_json::from_str::<LedgerFile>(&text).map_err(|e| LedgerError::Corrupt {
                        path: path.display().to_string(),
                        reason: e.to_string(),
                    })
                }) {
                Ok(file) => found.push(ForeignLedger { path, file }),
                Err(e) => corrupt.push(e),
            }
        }
        Ok((found, corrupt))
    }

    /// Deletes a swept file of a dead instance.
    ///
    /// # Errors
    /// [`LedgerError::Io`] when it cannot be removed.
    pub fn retire(path: &Path) -> Result<(), LedgerError> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io(path, e)),
        }
    }
}
