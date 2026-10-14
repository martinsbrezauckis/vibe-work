//! The project state on disk: `<dir>/projects.json`, user-only, replaced atomically after every change.
//!
//! Depends on: vw-platform (`ensure_private_dir`, `replace_private`, `set_aside`), `serde_json`, `state`.
//! Called by: vw-core-api (one store per core), the private tests.
//! Invariants:
//! - Memory never runs ahead of disk: [`ProjectStore::update`] applies a change to a copy, writes it, and swaps it in
//!   only when the write succeeded; a failed write changes nothing and returns [`ProjectsError::Io`].
//! - A file that does not parse (or names a newer format) is moved aside (`projects.json.bad-<ms>`) and the store
//!   starts empty; it is never overwritten.
//! - Temporary files of writes a crash cut short are removed at open (`remove_stale_temps`).
//! - Without a folder the state lives in memory only (tests, a core without app data).
//! - Plain blocking calls; the lock is never held across an await (the core calls from the blocking pool).

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::model::ProjectsError;
use crate::state::State;

/// The store file's name inside its folder.
pub const FILE_NAME: &str = "projects.json";

/// One core's project store.
pub struct ProjectStore {
    dir: Option<PathBuf>,
    state: Mutex<State>,
    fresh: bool,
    /// False when an existing store file could neither be read nor moved aside: saves are skipped so it is never
    /// overwritten (projects then live in memory for this run).
    writable: bool,
}

impl std::fmt::Debug for ProjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectStore")
            .field("dir", &self.dir)
            .field("fresh", &self.fresh)
            .field("writable", &self.writable)
            .finish_non_exhaustive()
    }
}

/// What [`load`] found: the state, whether this is the first run (no file at all), and whether the store may write
/// its file (false when an existing file could neither be read nor moved aside: writing would destroy it).
struct Loaded {
    state: State,
    fresh: bool,
    writable: bool,
}

/// Reads the store file. Only "not found" counts as a first run. Any other read error, an unparseable file or a
/// newer format moves the file aside (`projects.json.bad-<ms>`) and starts empty; when it cannot be moved either,
/// the store keeps working in memory only and never writes over it (the user's file stays as it was).
fn load(file: &std::path::Path) -> Loaded {
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                state: State::default(),
                fresh: true,
                writable: true,
            };
        }
        Err(e) => return set_aside_or_keep(file, &format!("could not be read ({e})")),
    };
    match serde_json::from_slice::<State>(&bytes) {
        Ok(state) if state.version <= State::VERSION => Loaded {
            state,
            fresh: false,
            writable: true,
        },
        Ok(_) => set_aside_or_keep(file, "names a newer format"),
        Err(_) => set_aside_or_keep(file, "does not parse"),
    }
}

/// Moves an unusable store file aside and starts empty; when the move fails, starts empty in memory only.
fn set_aside_or_keep(file: &std::path::Path, why: &str) -> Loaded {
    if let Some(aside) = vw_platform::set_aside(file, crate::now_ms()) {
        tracing::warn!(file = ?aside, why, "the projects file was moved aside; starting empty");
        Loaded {
            state: State::default(),
            fresh: false,
            writable: true,
        }
    } else {
        tracing::warn!(
            file = ?file,
            why,
            "the projects file could not be moved aside: projects are kept in memory only this run, the file is untouched"
        );
        Loaded {
            state: State::default(),
            fresh: false,
            writable: false,
        }
    }
}

/// Removes temporary files of writes a crash cut short (`projects.json.new-<pid>-<n>`, `replace_private`): the real
/// file is the old or the new state, the temporary one is garbage.
fn remove_stale_temps(dir: &std::path::Path) {
    let prefix = format!("{FILE_NAME}.new-");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

impl ProjectStore {
    /// Opens the store in `dir` (created user-only on the first write), or an in-memory store for `None`.
    #[must_use]
    pub fn open(dir: Option<PathBuf>) -> Self {
        let loaded = match &dir {
            Some(dir) => {
                remove_stale_temps(dir);
                load(&dir.join(FILE_NAME))
            }
            None => Loaded {
                state: State::default(),
                fresh: true,
                writable: true,
            },
        };
        Self {
            dir,
            state: Mutex::new(loaded.state),
            fresh: loaded.fresh,
            writable: loaded.writable,
        }
    }

    /// True when there was no store file at open (the first run: every session is filed by its folder).
    #[must_use]
    pub fn was_fresh(&self) -> bool {
        self.fresh
    }

    /// The store file, when the store has a folder.
    #[must_use]
    pub fn file(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(FILE_NAME))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads the state.
    pub fn read<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        f(&self.lock())
    }

    /// Applies `f` to a copy of the state; when the copy differs it is written and then kept. Returns `f`'s value
    /// and whether anything changed.
    ///
    /// # Errors
    /// `f`'s error (nothing changes), or [`ProjectsError::Io`] when the write failed (nothing changes).
    pub fn update<T>(
        &self,
        f: impl FnOnce(&mut State) -> Result<T, ProjectsError>,
    ) -> Result<(T, bool), ProjectsError> {
        let mut guard = self.lock();
        let mut next = guard.clone();
        let value = f(&mut next)?;
        if next == *guard {
            return Ok((value, false));
        }
        next.version = State::VERSION;
        self.save(&next)?;
        *guard = next;
        Ok((value, true))
    }

    fn save(&self, state: &State) -> Result<(), ProjectsError> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        // An existing file that could neither be read nor moved aside is never overwritten (memory-only run).
        if !self.writable {
            return Ok(());
        }
        vw_platform::ensure_private_dir(dir).map_err(|e| ProjectsError::Io(e.to_string()))?;
        let bytes = serde_json::to_vec(state).map_err(|e| ProjectsError::Io(e.to_string()))?;
        vw_platform::replace_private(&dir.join(FILE_NAME), &bytes)
            .map_err(|e| ProjectsError::Io(e.to_string()))
    }
}
