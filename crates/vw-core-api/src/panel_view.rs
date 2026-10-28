//! What the window answers to the agent's `panel_read`: the side panel of one
//! session as plain typed fields: its tabs, the open editors (path, focus, unsaved text, selection, cursor line,
//! the split and the read-only saved side) and the Files tab (folder, view, search text).
//!
//! Depends on: serde. Called by: `files_view` (`FilesRequest::PanelAnswer`), `panel_read`, the UI through the
//! generated TypeScript.
//! Invariants: unknown fields are refused, so a window can only report these fields; nothing here can carry a
//! terminal's output, a browser page or any form field (those tabs are only named); `panel_read` bounds every text.

use serde::{Deserialize, Serialize};

/// One session's side panel, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelView {
    /// The side panel is open.
    pub open: bool,
    /// Its tabs by label, in order (`Files`, `Changes`, `Terminal`, `Browser`, a file's short name).
    pub tabs: Vec<String>,
    /// The tab shown, by label.
    pub active: Option<String>,
    /// The open editors.
    pub editors: Vec<PanelEditor>,
    /// The Files tab, when it is open.
    pub files: Option<PanelFiles>,
}

/// One open editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelEditor {
    /// The path the tab's tooltip shows (relative to the session folder when inside it).
    pub path: String,
    /// The editor has focus (it receives the next opened file).
    pub focused: bool,
    /// `left` or `right` while two editors are side by side.
    pub side: Option<String>,
    /// A read-only comparison side: `saved` (the last commit) or `disk`.
    pub saved_version: Option<String>,
    /// The editor holds edits not saved to disk.
    pub unsaved: bool,
    /// The editor's whole text, sent only when it differs from the file on disk (the user's own unsaved text).
    pub unsaved_text: Option<String>,
    /// The selected lines, when something is selected.
    pub selection: Option<PanelSelection>,
    /// The cursor's line (1-based).
    pub cursor_line: Option<u32>,
    /// Lines marked as changed on disk while the file was open.
    pub changed_lines: Vec<u32>,
}

/// A selection in an editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelSelection {
    /// First selected line (1-based).
    pub start_line: u32,
    /// Last selected line.
    pub end_line: u32,
    /// The selected text.
    pub text: String,
}

/// The Files tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelFiles {
    /// The folder it shows.
    pub folder: Option<String>,
    /// `files` or `history`.
    pub view: String,
    /// The text in its search field.
    pub search: String,
}
