//! The File tab's part of the command API: the requests both clients send inside `Command::Files`, and
//! what comes back. Rules in `files_scope` (which paths may be shown) and `files_read` (what a file is shown as);
//! the handler is `files`.
//!
//! Depends on: vw-pages (`ViewerView`), vw-preview (`Rect`), `files_read` (`FileKind`), serde. Called by: `command`,
//! `files`, the UI through the generated TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - Only `resolve` takes path TEXT (from the chat); every other request names a file by the opaque `file` handle
//!   `resolve` returned, and the core checks the file against the caller's scope again each time.
//! - The Debug API acts as the user: same scope, same answers, no dialogs. `open` of an HTML
//!   or SVG file asks first (the browser runs its scripts); for the Debug API the core's confirmer answers itself.

use serde::{Deserialize, Serialize};
use vw_pages::ViewerView;
use vw_preview::Rect;

use crate::files_read::FileKind;

/// Most paths in one `resolve`.
pub const MAX_RESOLVE: usize = 200;

/// One File tab request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum FilesRequest {
    /// Which of these paths (from the chat) name a file the app may show: relative to the session's folder or
    /// absolute, inside the session's folder or a picked folder. At most [`MAX_RESOLVE`].
    Resolve {
        /// The session.
        session_id: String,
        /// Path texts as the chat wrote them (`src/main.rs`, `/home/u/p/a.md`, `file:///...`, `~/p/x`).
        paths: Vec<String>,
    },
    /// The file's kind, size and content (text head, image) for the File tab.
    Read {
        /// The session.
        session_id: String,
        /// The handle from `resolve`.
        file: String,
    },
    /// Whether the file still exists and its fingerprint (the File tab's live reload).
    Stat {
        /// The session.
        session_id: String,
        /// The handle from `resolve`.
        file: String,
    },
    /// Shows an HTML page or a PDF in the isolated viewer (the page server of the file's folder).
    View {
        /// The session.
        session_id: String,
        /// The handle from `resolve`.
        file: String,
    },
    /// Where the window shows the viewer (absent: hide it).
    Layout {
        /// The shown session (absent: hide).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
        /// The viewer's area in the main window's CSS pixels (absent: hide).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        rect: Option<Rect>,
    },
    /// Reloads the viewer.
    Reload {
        /// The session.
        session_id: String,
    },
    /// The session's viewer state.
    Viewer {
        /// The session.
        session_id: String,
    },
    /// Forgets the last link the viewed page tried to open.
    ClearExternal {
        /// The session.
        session_id: String,
    },
    /// A screenshot of the viewer showing the session.
    Screenshot {
        /// The session.
        session_id: String,
    },
    /// Opens the file with the user's default app (never a program; HTML/SVG after a confirmation).
    Open {
        /// The session.
        session_id: String,
        /// The handle from `resolve`.
        file: String,
    },
    /// Shows the file in the file manager.
    Reveal {
        /// The session.
        session_id: String,
        /// The handle from `resolve`.
        file: String,
    },
    /// Saves an edited text file (`files_write.rs`): only a text file read whole, only when it still has
    /// the `fingerprint` the tab read (else `stale`); line endings and a byte-order mark are kept.
    Write {
        /// The session.
        session_id: String,
        /// The handle from `resolve`.
        file: String,
        /// The whole new text.
        text: String,
        /// The fingerprint of the content the edit started from.
        fingerprint: String,
    },
}

impl FilesRequest {
    /// The session the request is about (`layout` may name none).
    #[must_use]
    pub fn session(&self) -> Option<&str> {
        match self {
            Self::Layout { session_id, .. } => session_id.as_deref(),
            Self::Resolve { session_id, .. }
            | Self::Read { session_id, .. }
            | Self::Stat { session_id, .. }
            | Self::View { session_id, .. }
            | Self::Reload { session_id }
            | Self::Viewer { session_id }
            | Self::ClearExternal { session_id }
            | Self::Screenshot { session_id }
            | Self::Open { session_id, .. }
            | Self::Reveal { session_id, .. }
            | Self::Write { session_id, .. } => Some(session_id),
        }
    }
}

/// One path of a `resolve`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FileRefView {
    /// The text as sent.
    pub text: String,
    /// One file may be shown directly. Without choices, false keeps missing and refused paths indistinguishable.
    pub ok: bool,
    /// The handle for the other requests.
    pub file: Option<String>,
    /// How to name it: relative to the session's folder, else the full path (a picked folder).
    pub display: Option<String>,
    /// The kind the name suggests (`read` decides from the bytes).
    pub kind: Option<FileKind>,
    /// Size in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub size: Option<u64>,
    /// Several allowed files share the name; the user must choose. Absent for a single hit or a refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub choices: Option<Vec<FileChoice>>,
    /// Full count when more than 50 choices passed the scope check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub total: Option<usize>,
}

/// One allowed file in a reference chooser, relative to the session folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FileChoice {
    /// Session-bound handle, rechecked before every use.
    pub file: String,
    /// Path relative to the session folder.
    pub display: String,
    /// Kind suggested by the file name.
    pub kind: FileKind,
    /// Size in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub size: u64,
}

/// A file's content for the File tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    /// The handle.
    pub file: String,
    /// As in [`FileRefView::display`].
    pub display: String,
    /// The file name.
    pub name: String,
    /// The kind, from the bytes.
    pub kind: FileKind,
    /// Size in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub size: u64,
    /// Changes when the file does.
    pub fingerprint: String,
    /// Text (text, Markdown, HTML source, SVG source): the start of the file, at most 256 KiB.
    pub text: Option<String>,
    /// `text` is only the start of the file.
    pub truncated: bool,
    /// An image as a `data:` URL.
    pub data_url: Option<String>,
    /// An image too large to show.
    pub too_large: bool,
    /// `passive` (opens directly), `active` (asks first: a browser runs it) or `refused` (never opened).
    pub open_class: String,
}

/// What a File tab request answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FilesResult {
    /// `resolve`: one entry per path, in order.
    Resolved {
        /// The answers.
        refs: Vec<FileRefView>,
    },
    /// `read`.
    Content {
        /// The file.
        content: FileContent,
    },
    /// `stat`.
    Stat {
        /// The file is still there (and still allowed).
        exists: bool,
        /// Its fingerprint, when it exists.
        fingerprint: Option<String>,
    },
    /// `view`, `reload`, `viewer`, `clearExternal`.
    Viewer {
        /// The session's viewer, when it has one.
        viewer: Option<ViewerView>,
    },
    /// `screenshot`.
    Screenshot {
        /// PNG, base64.
        png_base64: String,
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// `layout`, `open`, `reveal`.
    Done {},
    /// `write`.
    Written {
        /// The file's fingerprint now.
        fingerprint: String,
        /// Its size now.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        size: u64,
    },
}
