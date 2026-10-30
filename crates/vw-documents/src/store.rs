//! The documents the user added and that are not in a message yet: the file's bytes (until read),
//! its facts (kind, size, pages), and its text once read. Original bytes remain until removal so
//! the core can save every accepted attachment into the session project.
//!
//! Life of a document: `add_path` (the native picker or an OS drop gave the path, in Rust) -> local
//! kinds are read at once (`ready`), OCR kinds wait for the caller to start automatic reading (`waiting`) -> `begin_read` hands the
//! bytes to the caller, which calls OCR -> `finish_read` stores the text (`ready`) or the reason
//! (`failed`, bytes kept for another try). `save_with` writes the original bytes once and keeps the
//! resulting path for the composer.
//!
//! Depends on: `kind`, `pages`, `local`, `assemble`, getrandom. Called by: vw-core-api (`documents`).
//! Invariants: nothing here takes a path or bytes from a client (only Rust callers); at most
//! [`MAX_DOCS`] documents and [`MAX_HELD_BYTES`] of original file bytes plus the text read from them are held
//! (a document whose text would pass the budget is not kept); a file over [`MAX_FILE_BYTES`] is refused before it is
//! read in full; ids are random.

use std::fmt::Write as _;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;

use crate::assemble::{Extracted, Kept, from_pages, from_text};
use crate::kind::{DocKind, KindError, ReadMethod, detect};
use crate::local::{LocalError, read_local};
use crate::pages::page_count;

/// Largest file accepted (Mistral OCR's limit, its FAQ).
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
/// Most documents held at once.
pub const MAX_DOCS: usize = 8;
/// Most bytes held at once: the original files and every text read from them (the message text and the complete
/// text kept for the saved reader).
pub const MAX_HELD_BYTES: u64 = 200 * 1024 * 1024;
/// Largest picture handed to the composer (the gate's image limit).
pub const MAX_PICTURE_BYTES: usize = 10 * 1024 * 1024;

/// Where a document is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DocStatus {
    /// An OCR kind waiting for automatic reading to start.
    Waiting,
    /// OCR is running.
    Reading,
    /// The text is ready.
    Ready,
    /// OCR failed (the bytes are kept: the user can try again).
    Failed,
}

/// Facts about one document (no text, no path).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    /// Random id (`doc-<hex>`).
    pub id: String,
    /// The file name (no folder).
    pub name: String,
    /// Detected kind.
    pub kind: DocKind,
    /// How it is read.
    pub method: ReadMethod,
    /// File size in bytes.
    pub size: u64,
    /// Pages counted before sending (`None`: unknown or not paged).
    pub pages: Option<u32>,
    /// Life stage.
    pub status: DocStatus,
    /// Characters of the text, once ready.
    pub chars: Option<u32>,
    /// What was kept when the text was over the limit.
    pub kept: Option<Kept>,
    /// The OCR model that read it (Mistral's answer).
    pub model: Option<String>,
    /// Pages Mistral says it processed.
    pub pages_read: Option<u32>,
    /// Why OCR failed.
    pub error: Option<String>,
    /// PNG, JPEG, WebP or GIF up to 10 MB: can go into the message as a picture instead.
    pub can_attach_as_picture: bool,
}

