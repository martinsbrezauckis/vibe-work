//! Protected-folder proposals, captured without writes for future review clients.
//! Depends on standard paths, SHA-256, similar and tempfile; has no app dependency.
//! Callers provide the user decision and persist the returned record, including errors.
//! Approval preflights all paths and bases before writing; I/O failure stops without rollback.

mod apply;
mod error;
mod paths;
mod record;
mod review;
mod types;

pub use error::{ApplyError, Error};
pub use record::{Record, RecordFile, RecordProblem};
pub use types::{
    Change, ContentKind, Decision, FileReview, Kind, MAX_REASON_BYTES, MAX_RECORD_DIFF_BYTES,
    Request, Status,
};

use std::collections::HashSet;
use std::fmt;
use std::fs::{self, Permissions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

struct Snapshot {
    path: PathBuf,
    kind: Kind,
    base: Option<Vec<u8>>,
    base_hash: Option<[u8; 32]>,
    permissions: Option<Permissions>,
    content: Option<Vec<u8>>,
}

/// Immutable captured proposal for one protected root.
/// Construct it with a [`Request`] containing the session, reason and file list.
/// Only [`Self::apply`] writes; consuming it prevents a second decision on the same instance.
pub struct Proposal {
    id: String,
    session_id: String,
    created_at_ms: u64,
    reason: String,
    root: PathBuf,
    canonical_root: PathBuf,
    create_parents: bool,
    files: Vec<Snapshot>,
}

impl fmt::Debug for Proposal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Proposal")
            .field("id", &self.id)
            .field("session_id", &self.session_id)
            .field("files", &self.files.len())
            .finish_non_exhaustive()
    }
}

impl Proposal {
    /// Validate paths, read existing bytes and capture their SHA-256 bases without writing.
    /// Add requires absence; Edit/Remove require regular files. Parents must already exist.
    /// Dot components are normalized; symlinks in the root or target ancestry are refused.
    /// Paths must be Unicode so every accepted proposal can produce a serializable record.
    ///
    /// # Errors
    /// Invalid metadata, duplicate/unsafe paths, unsuitable file types, I/O or entropy failure.
    pub fn new(root: impl AsRef<Path>, changes: Request) -> Result<Self, Error> {
        Self::capture(root.as_ref(), changes, false)
    }

    /// Capture additions whose parent folders do not exist yet. No folders are created until approval.
    /// Existing ancestors and newly introduced symlinks are checked before any mutation.
    ///
    /// # Errors
    /// The same capture errors as [`Self::new`].
    pub fn with_new_directories(root: impl AsRef<Path>, changes: Request) -> Result<Self, Error> {
        Self::capture(root.as_ref(), changes, true)
    }

    fn capture(root: &Path, changes: Request, create_parents: bool) -> Result<Self, Error> {
        validate_request(&changes)?;
        let root = paths::absolute_root(root)?;
        let canonical_root = paths::validate_root(&root)?;
        let mut seen = HashSet::new();
        let mut files = Vec::with_capacity(changes.changes.len());
        for change in changes.changes {
            let path = paths::normalize(change.path())?;
            if !seen.insert(path.clone()) {
                return Err(Error::DuplicatePath { path });
            }
            let target =
                paths::validate_target_with_parents(&canonical_root, &path, create_parents)?;
            let kind = change.kind();
            match (kind, &target.metadata) {
                (Kind::Add, Some(_)) => return Err(Error::AlreadyExists { path }),
                (Kind::Edit | Kind::Remove, None) => return Err(Error::MissingFile { path }),
                (_, Some(metadata)) if !metadata.is_file() => {
                    return Err(Error::WrongType {
                        path,
                        expected: "regular file",
                    });
                }
                _ => {}
            }
            let base = target
                .metadata
                .as_ref()
                .map(|_| fs::read(&target.absolute).map_err(|error| Error::io(&path, error)))
                .transpose()?;
            let base_hash = base.as_deref().map(hash);
            files.push(Snapshot {
                path,
                kind,
                base,
                base_hash,
                permissions: target.metadata.map(|metadata| metadata.permissions()),
                content: change.into_content(),
            });
        }
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| Error::Entropy(error.to_string()))?;
        Ok(Self {
            id: format!("{:032x}", u128::from_be_bytes(random)),
            session_id: changes.session_id,
            created_at_ms: now_ms(),
            reason: changes.reason,
            root,
            canonical_root,
            create_parents,
            files,
        })
    }

    /// Proposal identity generated from 128 bits of OS randomness.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Session owning the proposal and its record.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Creation time in Unix milliseconds.
    #[must_use]
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Bounded plain-text agent explanation.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Per-file reviews computed solely from captured bytes; text diffs are complete.
    #[must_use]
    pub fn review(&self) -> Vec<FileReview> {
        self.files.iter().map(review::file).collect()
    }

    /// Consume one user decision and return its session record.
    /// Approve validates all paths and base hashes before the first mutation. Each write
    /// uses a same-folder temporary file and atomic rename; Removes delete the file.
    /// Decline performs no filesystem I/O. The caller must persist both success and error records.
    ///
    /// # Errors
    /// Stale bases or unsafe paths apply nothing. I/O errors stop immediately and report
    /// already-applied files in [`ApplyError::record`]; there is no rollback journal.
    pub fn apply(self, decision: Decision) -> Result<Record, ApplyError> {
        apply::run(&self, decision)
    }
}

fn validate_request(request: &Request) -> Result<(), Error> {
    if request.session_id.trim().is_empty() || request.session_id.chars().any(char::is_control) {
        return Err(Error::InvalidSessionId);
    }
    if request.reason.trim().is_empty()
        || request.reason.len() > MAX_REASON_BYTES
        || request
            .reason
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(Error::InvalidReason);
    }
    if request.changes.is_empty() {
        return Err(Error::EmptyChanges);
    }
    Ok(())
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
