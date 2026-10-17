//! Typed capture/apply errors; depends on thiserror and record metadata.
//! Clients receive a session record even when approval is refused or partly fails.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::Record;

/// Proposal capture or application failure.
#[derive(Debug, Error)]
pub enum Error {
    /// Absolute, parent-traversing, empty or non-Unicode relative path.
    #[error("invalid proposal path: {path}")]
    InvalidPath {
        /// Refused path.
        path: PathBuf,
    },
    /// A target or ancestor is a symlink.
    #[error("symlink in proposal path: {path}")]
    Symlink {
        /// Symlink that was found.
        path: PathBuf,
    },
    /// An existing parent resolves outside the captured root.
    #[error("proposal path resolves outside the protected root: {path}")]
    OutsideRoot {
        /// Relative path or changed root.
        path: PathBuf,
    },
    /// The root or an Edit/Remove target is not of the required type.
    #[error("not a {expected}: {path}")]
    WrongType {
        /// Path with the wrong type.
        path: PathBuf,
        /// Required filesystem type.
        expected: &'static str,
    },
    /// Add requires an absent target.
    #[error("add target already exists: {path}")]
    AlreadyExists {
        /// Existing relative path.
        path: PathBuf,
    },
    /// Edit and Remove require an existing target.
    #[error("proposal target does not exist: {path}")]
    MissingFile {
        /// Missing relative path.
        path: PathBuf,
    },
    /// More than one operation refers to the same normalized path.
    #[error("duplicate proposal path: {path}")]
    DuplicatePath {
        /// Repeated relative path.
        path: PathBuf,
    },
    /// There are no file operations to review.
    #[error("a proposal must contain at least one file change")]
    EmptyChanges,
    /// A proposal must belong to a nonempty session.
    #[error("a proposal requires a nonempty session id without control characters")]
    InvalidSessionId,
    /// Reason is empty, too long or includes non-text control characters.
    #[error(
        "reason must be nonempty plain text of at most {} UTF-8 bytes",
        crate::MAX_REASON_BYTES
    )]
    InvalidReason,
    /// Secure randomness for the proposal id could not be obtained.
    #[error("cannot create proposal id: {0}")]
    Entropy(String),
    /// The base hash or expected absence changed before approval.
    #[error("proposal is stale: {path}")]
    Stale {
        /// First relative path whose base no longer matches.
        path: PathBuf,
    },
    /// An operation failed; no later file operations are attempted.
    #[error("proposal I/O failed for {path}: {source}")]
    Io {
        /// Relative target, or the protected root during root validation.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: io::Error,
    },
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn path(&self) -> Option<&std::path::Path> {
        match self {
            Self::InvalidPath { path }
            | Self::Symlink { path }
            | Self::OutsideRoot { path }
            | Self::WrongType { path, .. }
            | Self::AlreadyExists { path }
            | Self::MissingFile { path }
            | Self::DuplicatePath { path }
            | Self::Stale { path }
            | Self::Io { path, .. } => Some(path),
            _ => None,
        }
    }
}

/// Failed approval and its bounded record, ready for the caller to persist.
#[derive(Debug, Error)]
#[error("{error}")]
pub struct ApplyError {
    /// Stale, unsafe path or filesystem failure.
    #[source]
    pub error: Error,
    /// Includes the final status and exactly which files were applied.
    pub record: Box<Record>,
}
