//! What kind of document a file is, decided from its bytes (the name only breaks ties the bytes cannot:
//! CSV vs. plain text vs. Markdown), and how it is read (Mistral OCR or locally). Since any UTF-8 text
//! without NUL is text (source code, JSON, YAML,...), and archives get their own sentence.
//!
//! Depends on: `zipinfo` (the entry names of a ZIP container). Called by: `store` (on add), tests.
//! Invariants: a file's extension never makes binary bytes a text kind, and never makes a ZIP a PDF
//! (or the other way round); anything not listed is refused with a reason a person can act on.

use serde::Serialize;

use crate::zipinfo::entry_names;

/// The kinds Vibe Work accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DocKind {
    /// PDF (`%PDF-`).
    Pdf,
    /// Word (Office Open XML: a ZIP with `word/`).
    Docx,
    /// `PowerPoint` (a ZIP with `ppt/`).
    Pptx,
    /// Excel (a ZIP with `xl/`): read locally.
    Xlsx,
    /// Comma-separated values: read locally.
    Csv,
    /// Plain text or Markdown: read locally.
    Text,
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
    /// WebP.
    Webp,
    /// GIF.
    Gif,
    /// TIFF.
    Tiff,
    /// BMP.
    Bmp,
    /// AVIF.
    Avif,
    /// Another file type: saved to the project and supplied to the agent by path.
    Binary,
}

/// How a kind is turned into text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadMethod {
    /// Sent to Mistral OCR (spends the user's OCR quota; only after the user asks).
    Ocr,
    /// Read on this computer (no quota, nothing leaves before the message is sent).
    Local,
    /// Kept as a project file; no text extraction is available in the composer.
    File,
}

impl DocKind {
    /// How this kind is read.
    #[must_use]
    pub fn method(self) -> ReadMethod {
        match self {
            Self::Xlsx | Self::Csv | Self::Text => ReadMethod::Local,
            Self::Binary => ReadMethod::File,
            _ => ReadMethod::Ocr,
        }
    }

    /// True for image kinds (OCR goes through `image_url`, not `document_url`).
    #[must_use]
    pub fn is_image(self) -> bool {
        matches!(
            self,
            Self::Png | Self::Jpeg | Self::Webp | Self::Gif | Self::Tiff | Self::Bmp | Self::Avif
        )
    }

    /// The image kinds Vibe itself accepts as a picture in a message (the gate's list).
    #[must_use]
    pub fn can_attach_as_picture(self) -> bool {
        matches!(self, Self::Png | Self::Jpeg | Self::Webp | Self::Gif)
    }

    /// The MIME type used in the `data:` URL sent to OCR.
    #[must_use]
    pub fn mime(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Csv => "text/csv",
            Self::Text => "text/plain",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
            Self::Tiff => "image/tiff",
            Self::Bmp => "image/bmp",
            Self::Avif => "image/avif",
            Self::Binary => "application/octet-stream",
        }
    }

    /// A short name for people ("PDF", "Word document",...).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Docx => "Word document",
            Self::Pptx => "PowerPoint",
            Self::Xlsx => "Excel workbook",
            Self::Csv => "CSV",
            Self::Text => "text",
            Self::Png => "PNG image",
            Self::Jpeg => "JPEG image",
            Self::Webp => "WebP image",
            Self::Gif => "GIF image",
            Self::Tiff => "TIFF image",
            Self::Bmp => "BMP image",
            Self::Avif => "AVIF image",
            Self::Binary => "file",
        }
    }
}

/// The sentence shown when a file is not accepted.
pub const UNSUPPORTED: &str = "This file type is not supported. Attach a PDF, Word, PowerPoint, Excel, CSV, text file or an image.";

fn image_kind(bytes: &[u8]) -> Option<DocKind> {
    let at = |range: std::ops::Range<usize>| bytes.get(range);
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(DocKind::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(DocKind::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(DocKind::Gif)
    } else if bytes.starts_with(b"RIFF") && at(8..12) == Some(b"WEBP".as_slice()) {
        Some(DocKind::Webp)
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some(DocKind::Tiff)
    } else if bytes.starts_with(b"BM") && bytes.len() > 26 {
        Some(DocKind::Bmp)
    } else if at(4..8) == Some(b"ftyp".as_slice()) && matches!(at(8..12), Some(b"avif" | b"avis")) {
        Some(DocKind::Avif)
    } else {
        None
    }
}

/// The Office kind of a ZIP container, from the folders it holds.
fn office_kind(bytes: &[u8]) -> Option<DocKind> {
    let names = entry_names(bytes).ok()?;
    let has = |prefix: &str| names.iter().any(|n| n.starts_with(prefix));
    if !names.iter().any(|n| n == "[Content_Types].xml") {
        return None;
    }
    if has("word/") {
        Some(DocKind::Docx)
    } else if has("ppt/") {
        Some(DocKind::Pptx)
    } else if has("xl/") {
        Some(DocKind::Xlsx)
    } else {
        None
    }
}

/// True for gzip, 7-Zip and RAR archives (ZIP is told apart from Office files by its entries).
fn is_archive(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x1f\x8b")
        || bytes.starts_with(b"7z\xbc\xaf\x27\x1c")
        || bytes.starts_with(b"Rar!\x1a\x07")
}

/// True when `bytes` is UTF-8 text a person would call text: valid UTF-8 and no NUL byte.
#[must_use]
pub fn is_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Why a file was not accepted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KindError {
    /// No accepted kind matches the bytes.
    #[error("{UNSUPPORTED}")]
    Unsupported,
    /// A ZIP (not an Office file), gzip, 7-Zip or RAR archive: archives are not unpacked.
    #[error("Archives are not unpacked. Unpack it and attach the files you need.")]
    Archive,
    /// The name says text but the bytes are not UTF-8 text.
    #[error(
        "This file is not UTF-8 text, so it cannot be read as {0}. Save it as UTF-8 and try again."
    )]
    NotUtf8(&'static str),
}

/// Decides the kind of a file from its bytes; `name` is used only for CSV / text / Markdown.
///
/// # Errors
/// [`KindError`] with the reason shown to the user.
pub fn detect(name: &str, bytes: &[u8]) -> Result<DocKind, KindError> {
    if bytes.starts_with(b"%PDF-") {
        return Ok(DocKind::Pdf);
    }
    if let Some(kind) = image_kind(bytes) {
        return Ok(kind);
    }
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        return office_kind(bytes).ok_or(KindError::Archive);
    }
    if is_archive(bytes) {
        return Err(KindError::Archive);
    }
    let named = match extension(name).as_str() {
        "csv" => Some((DocKind::Csv, "CSV")),
        "txt" | "text" | "md" | "markdown" | "log" => Some((DocKind::Text, "text")),
        _ => None,
    };
    match (named, is_text(bytes)) {
        (Some((kind, _)), true) => Ok(kind),
        (Some((_, label)), false) => Err(KindError::NotUtf8(label)),
        // source code, JSON, YAML, HTML source, a `Makefile`: any UTF-8 text without NUL is read as
        // text, decided by the bytes (the name plays no part, so binary bytes never become text).
        (None, true) => Ok(DocKind::Text),
        (None, false) => Err(KindError::Unsupported),
    }
}
