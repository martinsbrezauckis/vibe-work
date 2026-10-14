//! Project handbook settings, errors and the checked section of a pick: the types the service, the core and the UI
//! share (split out of `service` to keep files short).
//!
//! Depends on: vw-preview (engine errors), `comments`, `paths`. Called by: `service`, `picking`, vw-core-api.

use std::path::PathBuf;
use std::time::Duration;

use vw_preview::EngineError;

/// Settings.
#[derive(Clone, Debug)]
pub struct PagesConfig {
    /// App-data folder for comments (`comments/`) and settings (`projects/`), user-only.
    pub data_dir: PathBuf,
    /// The viewer's website data folder.
    pub view_storage: PathBuf,
    /// Longest engine operation. Default 10 s.
    pub engine_timeout: Duration,
    /// Longest wait for a click in a pick. Default 60 s.
    pub pick_timeout: Duration,
    /// How often a pick is read. Default 200 ms.
    pub pick_poll: Duration,
    /// Longest wait for a page load after `open`. Default 5 s.
    pub load_wait: Duration,
    /// Label prefix of this service's viewer webview. Default [`crate::LABEL_PREFIX`] (the Project tab); the file
    /// preview runs a second service with [`FILES_LABEL_PREFIX`] so the two webviews never share a
    /// label. Must start with `vw-preview-` (the app's navigation guard and capability rely on it).
    pub label_prefix: &'static str,
    /// The service shows PDFs too (the File tab): its servers answer `.pdf` and the viewer tracks a PDF
    /// as the shown file. Off for the Project tab.
    pub pdf: bool,
}

/// Label prefix of the file preview's viewer webview.
pub const FILES_LABEL_PREFIX: &str = "vw-preview-files-";

impl PagesConfig {
    /// Defaults with the two folders.
    #[must_use]
    pub fn new(data_dir: PathBuf, view_storage: PathBuf) -> Self {
        Self {
            data_dir,
            view_storage,
            engine_timeout: Duration::from_secs(10),
            pick_timeout: Duration::from_secs(60),
            pick_poll: Duration::from_millis(200),
            load_wait: Duration::from_secs(5),
            label_prefix: crate::LABEL_PREFIX,
            pdf: false,
        }
    }

    /// The File tab's viewer: [`FILES_LABEL_PREFIX`] and PDFs on.
    #[must_use]
    pub fn for_files(data_dir: PathBuf, view_storage: PathBuf) -> Self {
        Self {
            label_prefix: FILES_LABEL_PREFIX,
            pdf: true,
            ..Self::new(data_dir, view_storage)
        }
    }
}

/// Why a Pages operation failed.
#[derive(Debug, thiserror::Error)]
pub enum PagesError {
    /// The project has no handbook folder yet.
    #[error(
        "this project has no handbook yet ({0}/); Vibe creates it when it writes the first page"
    )]
    NoPagesFolder(String),
    /// A bad page path.
    #[error(transparent)]
    Path(#[from] crate::paths::PathError),
    /// The page does not exist.
    #[error("no page {0} in the handbook folder")]
    NoPage(String),
    /// Nothing of this session is shown in the viewer.
    #[error("this session's page is not shown in the Project tab (open the Project tab first)")]
    NotShown,
    /// A pick ended without anything picked.
    #[error("nothing was picked (Escape, the page changed, or no click within the time limit)")]
    NothingPicked,
    /// The picked element is not inside a section of the page file.
    #[error(
        "the picked part of the page is not inside a section with an id; pick a part of a section"
    )]
    NoSection,
    /// The engine failed.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// A comment operation failed.
    #[error(transparent)]
    Comment(#[from] crate::comments::CommentError),
    /// An answer operation failed (Format 1.2 inputs).
    #[error(transparent)]
    Answer(#[from] crate::answers::AnswerError),
    /// Anything else (plain words).
    #[error("{0}")]
    Other(String),
}

/// A section picked in a page, checked against the page file.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PickedSection {
    /// The page.
    pub page: String,
    /// The section id (exists in the file).
    pub section: String,
    /// Its label (from the file).
    pub label: String,
    /// The selected text, if the pick came from a selection.
    pub quote: Option<String>,
    /// The picked element's own text (untrusted page text).
    pub text: String,
}