/// Why a file was not added.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddError {
    /// Over [`MAX_FILE_BYTES`].
    #[error("This file is larger than 50 MB, the most Mistral OCR accepts.")]
    TooLarge,
    /// The kind is not accepted.
    #[error("{0}")]
    Kind(#[from] KindError),
    /// A local read failed.
    #[error("{0}")]
    Local(#[from] LocalError),
    /// Too many documents or bytes held.
    #[error("Too many documents are waiting. Send or remove some first (at most 8, 200 MB).")]
    Full,
    /// Not a regular file.
    #[error("Only files can be attached, not folders or devices.")]
    NotAFile,
    /// The file could not be read.
    #[error("The file could not be read: {0}")]
    Io(String),
}

/// Why a stored document cannot do what was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// No such id (removed, or never added).
    #[error("No such document (it was removed or sent).")]
    NotFound,
    /// Not in a state for this (reading twice, reading a local kind, a picture of a PDF).
    #[error("{0}")]
    Wrong(&'static str),
    /// Could not persist a file in the session project.
    #[error("Could not save this attachment in the project: {0}")]
    Io(String),
}

/// What OCR needs for one document.
pub struct ReadJob {
    /// The document's id.
    pub id: String,
    /// The file name.
    pub name: String,
    /// The kind (its MIME type and whether it is an image).
    pub kind: DocKind,
    /// The file's bytes (handed back with [`DocumentStore::finish_read`]).
    pub bytes: Vec<u8>,
}

/// What OCR returned for one document.
pub struct OcrPages {
    /// Page texts (Markdown) in page order.
    pub pages: Vec<String>,
    /// The model Mistral used.
    pub model: String,
    /// Pages Mistral says it processed.
    pub pages_processed: Option<u32>,
}

struct Entry {
    info: DocumentInfo,
    bytes: Option<Vec<u8>>,
    text: Option<String>,
    full_text: Option<String>,
    saved: Option<(String, PathBuf)>,
}

/// Bytes of `text` and `full_text` together.
fn text_bytes(text: Option<&String>, full_text: Option<&String>) -> u64 {
    [text, full_text]
        .into_iter()
        .flatten()
        .map(|t| t.len() as u64)
        .sum()
}

impl Entry {
    /// What this document holds against [`MAX_HELD_BYTES`]: its file (also while OCR has the bytes) and its texts.
    fn held(&self) -> u64 {
        self.info.size + text_bytes(self.text.as_ref(), self.full_text.as_ref())
    }
}

/// The pending-document store (one per core).
#[derive(Default)]
pub struct DocumentStore {
    entries: Mutex<Vec<Entry>>,
}

fn new_id() -> String {
    let mut buf = [0_u8; 8];
    // getrandom fails only without an OS random source; the time is then a unique-enough fallback for
    // an id that grants nothing (ids are not secrets).
    if getrandom::fill(&mut buf).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        buf = u64::try_from(nanos & u128::from(u64::MAX))
            .unwrap_or(0)
            .to_le_bytes();
    }
    buf.iter().fold(String::from("doc-"), |mut id, b| {
        let _ = write!(id, "{b:02x}");
        id
    })
}

fn file_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name: String = name.chars().filter(|c| !c.is_control()).take(200).collect();
    if name.trim().is_empty() {
        "document".to_owned()
    } else {
        name
    }
}

