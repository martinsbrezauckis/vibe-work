//! Typed project instruction file requests and answers for the Projects screen.
//! Depends on serde and the document shape; called by the core dispatcher and generated UI client.
//! Invariant: names are relative allowlisted file names, never client supplied absolute paths.

use serde::{Deserialize, Serialize};

use crate::instructions_view::InstructionsDoc;

/// One allowed instruction file shown in a project's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectInstructionFile {
    /// `AGENTS.md` or a direct `docs/instructions/*.md` name.
    pub name: String,
    /// Whether the file exists.
    pub exists: bool,
    /// Whether AGENTS.md links to this file.
    pub linked: bool,
    /// Whether the file can be read and edited by this API.
    pub editable: bool,
}

/// A user action in one saved project's main folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProjectInstructionsRequest {
    /// List allowed files and the saved main folder identity.
    List {
        /// Saved project ID.
        project_id: String,
    },
    /// Read one allowed file.
    Read {
        /// Saved project ID.
        project_id: String,
        /// Relative allowlisted name.
        name: String,
    },
    /// Prepare an AGENTS.md draft that links or unlinks one instruction file; this never writes.
    LinkDraft {
        /// Saved project ID.
        project_id: String,
        /// Relative name of the extra file.
        name: String,
        /// True adds a link; false removes it.
        linked: bool,
    },
    /// Create an absent file on explicit Save after review.
    Create {
        /// Saved project ID.
        project_id: String,
        /// Relative allowlisted name.
        name: String,
        /// Folder shown at review time.
        expected_folder: String,
        /// Complete new text.
        text: String,
    },
    /// Replace a file only while its complete text still matches the reviewed version.
    Write {
        /// Saved project ID.
        project_id: String,
        /// Relative allowlisted name.
        name: String,
        /// Folder shown at review time.
        expected_folder: String,
        /// Complete new text.
        text: String,
        /// Complete text shown at review time.
        expected: String,
    },
    /// Delete a file after a native confirmation naming its full path, if its content is unchanged.
    Delete {
        /// Saved project ID.
        project_id: String,
        /// Relative allowlisted name.
        name: String,
        /// Folder shown at review time.
        expected_folder: String,
        /// Complete text shown at review time.
        expected: String,
    },
}

/// The result of a project instruction request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ProjectInstructionsResult {
    /// Files in the saved main folder.
    List {
        /// Saved main folder.
        folder: String,
        /// Root AGENTS.md and direct Markdown files in docs/instructions.
        files: Vec<ProjectInstructionFile>,
    },
    /// A file's current document, including its text and editability.
    Doc {
        /// Relative allowlisted name.
        name: String,
        /// Current file state.
        doc: InstructionsDoc,
    },
    /// A reviewed AGENTS.md draft with a link change.
    LinkDraft {
        /// Current AGENTS.md state.
        doc: InstructionsDoc,
        /// Proposed complete AGENTS.md text.
        draft: String,
    },
    /// A deleted file's relative name.
    Deleted {
        /// Relative allowlisted name.
        name: String,
    },
}
