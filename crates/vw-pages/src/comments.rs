//! Page comments (`PROJECT_PAGES` "One source, many views"): the user's private working notes on page sections,
//! kept in Vibe Work's own data, never in the repository. Keyed by project folder + page path + section id; a
//! comment whose section (or page) is gone is listed as detached, never dropped.
//!
//! Depends on: serde, sha2, getrandom, vw-platform (user-only files), `paths`. Called by: `service`, vw-core-api.
//! Invariants:
//! - One JSON file per project: `<dir>/<project key>.json`, user-only, replaced atomically (written to a new
//!   private file, then renamed over the old one).
//! - A file that does not parse is moved aside (`.json.bad-<ms>`) and the project starts with no comments; it is
//!   never overwritten in place.
//! - Comment text at most [`MAX_TEXT`] characters, quotes at most [`MAX_QUOTE`], [`MAX_PER_PROJECT`] comments.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::PagePath;

/// Longest comment text.
pub const MAX_TEXT: usize = 4000;
/// Longest quoted selection kept with a comment.
pub const MAX_QUOTE: usize = 2000;
/// Most comments per project.
pub const MAX_PER_PROJECT: usize = 5000;

/// One comment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    /// Random id.
    pub id: String,
    /// The page (path inside the pages folder).
    pub page: String,
    /// The section id it is on.
    pub section: String,
    /// The section's label when the comment was written (shown for detached comments).
    pub section_label: String,
    /// The text the user had selected, if any.
    pub quote: Option<String>,
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
pub enum CommentError {
    /// Empty or too long text.
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
    comments: Vec<Comment>,
}

/// The key of a project folder (first 16 bytes of SHA-256 of a domain tag and the folder path, hex).
#[must_use]
pub fn project_key(project: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"vibe-work-pages\0");
    hasher.update(project.to_string_lossy().as_bytes());
    hex(&hasher.finalize()[..16])
}

/// Lowercase hex of `bytes`.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn random_id() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        return format!("c{}", now_ms());
    }
    format!("c{}", hex(&bytes))
}

/// The comments of every project, in `dir`.
pub struct CommentStore {
    dir: PathBuf,
    lock: Mutex<()>,
}

impl CommentStore {
    /// A store in `dir` (created user-only on first write).
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
        }
    }

    fn file(&self, project: &Path) -> PathBuf {
        self.dir.join(format!("{}.json", project_key(project)))
    }

    fn load(&self, project: &Path) -> File {
        let path = self.file(project);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return File::default();
        };
        if let Ok(file) = serde_json::from_str::<File>(&text) {
            return file;
        }
        let aside = vw_platform::set_aside(&path, now_ms());
        tracing::warn!(file = ?aside, "page comments file did not parse: moved aside");
        File::default()
    }

    fn save(&self, project: &Path, file: &File) -> Result<(), CommentError> {
        vw_platform::ensure_private_dir(&self.dir).map_err(|e| CommentError::Io(e.to_string()))?;
        let bytes = serde_json::to_vec_pretty(file).map_err(|e| CommentError::Io(e.to_string()))?;
        vw_platform::replace_private(&self.file(project), &bytes)
            .map_err(|e| CommentError::Io(e.to_string()))
    }

    /// The comments on `page`, split into attached (section exists in `sections`) and detached (section gone, or
    /// `sections` is `None` because the page is gone), each oldest first.
    #[must_use]
    pub fn list(
        &self,
        project: &Path,
        page: &PagePath,
        sections: Option<&[String]>,
    ) -> (Vec<Comment>, Vec<Comment>) {
        let _guard = self.lock.lock();
        let file = self.load(project);
        file.comments
            .into_iter()
            .filter(|c| c.page == page.as_str())
            .partition(|c| sections.is_some_and(|s| s.contains(&c.section)))
    }

    /// How many comments each page of the project has.
    #[must_use]
    pub fn counts(&self, project: &Path) -> std::collections::HashMap<String, u32> {
        let _guard = self.lock.lock();
        let mut counts = std::collections::HashMap::new();
        for comment in self.load(project).comments {
            *counts.entry(comment.page).or_insert(0) += 1;
        }
        counts
    }

    /// Adds a comment. Returns it.
    ///
    /// # Errors
    /// [`CommentError::Invalid`] for empty or oversized text, a full store; [`CommentError::Io`].
    pub fn add(
        &self,
        project: &Path,
        page: &PagePath,
        section: (&str, &str),
        text: &str,
        quote: Option<&str>,
    ) -> Result<Comment, CommentError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CommentError::Invalid("the comment is empty".to_owned()));
        }
        if text.chars().count() > MAX_TEXT {
            return Err(CommentError::Invalid(format!(
                "a comment can have at most {MAX_TEXT} characters"
            )));
        }
        let _guard = self.lock.lock();
        let mut file = self.load(project);
        if file.comments.len() >= MAX_PER_PROJECT {
            return Err(CommentError::Invalid(
                "this project has too many comments; delete some first".to_owned(),
            ));
        }
        let comment = Comment {
            id: random_id(),
            page: page.to_string(),
            section: section.0.chars().take(100).collect(),
            section_label: section.1.chars().take(120).collect(),
            quote: quote
                .map(|q| q.trim().chars().take(MAX_QUOTE).collect::<String>())
                .filter(|q| !q.is_empty()),
            text: text.to_owned(),
            created_ms: now_ms(),
            sent_ms: None,
        };
        file.version = 1;
        file.comments.push(comment.clone());
        self.save(project, &file)?;
        Ok(comment)
    }

    /// Deletes comment `id`.
    ///
    /// # Errors
    /// [`CommentError::NotFound`], [`CommentError::Io`].
    pub fn delete(&self, project: &Path, id: &str) -> Result<(), CommentError> {
        let _guard = self.lock.lock();
        let mut file = self.load(project);
        let before = file.comments.len();
        file.comments.retain(|c| c.id != id);
        if file.comments.len() == before {
            return Err(CommentError::NotFound);
        }
        self.save(project, &file)
    }

    /// Marks comments as put into the composer now. Returns them.
    ///
    /// # Errors
    /// [`CommentError::NotFound`] when none of `ids` exists; [`CommentError::Io`].
    pub fn mark_sent(&self, project: &Path, ids: &[String]) -> Result<Vec<Comment>, CommentError> {
        let _guard = self.lock.lock();
        let mut file = self.load(project);
        let now = now_ms();
        let mut marked = Vec::new();
        for comment in &mut file.comments {
            if ids.contains(&comment.id) {
                comment.sent_ms = Some(now);
                marked.push(comment.clone());
            }
        }
        if marked.is_empty() {
            return Err(CommentError::NotFound);
        }
        self.save(project, &file)?;
        Ok(marked)
    }

    /// One comment by id.
    #[must_use]
    pub fn get(&self, project: &Path, id: &str) -> Option<Comment> {
        let _guard = self.lock.lock();
        self.load(project).comments.into_iter().find(|c| c.id == id)
    }
}
