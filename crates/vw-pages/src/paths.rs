//! Page paths: a page is named by its path relative to the project's handbook folder (`plans/verbose.html`), always
//! with `/`, never leaving the folder, never hidden. Also the file kinds the page server may serve.
//!
//! Depends on: serde. Called by: `page`, `links`, `hub`, `server`, `comments`, `service`, vw-core-api.
//! Invariants:
//! - A [`PagePath`] has 1-8 segments, none empty, `.`, `..`, starting with `.`, containing `\`, `:`, NUL or a
//!   control character; at most 1,024 bytes. It is a page (`.html`, `.htm`, `.md`).
//! - [`asset_allowed`] is a positive list: everything else is not served.
//! - [`PagePath::viewable`] additionally accepts `.pdf` for the file preview's viewer, whose server
//!   also serves `.pdf` (`PagesServer::start_for_files`); the Project tab's server does not.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Most segments in a page path.
pub const MAX_DEPTH: usize = 8;
/// Longest page path in bytes.
pub const MAX_LEN: usize = 1024;
/// The default handbook folder, relative to the project folder (Handbook Format 1.2 §2).
pub const DEFAULT_FOLDER: &str = crate::handbook::format::FOLDER;
/// The manifest and table of contents (§3, §8), written by the agent.
pub const INDEX: &str = "index.html";

/// Why a path is refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// Empty, too long or too deep.
    #[error("the page path is empty, too long or too deep")]
    Shape,
    /// A segment that could leave the folder or name a hidden file.
    #[error("the page path has a segment that is not allowed ({0})")]
    Segment(String),
    /// Not a page file.
    #[error("only .html, .htm and .md files are pages")]
    NotAPage,
}

/// A checked page path (see the module invariants).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PagePath(String);

impl PagePath {
    /// Checks `text` (a leading `/` or `./` is ignored).
    ///
    /// # Errors
    /// [`PathError`] when the path is not a safe page path.
    pub fn parse(text: &str) -> Result<Self, PathError> {
        let joined = clean_relative(text)?;
        if !is_page_name(&joined) {
            return Err(PathError::NotAPage);
        }
        Ok(Self(joined))
    }

    /// Checks a file the file preview shows in the viewer: the same path rules, but `.pdf` is accepted
    /// besides the page names (the engine's own PDF viewer shows it).
    ///
    /// # Errors
    /// [`PathError`] when the path is not a safe path or not a page or PDF.
    pub fn viewable(text: &str) -> Result<Self, PathError> {
        let joined = clean_relative(text)?;
        let pdf = Path::new(&joined)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        if !is_page_name(&joined) && !pdf {
            return Err(PathError::NotAPage);
        }
        Ok(Self(joined))
    }

    /// The text (`plans/x.html`).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The file under `root`.
    #[must_use]
    pub fn under(&self, root: &Path) -> PathBuf {
        self.0.split('/').fold(root.to_path_buf(), |p, s| p.join(s))
    }

    /// The folder part (`plans` for `plans/x.html`, "" at the top).
    #[must_use]
    pub fn folder(&self) -> &str {
        self.0.rsplit_once('/').map_or("", |(dir, _)| dir)
    }

    /// The first segment when the page is in a folder (`plans`), else `None`.
    #[must_use]
    pub fn top_folder(&self) -> Option<&str> {
        self.0.split_once('/').map(|(top, _)| top)
    }

    /// True for Markdown pages.
    #[must_use]
    pub fn is_markdown(&self) -> bool {
        self.0.to_ascii_lowercase().ends_with(".md")
    }
}

impl fmt::Display for PagePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for PagePath {
    type Error = PathError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<PagePath> for String {
    fn from(value: PagePath) -> Self {
        value.0
    }
}

/// Normalizes a relative path (`/`-separated, see the module invariants) without the page-extension rule.
///
/// # Errors
/// [`PathError::Shape`] or [`PathError::Segment`].
pub fn clean_relative(text: &str) -> Result<String, PathError> {
    let trimmed = text.trim().trim_start_matches("./").trim_start_matches('/');
    if trimmed.is_empty() || trimmed.len() > MAX_LEN {
        return Err(PathError::Shape);
    }
    let segments: Vec<&str> = trimmed.split('/').collect();
    if segments.len() > MAX_DEPTH {
        return Err(PathError::Shape);
    }
    for segment in &segments {
        if !segment_allowed(segment) {
            return Err(PathError::Segment(segment.chars().take(40).collect()));
        }
    }
    Ok(segments.join("/"))
}

/// One path segment is allowed: not empty, not `.`/`..`, not hidden, no `\`, `:`, NUL or control characters.
#[must_use]
pub fn segment_allowed(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.starts_with('.')
        && segment.len() <= 255
        && !segment
            .chars()
            .any(|c| c == '\\' || c == ':' || c.is_control())
}

/// True for page file names (`.html`, `.htm`, `.md`, any case).
#[must_use]
pub fn is_page_name(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|e| {
        e.eq_ignore_ascii_case("html")
            || e.eq_ignore_ascii_case("htm")
            || e.eq_ignore_ascii_case("md")
    })
}

/// The MIME type for a file the page server may serve, or `None` when it is not served.
#[must_use]
pub fn asset_allowed(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "html" | "htm" | "md" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => return None,
    })
}
