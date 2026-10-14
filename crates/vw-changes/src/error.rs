//! Typed errors of the Changes core. Callers match on the variant: `Stale` and `ConfirmDeletes` are user
//! states ("file changed, refresh", "confirm deleting these"), the rest are failures.
//!
//! Depends on: thiserror. Called by: every module of the crate, vw-core-api (maps them to `CoreError`).
//! Invariant: no message carries file contents; paths and git's stderr (clipped) only.

use std::path::PathBuf;

/// Why a Changes operation did not happen.
#[derive(Debug, thiserror::Error)]
pub enum ChangesError {
    /// `git` could not be started (not installed, not on PATH).
    #[error("git is not available: {0}")]
    GitMissing(String),
    /// A git command exited with an error.
    #[error("git {command} failed: {message}")]
    Git {
        /// The git subcommand (`add`, `diff`, ...).
        command: String,
        /// git's stderr, clipped.
        message: String,
    },
    /// A git command ran longer than allowed and was stopped.
    #[error("git {command} took longer than {seconds} s and was stopped")]
    Timeout {
        /// The git subcommand.
        command: String,
        /// The limit.
        seconds: u64,
    },
    /// A git command printed more than the output cap.
    #[error("git {command} printed more than {limit} bytes")]
    TooMuchOutput {
        /// The git subcommand.
        command: String,
        /// The cap.
        limit: usize,
    },
    /// The file (or the whole diff) is not what the user saw: nothing was changed.
    #[error("file changed, refresh: {path}")]
    Stale {
        /// The file, or `*` for a bulk action.
        path: String,
    },
    /// The action would delete files the request did not name; nothing was changed.
    #[error("confirm deleting {}", paths.join(", "))]
    ConfirmDeletes {
        /// Every file the action would delete.
        paths: Vec<String>,
    },
    /// The target is not in the current diff (or the hunk id is unknown).
    #[error("not in the current changes: {0}")]
    NotFound(String),
    /// The action does not apply to this target (a hunk of a binary file, a submodule, ...).
    #[error("{0}")]
    Refused(String),
    /// The baseline's objects are gone from the repository and cannot be rebuilt.
    #[error("the review baseline is lost: {0}")]
    BaselineLost(String),
    /// A filesystem operation in our data folder or the working tree failed.
    #[error("{action} {}: {source}", path.display())]
    Io {
        /// What was being done.
        action: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
    /// git printed something we could not read.
    #[error("unexpected git output: {0}")]
    Parse(String),
}

impl ChangesError {
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

    /// True for errors that mean "the repository lost objects we rely on" (a `git gc --prune=now` ran):
    /// the caller rebuilds the baseline's objects and the current index, then retries once.
    #[must_use]
    pub fn is_missing_object(&self) -> bool {
        match self {
            Self::Git { message, .. } => {
                let m = message.to_ascii_lowercase();
                m.contains("unable to read")
                    || m.contains("bad object")
                    || m.contains("missing blob")
                    || m.contains("missing tree")
                    || m.contains("invalid object")
                    || m.contains("is not a valid object")
                    || m.contains("bad tree object")
                    || m.contains("could not read")
            }
            _ => false,
        }
    }
}

/// Clips `text` to `max` characters on a char boundary (git stderr in errors and logs).
pub(crate) fn clip(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    match trimmed.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &trimmed[..cut]),
        None => trimmed.to_owned(),
    }
}
