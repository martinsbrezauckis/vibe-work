//! Typed errors of the git outer loop. Callers match on the variant: `Blocked`, `Stale`, `Rejected` and
//! `NotFound` are user states said in plain words (the UI shows the text as is), the rest are failures.
//!
//! Depends on: thiserror, vw-changes (`ChangesError` of the hardened runner). Called by: every module of the
//! crate, vw-core-api (maps them to `CoreError`).
//! Invariant: no message carries file contents or credentials; names, paths, scrubbed addresses and git's
//! clipped stderr only.

use std::path::PathBuf;

use vw_changes::ChangesError;

/// Why a git action did not happen.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// The action is not possible in this state (detached checkout, a merge in progress, a repository
    /// setting that names a program, nothing to commit, ...). The text says why and what to do.
    #[error("{0}")]
    Blocked(String),
    /// The folder is not what the user saw (files or the branch moved): nothing was changed.
    #[error("{0}")]
    Stale(String),
    /// The remote or the forge said no (non-fast-forward, authentication, a PR exists): git's or gh's words.
    #[error("{0}")]
    Rejected(String),
    /// A branch, remote or pull request the request names does not exist.
    #[error("{0}")]
    NotFound(String),
    /// A git call failed (from the hardened runner).
    #[error(transparent)]
    Git(#[from] ChangesError),
    /// `gh` could not be started or printed something unexpected.
    #[error("gh: {0}")]
    Gh(String),
    /// A file of our own could not be read or written.
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

impl GitError {
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
#[must_use]
pub fn clip(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    match trimmed.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &trimmed[..cut]),
        None => trimmed.to_owned(),
    }
}
