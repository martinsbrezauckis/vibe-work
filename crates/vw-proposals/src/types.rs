//! Public request, review and decision types; serde is used only for review metadata.
//! Called by proposal clients and the record builder; file bytes stay out of records.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Maximum UTF-8 byte length of the agent's plain-text reason.
pub const MAX_REASON_BYTES: usize = 1024;
/// Maximum combined diff-text bytes retained in one session record.
pub const MAX_RECORD_DIFF_BYTES: usize = 64 * 1024;

/// Metadata and the file-change list passed to [`crate::Proposal::new`].
/// Keeping these together makes every captured proposal immediately recordable.
#[derive(Debug)]
pub struct Request {
    /// Session that owns this proposal.
    pub session_id: String,
    /// Agent explanation, treated as plain text, never parsed as markup.
    pub reason: String,
    /// Changes in application order, relative to one protected root.
    pub changes: Vec<Change>,
}

/// A proposed file operation. Content is bytes, including binary files.
#[derive(Debug)]
pub enum Change {
    /// Create a file that does not exist; its parent directory must exist.
    Add {
        /// Path relative to the protected root.
        path: PathBuf,
        /// Complete proposed file bytes.
        content: Vec<u8>,
    },
    /// Replace an existing regular file, preserving its captured permissions.
    Edit {
        /// Path relative to the protected root.
        path: PathBuf,
        /// Complete proposed file bytes.
        content: Vec<u8>,
    },
    /// Remove an existing regular file.
    Remove {
        /// Path relative to the protected root.
        path: PathBuf,
    },
}

impl Change {
    /// Relative path supplied by the agent.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Add { path, .. } | Self::Edit { path, .. } | Self::Remove { path } => path,
        }
    }

    /// Operation kind without its proposed content.
    #[must_use]
    pub fn kind(&self) -> Kind {
        match self {
            Self::Add { .. } => Kind::Add,
            Self::Edit { .. } => Kind::Edit,
            Self::Remove { .. } => Kind::Remove,
        }
    }

    pub(crate) fn into_content(self) -> Option<Vec<u8>> {
        match self {
            Self::Add { content, .. } | Self::Edit { content, .. } => Some(content),
            Self::Remove { .. } => None,
        }
    }
}

/// File operation without file content.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Create a file.
    Add,
    /// Replace a file.
    Edit,
    /// Delete a file.
    Remove,
}

/// Whether both sides can be displayed as UTF-8 text without NUL bytes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    /// Both sides are text.
    Text,
    /// At least one side is binary; no diff text is produced.
    Binary,
}

/// One captured file's complete review, independent of later filesystem changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileReview {
    /// Add, edit or remove.
    pub kind: Kind,
    /// Normalized path relative to the protected root.
    pub path: PathBuf,
    /// Text or binary.
    pub content_kind: ContentKind,
    /// Original bytes; zero for Add.
    pub before_bytes: usize,
    /// Proposed bytes; zero for Remove.
    pub after_bytes: usize,
    /// Unified diff for text, including file headers; None for binary.
    pub diff: Option<String>,
    /// Added text lines; zero for binary.
    pub added_lines: usize,
    /// Removed text lines; zero for binary.
    pub removed_lines: usize,
}

impl FileReview {
    /// Compact description, also used for binary files without a diff.
    #[must_use]
    pub fn summary(&self) -> String {
        let kind = match self.content_kind {
            ContentKind::Text => "text",
            ContentKind::Binary => "binary",
        };
        format!(
            "{kind}, {} bytes -> {} bytes",
            self.before_bytes, self.after_bytes
        )
    }
}

/// Explicit user decision supplied by the future review client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    /// Revalidate all files, then apply in request order.
    Approve,
    /// Record the decline without reading or writing the protected root.
    Decline,
}

/// Final outcome stored in session history. Pending proposals have no decision yet.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// All proposed changes were applied.
    Approved,
    /// The user declined; no files were applied.
    Declined,
    /// A base no longer matches; no files were applied.
    Stale,
    /// A path check or filesystem operation failed; consult the applied-file list.
    Failed,
}
