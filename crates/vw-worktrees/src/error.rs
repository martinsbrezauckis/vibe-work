//! Typed errors of the worktree core. Callers match on the variant: `NotOffered`, `InvalidName`, `Taken`
//! and `Stale` are user states (said in plain words), the rest are failures.
//!
//! Depends on: thiserror, vw-changes (`ChangesError` of the git runner). Called by: every module of the crate,
//! vw-core-api (maps them to `CoreError`).
//! Invariant: no message carries file contents; names, paths and git's clipped stderr only.

use std::path::PathBuf;

use vw_changes::ChangesError;

/// Why a worktree operation did not happen.
#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    /// The folder is not inside a git working tree (or git refuses it).
    #[error("{0}")]
    NotGit(String),
    /// A copy is not offered for this repository (the reason names the setting).
    #[error("{0}")]
    NotOffered(String),
    /// The branch or base name cannot be used.
    #[error("{0}")]
    InvalidName(String),
    /// The branch or the folder name exists already; nothing was created.
    #[error("{0}")]
    Taken(String),
    /// The worktree is not what the user saw (its changed files differ): nothing was removed.
    #[error("{0}")]
    Stale(String),
    /// A git call failed (from the hardened runner).
    #[error(transparent)]
    Git(#[from] ChangesError),
    /// A file of our own records could not be read or written.
    #[error("{action} {}: {source}", path.display())]
    Io {
        /// What was being done.
        action: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
}

impl WorktreeError {
    /// An `Io` error for `path`.
    pub(crate) fn io(
        action: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}

/// `text` clipped to `max` characters on a char boundary (git output in messages).
pub(crate) fn clip(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    match trimmed.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &trimmed[..cut]),
        None => trimmed.to_owned(),
    }
}
