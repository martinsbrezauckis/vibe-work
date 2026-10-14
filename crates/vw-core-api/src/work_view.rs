//! The Libraries and image-generation parts of the command API: the requests both clients send inside
//! `Command::Libraries` and `Command::Images`, what comes back, and the facts the UI shows.
//!
//! Depends on: serde, vw-mistral (library facts). Called by: `command`, `work` (the handlers), the UI through the
//! generated TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - No request carries a path. Files reach a library either from the native picker (Rust side,
//!   `Core::upload_library_paths`) or as bytes the client already holds (`upload`), like a drop.
//! - Document text, titles, summaries and answers are the user's file content (possibly hostile): the UI renders
//!   them as text.

use serde::{Deserialize, Serialize};

/// Who may use a library from a chat: nobody (it is only listed), every session, or the sessions of one project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum LibraryScope {
    /// Listed here only; the agent does not search it (libraries made elsewhere start like this).
    Off {},
    /// Every session's agent may search it.
    Global {},
    /// Only sessions of this project (project id).
    Project {
        /// The project.
        project_id: String,
    },
}

/// One file the client already holds (a drop on the Libraries page, or a Debug API client).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryFile {
    /// The file name (only the last segment is kept).
    pub name: String,
    /// The bytes, base64 (at most 50 MB decoded).
    pub data: String,
}

/// One request about Libraries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum LibrariesRequest {
    /// The libraries the key can see, with their scope and use facts.
    List {},
    /// Makes a library on Mistral.
    Create {
        /// Its name (1-100 characters).
        name: String,
        /// Optional description.
        description: Option<String>,
        /// Who may use it.
        scope: LibraryScope,
    },
    /// Changes who may use a library (stored in Vibe Work; Mistral is not called).
    SetScope {
        /// The library.
        library_id: String,
        /// The new scope.
        scope: LibraryScope,
    },
    /// Deletes a library and all its documents on Mistral (cannot be undone).
    Delete {
        /// The library.
        library_id: String,
    },
    /// The documents of a library.
    Documents {
        /// The library.
        library_id: String,
    },
    /// Uploads files the client holds (sends them to Mistral, which keeps and indexes them).
    Upload {
        /// The library.
        library_id: String,
        /// The files.
        files: Vec<LibraryFile>,
    },
    /// Deletes one document on Mistral.
    DeleteDocument {
        /// The library.
        library_id: String,
        /// The document.
        document_id: String,
    },
    /// The text Mistral extracted from a document (the "open source" view of a citation).
    Text {
        /// The library.
        library_id: String,
        /// The document.
        document_id: String,
    },
    /// Asks a question across libraries and answers with numbered sources.
    Ask {
        /// The question (1-4,000 characters).
        question: String,
        /// The libraries to search (empty: every library that is not off).
        library_ids: Vec<String>,
    },
}

/// One library as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LibraryView {
    /// Mistral's id.
    pub id: String,
    /// Its name (user text).
    pub name: String,
    /// Its description (user text).
    pub description: Option<String>,
    /// Documents in it.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub documents: u64,
    /// Their total size in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total_size: u64,
    /// Last change (ISO time).
    pub updated_at: String,
    /// Who may use it here.
    pub scope: LibraryScope,
}

/// One document as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LibraryDocumentView {
    /// Mistral's id.
    pub id: String,
    /// Its library.
    pub library_id: String,
    /// File name (user text).
    pub name: String,
    /// Size in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub size: Option<u64>,
    /// Mistral's processing state: `todo`, `in_progress`, `done`, `error`, ... (`done` = searchable).
    pub status: String,
    /// Pages, when known.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub pages: Option<u64>,
    /// Mistral's summary (file content: show as text).
    pub summary: Option<String>,
    /// Tokens processed during indexing.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub tokens: Option<u64>,
    /// Upload time (ISO).
    pub created_at: String,
}

/// One source of an answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LibrarySourceView {
    /// 1-based number.
    pub n: u32,
    /// The library it is in (`None` when it is no longer in any searched library).
    pub library_id: Option<String>,
    /// The document.
    pub document_id: String,
    /// Its title (file name; file content: show as text).
    pub title: String,
    /// The passage the search returned (file content: show as text).
    pub snippet: String,
}

/// A file that was not uploaded, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LibraryUploadRefusal {
    /// The file name.
    pub name: String,
    /// Why (plain sentence).
    pub reason: String,
}

/// What leaves the computer and the request limits for Libraries and images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkCosts {
    /// One sentence about what leaves the computer.
    pub leaves: String,
    /// One sentence about limits.
    pub limits: String,
}

/// What a Libraries request answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LibrariesResult {
    /// `list`.
    List {
        /// The libraries.
        libraries: Vec<LibraryView>,
        /// What uploading and asking sends, and the limits.
        costs: WorkCosts,
    },
    /// `create`, `setScope`.
    Library {
        /// The library now.
        library: LibraryView,
    },
    /// `delete`, `deleteDocument`.
    Deleted {},
    /// `documents`.
    Documents {
        /// The library.
        library_id: String,
        /// Its documents.
        documents: Vec<LibraryDocumentView>,
    },
    /// `upload` (and the picker's upload).
    Uploaded {
        /// Documents now in the library (processing on Mistral's side).
        added: Vec<LibraryDocumentView>,
        /// Files not uploaded.
        refused: Vec<LibraryUploadRefusal>,
    },
    /// `text`.
    Text {
        /// The document's name.
        name: String,
        /// Its extracted text (file content: show as text).
        text: String,
        /// True when the text was cut.
        truncated: bool,
    },
    /// `ask`.
    Answer {
        /// The written answer (citing titles in square brackets; file content: show as text).
        answer: String,
        /// The sources, numbered.
        sources: Vec<LibrarySourceView>,
        /// The searches it ran.
        queries: Vec<String>,
        /// The libraries searched.
        library_ids: Vec<String>,
        /// Tokens used (all kinds), as reported by the service.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        tokens: u64,
    },
}

/// One request about image generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ImagesRequest {
    /// The use facts and this run's request count (no switch; image generation asks nothing of its own).
    Read {},
}

/// What an images request answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ImagesResult {
    /// The use facts (shown in Settings).
    pub costs: WorkCosts,
    /// Image requests sent to Mistral in this run of the app (all sessions, including failed requests).
    pub made: u32,
}
