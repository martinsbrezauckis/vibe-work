//! Diff line comments: the user's private notes on lines of a session's changes, kept in Vibe Work's
//! own data, never in the repository. Same discipline as the page comments (`vw-pages` `CommentStore`): one JSON
//! file per session, user-only, replaced atomically; a file that does not parse is moved aside and the session
//! starts with no comments, never overwritten. A comment whose line is no longer in the diff is **outdated**
//! (decided by [`crate::line_anchor::locate`] at read time); it stays in the file until the user deletes it.
//!
//! Depends on: serde, sha2, vw-platform (`ensure_private_dir`, `replace_private`, `set_aside`), `line_anchor`,
//! `model`. Called by: vw-core-api (`lineComments` command).
//! Invariants:
//! - File: `<dir>/<sha256(session id)[..16] hex>.json`; a session id never becomes a path component.
//! - Text at most [`MAX_TEXT`] characters, [`MAX_PER_SESSION`] comments per session.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::line_anchor::LineAnchor;
use crate::model::DiffLine;
use crate::store::now_ms;

/// Longest comment text.
pub const MAX_TEXT: usize = 4000;
/// Most comments per session.
pub const MAX_PER_SESSION: usize = 2000;

/// One comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LineComment {
    /// Random id.
    pub id: String,
    /// The file (repository-relative path as the review shows it).
    pub path: String,
    /// The commented lines as the user saw them, with their numbers then.
    pub lines: Vec<DiffLine>,
    /// Where it sits.
    pub anchor: LineAnchor,
    /// The comment.
    pub text: String,
    /// Written at, ms since the Unix epoch.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_ms: u64,
    /// Put into the composer to send at, ms (`None`: not sent yet).
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub sent_ms: Option<u64>,
}

/// Why a comment operation failed.
#[derive(Debug, thiserror::Error)]
pub enum LineCommentError {
    /// Empty or too long text, a full store.
    #[error("{0}")]
    Invalid(String),
    /// No comment with this id.
    #[error("no comment with this id")]
    NotFound,
    /// The store file could not be written.
    #[error("the comments could not be saved: {0}")]
    Io(String),
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    version: u32,
    comments: Vec<LineComment>,
}