fn u32_of(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl DocumentStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        // A panic while holding the lock leaves plain data behind; keep using it.
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reads and adds the file at `path` (a path the native picker or an OS drop gave, never one from
    /// a client).
    ///
    /// # Errors
    /// [`AddError`] in plain words; nothing is added then.
    pub fn add_path(&self, path: &Path) -> Result<DocumentInfo, AddError> {
        let meta = std::fs::metadata(path).map_err(|e| AddError::Io(e.kind().to_string()))?;
        if !meta.is_file() {
            return Err(AddError::NotAFile);
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(AddError::TooLarge);
        }
        let file = std::fs::File::open(path).map_err(|e| AddError::Io(e.kind().to_string()))?;
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| AddError::Io(e.kind().to_string()))?;
        self.add_bytes(&file_name(path), bytes)
    }

    /// Adds a file's bytes under `name` (Rust callers only: [`DocumentStore::add_path`] and tests).
    ///
    /// # Errors
    /// [`AddError`].
    pub fn add_bytes(&self, name: &str, bytes: Vec<u8>) -> Result<DocumentInfo, AddError> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(AddError::TooLarge);
        }
        // Reserve capacity under the same lock used to insert: concurrent adds cannot all parse
        // against one spare slot. Reject a full store before kind detection or local extraction.
        let mut entries = self.lock();
        let held_now: u64 = entries.iter().map(Entry::held).sum();
        if entries.len() >= MAX_DOCS || held_now + bytes.len() as u64 > MAX_HELD_BYTES {
            return Err(AddError::Full);
        }
        // Unknown/legacy binary formats are still useful to the agent as project files (shipped in 0.1.1): they
        // are attached as the original file and its path, never read as text or sent to OCR because of a name.
        let kind = match detect(name, &bytes) {
            Ok(kind) => kind,
            Err(KindError::Unsupported) => DocKind::Binary,
            Err(error) => return Err(error.into()),
        };
        let pages = page_count(kind, &bytes);
        let mut info = DocumentInfo {
            id: new_id(),
            name: name.to_owned(),
            kind,
            method: kind.method(),
            size: bytes.len() as u64,
            pages,
            status: DocStatus::Waiting,
            chars: None,
            kept: None,
            model: None,
            pages_read: None,
            error: None,
            can_attach_as_picture: kind.can_attach_as_picture() && bytes.len() <= MAX_PICTURE_BYTES,
        };
        let mut full_text = None;
        let (held, text) = if kind == DocKind::Binary {
            info.status = DocStatus::Ready;
            (Some(bytes), None)
        } else if kind.method() == ReadMethod::Local {
            let body = read_local(kind, &bytes)?;
            full_text = Some(format!("File: {name}\n\n{body}"));
            let extracted = from_text(name, &body);
            set_ready(&mut info, &extracted);
            (Some(bytes), Some(extracted.text))
        } else {
            (Some(bytes), None)
        };
        // The text read here counts like the file: a small package that expands to much text can fill the budget.
        if held_now + info.size + text_bytes(text.as_ref(), full_text.as_ref()) > MAX_HELD_BYTES {
            return Err(AddError::Full);
        }
        entries.push(Entry {
            info: info.clone(),
            bytes: held,
            text,
            full_text,
            saved: None,
        });
        Ok(info)
    }

    /// Every document, oldest first.
    #[must_use]
    pub fn list(&self) -> Vec<DocumentInfo> {
        self.lock().iter().map(|e| e.info.clone()).collect()
    }

    /// One document's facts.
    #[must_use]
    pub fn info(&self, id: &str) -> Option<DocumentInfo> {
        self.lock()
            .iter()
            .find(|e| e.info.id == id)
            .map(|e| e.info.clone())
    }

    /// One document's facts and its text (`None` text until ready).
    #[must_use]
    pub fn read(&self, id: &str) -> Option<(DocumentInfo, Option<String>)> {
        self.lock()
            .iter()
            .find(|e| e.info.id == id)
            .map(|e| (e.info.clone(), e.text.clone()))
    }

    /// Complete text for the saved reader; message previews keep their independent bound.
    #[must_use]
    pub fn full_text(&self, id: &str) -> Option<String> {
        self.lock()
            .iter()
            .find(|e| e.info.id == id)
            .and_then(|e| e.full_text.clone())
    }

    /// Saves the original bytes once through a caller-supplied project writer, retaining the
    /// result for OCR completion and repeated UI reads. Call from a blocking thread.
    ///
    /// # Errors
    /// [`StoreError`] if the document is gone, reading before a save, or the writer fails.
    pub fn save_with(
        &self,
        id: &str,
        session_id: &str,
        write: impl FnOnce(&DocumentInfo, &[u8]) -> Result<PathBuf, String>,
    ) -> Result<PathBuf, StoreError> {
        let mut entries = self.lock();
        let entry = entries
            .iter_mut()
            .find(|e| e.info.id == id)
            .ok_or(StoreError::NotFound)?;
        if let Some((owner, path)) = &entry.saved {
            return if owner == session_id {
                Ok(path.clone())
            } else {
                Err(StoreError::Wrong(
                    "This attachment belongs to a different session.",
                ))
            };
        }
        let bytes = entry.bytes.as_deref().ok_or(StoreError::Wrong(
            "This document is being read; save it after reading finishes.",
        ))?;
        let path = write(&entry.info, bytes).map_err(StoreError::Io)?;
        entry.saved = Some((session_id.to_owned(), path.clone()));
        Ok(path)
    }

    /// Starts reading an OCR document: marks it `reading` and hands its bytes out.
    ///
    /// # Errors
    /// [`StoreError::NotFound`]; [`StoreError::Wrong`] for a local kind or one already reading/read.
    pub fn begin_read(&self, id: &str) -> Result<ReadJob, StoreError> {
        let mut entries = self.lock();
        let entry = entries
            .iter_mut()
            .find(|e| e.info.id == id)
            .ok_or(StoreError::NotFound)?;
        if entry.info.method != ReadMethod::Ocr {
            return Err(StoreError::Wrong(
                "This document is read locally; it needs no OCR.",
            ));
        }
        match entry.info.status {
            DocStatus::Waiting | DocStatus::Failed => {}
            DocStatus::Reading => {
                return Err(StoreError::Wrong("This document is being read already."));
            }
            DocStatus::Ready => {
                return Err(StoreError::Wrong("This document has been read already."));
            }
        }
        let bytes = entry
            .bytes
            .take()
            .ok_or(StoreError::Wrong("This document's file is gone."))?;
        entry.info.status = DocStatus::Reading;
        entry.info.error = None;
        Ok(ReadJob {
            id: id.to_owned(),
            name: entry.info.name.clone(),
            kind: entry.info.kind,
            bytes,
        })
    }

    /// Ends a read: stores the text, or the reason and the bytes for another try. Returns the new facts,
    /// or `None` when the document was removed meanwhile (the result is dropped).
    pub fn finish_read(
        &self,
        job: ReadJob,
        result: Result<OcrPages, String>,
    ) -> Option<DocumentInfo> {
        let mut entries = self.lock();
        let others: u64 = entries
            .iter()
            .filter(|e| e.info.id != job.id)
            .map(Entry::held)
            .sum();
        let entry = entries.iter_mut().find(|e| e.info.id == job.id)?;
        let result = result.and_then(|ocr| {
            let full_text = crate::from_pages_with_limit(&job.name, &ocr.pages, usize::MAX).text;
            let extracted = from_pages(&job.name, &ocr.pages);
            // The read text counts like an added document's: past the budget it is not kept.
            let held =
                others + entry.info.size + text_bytes(Some(&extracted.text), Some(&full_text));
            if held > MAX_HELD_BYTES {
                Err(AddError::Full.to_string())
            } else {
                Ok((ocr, full_text, extracted))
            }
        });
        match result {
            Ok((ocr, full_text, extracted)) => {
                entry.full_text = Some(full_text);
                set_ready(&mut entry.info, &extracted);
                entry.info.model = Some(ocr.model);
                entry.info.pages_read = ocr.pages_processed.or(Some(u32_of(ocr.pages.len())));
                entry.text = Some(extracted.text);
                // Original bytes remain available for the project attachment and picture path.
                entry.bytes = Some(job.bytes);
            }
            Err(reason) => {
                entry.info.status = DocStatus::Failed;
                entry.info.error = Some(reason);
                entry.bytes = Some(job.bytes);
            }
        }
        Some(entry.info.clone())
    }

    /// The bytes of a picture-capable image, for the composer's picture path (the document stays).
    ///
    /// # Errors
    /// [`StoreError`] when not found or not a picture the gate accepts.
    pub fn picture(&self, id: &str) -> Result<(&'static str, Vec<u8>), StoreError> {
        let entries = self.lock();
        let entry = entries
            .iter()
            .find(|e| e.info.id == id)
            .ok_or(StoreError::NotFound)?;
        if !entry.info.can_attach_as_picture {
            return Err(StoreError::Wrong(
                "Only PNG, JPEG, WebP and GIF images up to 10 MB can be attached as a picture.",
            ));
        }
        let bytes = entry.bytes.clone().ok_or(StoreError::Wrong(
            if entry.info.status == DocStatus::Reading {
                "This image is being read; try again in a moment."
            } else {
                "This image's file is no longer held; add it again."
            },
        ))?;
        Ok((entry.info.kind.mime(), bytes))
    }

    /// Removes a document (sent, or the user removed it). Returns whether it existed.
    pub fn remove(&self, id: &str) -> bool {
        let mut entries = self.lock();
        let before = entries.len();
        entries.retain(|e| e.info.id != id);
        entries.len() != before
    }
}

fn set_ready(info: &mut DocumentInfo, extracted: &Extracted) {
    info.status = DocStatus::Ready;
    info.chars = Some(u32_of(extracted.chars()));
    info.kept = extracted.kept;
}
