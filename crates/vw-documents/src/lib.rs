//! Documents the user attaches to a message: what kind a file is (from its bytes), how many pages it has
//! (told before any OCR quota is spent), reading Office documents, CSV and text on this computer, the text
//! that goes into the message (cut only at whole pages or lines, always saying what was kept), and the
//! store of documents that are not in a message yet.
//!
//! OCR itself is not here: the core calls Mistral through `vw-mistral` with the key from the key
//! service and hands the pages back to [`DocumentStore::finish_read`].
//!
//! Depends on: calamine, quick-xml, zip, flate2, serde. Called by: vw-core-api (`documents`), tests.
//! Invariant: paths come from native pickers/drops; browser uploads supply bytes. Every entry
//! checks the same kind and size bounds before reading.

mod assemble;
mod kind;
mod local;
mod office;
mod office_xml;
mod pages;
mod pdf_pages;
mod store;
mod text_budget;
mod zipinfo;

pub use assemble::{
    Extracted, Kept, KeptUnit, MAX_TEXT_CHARS, from_pages, from_pages_with_limit, from_text,
    from_text_with_limit,
};
pub use kind::{DocKind, KindError, ReadMethod, UNSUPPORTED, detect, is_text};
pub use local::{LocalError, read_local};
pub use pages::page_count;
pub use pdf_pages::count_pages;
pub use store::{
    AddError, DocStatus, DocumentInfo, DocumentStore, MAX_DOCS, MAX_FILE_BYTES, MAX_HELD_BYTES,
    MAX_PICTURE_BYTES, OcrPages, ReadJob, StoreError,
};
pub use text_budget::MAX_EXTRACTED_BYTES;