/// The file name stem of a session (first 16 bytes of SHA-256 of a domain tag and the id, hex).
#[must_use]
pub fn session_key(session_id: &str) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(b"vibe-work-line-comments\0");
    hasher.update(session_id.as_bytes());
    hasher.finalize()[..16]
        .iter()
        .fold(String::with_capacity(32), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// The line comments of every session, in `dir`.
pub struct LineCommentStore {
    dir: PathBuf,
    lock: Mutex<()>,
}

impl LineCommentStore {
    /// A store in `dir` (created user-only on first write).
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
        }
    }

    fn file(&self, session_id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", session_key(session_id)))
    }

    fn load(&self, session_id: &str) -> File {
        let path = self.file(session_id);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return File::default();
        };
        if let Ok(file) = serde_json::from_str::<File>(&text) {
            return file;
        }
        let aside = vw_platform::set_aside(&path, now_ms());
        tracing::warn!(file = ?aside, "line comments file did not parse: moved aside");
        File::default()
    }

    fn save(&self, session_id: &str, file: &File) -> Result<(), LineCommentError> {
        vw_platform::ensure_private_dir(&self.dir)
            .map_err(|e| LineCommentError::Io(e.to_string()))?;
        let bytes =
            serde_json::to_vec_pretty(file).map_err(|e| LineCommentError::Io(e.to_string()))?;
        vw_platform::replace_private(&self.file(session_id), &bytes)
            .map_err(|e| LineCommentError::Io(e.to_string()))
    }

    /// The session's comments, oldest first.
    #[must_use]
    pub fn list(&self, session_id: &str) -> Vec<LineComment> {
        let _guard = self.lock.lock();
        self.load(session_id).comments
    }

    /// Adds a comment on `lines` (already taken from the diff) at `anchor`. Returns it.
    ///
    /// # Errors
    /// [`LineCommentError::Invalid`] for empty or oversized text or a full store; [`LineCommentError::Io`].
    pub fn add(
        &self,
        session_id: &str,
        path: &str,
        lines: Vec<DiffLine>,
        anchor: LineAnchor,
        text: &str,
    ) -> Result<LineComment, LineCommentError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(LineCommentError::Invalid("the comment is empty".to_owned()));
        }
        if text.chars().count() > MAX_TEXT {
            return Err(LineCommentError::Invalid(format!(
                "a comment can have at most {MAX_TEXT} characters"
            )));
        }
        let _guard = self.lock.lock();
        let mut file = self.load(session_id);
        if file.comments.len() >= MAX_PER_SESSION {
            return Err(LineCommentError::Invalid(
                "this session has too many comments; delete some first".to_owned(),
            ));
        }
        let created_ms = now_ms();
        let id = format!(
            "l{}",
            crate::parse::fnv_hex(&[
                session_id.as_bytes(),
                path.as_bytes(),
                text.as_bytes(),
                &created_ms.to_le_bytes(),
                &file.comments.len().to_le_bytes(),
                &std::process::id().to_le_bytes(),
            ])
        );
        let comment = LineComment {
            id,
            path: path.to_owned(),
            lines,
            anchor,
            text: text.to_owned(),
            created_ms,
            sent_ms: None,
        };
        file.version = 1;
        file.comments.push(comment.clone());
        self.save(session_id, &file)?;
        Ok(comment)
    }

    /// Deletes comment `id`.
    ///
    /// # Errors
    /// [`LineCommentError::NotFound`], [`LineCommentError::Io`].
    pub fn delete(&self, session_id: &str, id: &str) -> Result<(), LineCommentError> {
        let _guard = self.lock.lock();
        let mut file = self.load(session_id);
        let before = file.comments.len();
        file.comments.retain(|c| c.id != id);
        if file.comments.len() == before {
            return Err(LineCommentError::NotFound);
        }
        self.save(session_id, &file)
    }

    /// Marks comments as put into the composer now. Returns them.
    ///
    /// # Errors
    /// [`LineCommentError::NotFound`] when none of `ids` exists; [`LineCommentError::Io`].
    pub fn mark_sent(
        &self,
        session_id: &str,
        ids: &[String],
    ) -> Result<Vec<LineComment>, LineCommentError> {
        let _guard = self.lock.lock();
        let mut file = self.load(session_id);
        let now = now_ms();
        let mut marked = Vec::new();
        for comment in &mut file.comments {
            if ids.contains(&comment.id) {
                comment.sent_ms = Some(now);
                marked.push(comment.clone());
            }
        }
        if marked.is_empty() {
            return Err(LineCommentError::NotFound);
        }
        self.save(session_id, &file)?;
        Ok(marked)
    }

    /// Moves session `old`'s comments to `new` (a session id hand-off): they are appended after any
    /// comments `new` already has, and `old`'s file is removed. Returns how many moved.
    ///
    /// # Errors
    /// [`LineCommentError::Io`] when `new`'s file cannot be written (`old`'s then stays as it was).
    pub fn move_session(&self, old: &str, new: &str) -> Result<usize, LineCommentError> {
        let _guard = self.lock.lock();
        let from = self.load(old);
        if from.comments.is_empty() {
            return Ok(0);
        }
        let moved = from.comments.len();
        let mut to = self.load(new);
        to.version = 1;
        to.comments.extend(from.comments);
        self.save(new, &to)?;
        let _ = std::fs::remove_file(self.file(old));
        Ok(moved)
    }

    /// Removes everything stored for a deleted session (a missing file is fine).
    pub fn remove_session(&self, session_id: &str) {
        let _guard = self.lock.lock();
        let _ = std::fs::remove_file(self.file(session_id));
    }
}
