//! The documents part of the command API: the requests both clients send inside
//! `Command::Documents`, what comes back, and the facts of one document as the UI sees them.
//!
//! Depends on: serde, vw-documents (the facts are converted from its `DocumentInfo`). Called by:
//! `command`, `event`, `documents` (the handler), the UI through the generated TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - **No operation adds a file.** Files come only from the native picker or an OS drop, handled in
//!   Rust (`Core::add_documents`); no request carries a path or file bytes, so neither the webview nor
//!   the Debug API can make the app read a file of its choosing (design §3).
//! - No request carries a caller-supplied path. `save` resolves a known session's cwd in core and
//!   returns the attachment path after its bytes are safely written there.

use serde::{Deserialize, Serialize};
use vw_documents::{DocStatus, DocumentInfo, KeptUnit, ReadMethod};

/// One request about documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DocumentsRequest {
    /// Every document added and not yet removed.
    List {},
    /// One document's facts and, once read, its text.
    Read {
        /// The document.
        id: String,
    },
    /// Saves original bytes under the project folder of a known session, returning an absolute
    /// path for model context. The client supplies an id, never a filesystem path.
    Save {
        /// Document to save.
        id: String,
        /// Session whose project folder owns the attachment.
        session_id: String,
    },
    /// Reads an OCR document with Mistral OCR (spends the user's OCR quota). From the Debug API origin
    /// the window asks the native confirmation (the file leaves the computer); the Debug API's is answered by the core.
    Extract {
        /// The document.
        id: String,
    },
    /// Removes a document (sent, or the user removed the chip).
    Remove {
        /// The document.
        id: String,
    },
    /// Adds files dropped on a composer: the window read the bytes of the files the user dropped (Tauri's
    /// own drop handling is disabled, so no path exists) and sends them here; the core runs the same kind
    /// detection and limits as for the picker. The Debug API may send it too (it acts as the user).
    AddDropped {
        /// The files.
        files: Vec<DroppedFile>,
    },
    /// The bytes of a PNG, JPEG, WebP or GIF document, for the composer's picture path ("Attach as
    /// picture" on a dropped image). The gate checks the picture again when it is sent.
    TakePicture {
        /// The document.
        id: String,
    },
}

/// One file dropped on a composer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DroppedFile {
    /// The file's name as the drop gave it (only the last segment is kept).
    pub name: String,
    /// The bytes, base64 (at most 50 MB decoded).
    pub data: String,
}

/// What a documents request answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DocumentsResult {
    /// `list`.
    List {
        /// Oldest first.
        documents: Vec<DocumentView>,
    },
    /// `read` and `extract`.
    Document {
        /// The facts.
        document: DocumentView,
        /// The text, once ready.
        text: Option<String>,
    },
    /// `save` completed; this absolute path can be supplied to the agent.
    Saved {
        /// The path of the original file in the session project.
        path: String,
    },
    /// `addDropped`.
    Added {
        /// Added documents.
        added: Vec<DocumentView>,
        /// Files not added, with the reason.
        refused: Vec<DocumentRefusal>,
    },
    /// `remove`.
    Removed {
        /// False when there was no such document.
        removed: bool,
    },
    /// `takePicture`.
    Picture {
        /// `image/png`, `image/jpeg`, `image/webp` or `image/gif`.
        mime_type: String,
        /// The bytes, base64.
        data: String,
        /// The file name.
        name: String,
    },
}

/// The facts of one document (no text or path; `save` returns the latter separately).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DocumentView {
    /// Random id.
    pub id: String,
    /// File name (no folder).
    pub name: String,
    /// `pdf`, `docx`, `pptx`, `xlsx`, `csv`, `text`, image kinds, or `binary`.
    pub kind: String,
    /// "PDF", "Word document", ...
    pub kind_label: String,
    /// `ocr` (Mistral OCR on request), `local` (read here), or `file` (path only).
    pub method: String,
    /// File size in bytes (at most 50 MB).
    pub size: u32,
    /// Pages counted before sending; `null`: unknown or not paged.
    pub pages: Option<u32>,
    /// `waiting`, `reading`, `ready` or `failed`.
    pub status: String,
    /// Characters of the text once ready.
    pub chars: Option<u32>,
    /// What was kept when the text was over the limit.
    pub kept: Option<KeptView>,
    /// The OCR model Mistral used.
    pub model: Option<String>,
    /// Pages Mistral says it processed.
    pub pages_read: Option<u32>,
    /// Why reading failed.
    pub error: Option<String>,
    /// Can go into the message as a picture instead.
    pub can_attach_as_picture: bool,
}

/// What part of a long document was kept (the rest was not sent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct KeptView {
    /// `pages` or `lines`.
    pub unit: KeptUnitView,
    /// Whole units kept from the start (0: part of the first one).
    pub kept: u32,
    /// Units in the document.
    pub of: u32,
}

/// The unit of [`KeptView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum KeptUnitView {
    /// OCR pages.
    Pages,
    /// Lines of text.
    Lines,
}

/// A file that was not added, with the reason in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DocumentRefusal {
    /// The file name (no folder).
    pub name: String,
    /// Why.
    pub reason: String,
}

/// How files arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DocumentSource {
    /// The native file picker (`pick_documents`).
    Picker,
    /// Dropped on the window (handled by the OS and Rust; off since set `dragDropEnabled: false`).
    Drop,
    /// Dropped on a composer; the window read the bytes (`addDropped`). The caller places the chips.
    Composer,
}

fn to_value<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

impl From<&DocumentInfo> for DocumentView {
    fn from(info: &DocumentInfo) -> Self {
        Self {
            id: info.id.clone(),
            name: info.name.clone(),
            kind: to_value(&info.kind),
            kind_label: info.kind.label().to_owned(),
            method: match info.method {
                ReadMethod::Ocr => "ocr",
                ReadMethod::Local => "local",
                ReadMethod::File => "file",
            }
            .to_owned(),
            size: u32::try_from(info.size).unwrap_or(u32::MAX),
            pages: info.pages,
            status: match info.status {
                DocStatus::Waiting => "waiting",
                DocStatus::Reading => "reading",
                DocStatus::Ready => "ready",
                DocStatus::Failed => "failed",
            }
            .to_owned(),
            chars: info.chars,
            kept: info.kept.map(|k| KeptView {
                unit: match k.unit {
                    KeptUnit::Pages => KeptUnitView::Pages,
                    KeptUnit::Lines => KeptUnitView::Lines,
                },
                kept: k.kept,
                of: k.of,
            }),
            model: info.model.clone(),
            pages_read: info.pages_read,
            error: info.error.clone(),
            can_attach_as_picture: info.can_attach_as_picture,
        }
    }
}
