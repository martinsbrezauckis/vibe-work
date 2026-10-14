//! Reading a file for the File tab: its kind from the bytes (the name only for Markdown, HTML and SVG,
//! which are text either way), the first [`TEXT_LIMIT`] bytes of text, the whole of an image up to
//! [`IMAGE_LIMIT`], and a fingerprint that changes when the file does.
//!
//! Depends on: std, base64. Called by: `files` (after `files_scope` allowed the canonical path), tests.
//! Invariants:
//! - The size limit is checked on the OPENED file's metadata, and reads are capped (`take`), so a file that grows
//!   while being read cannot make the read unbounded.
//! - Text is cut on a UTF-8 character boundary; bytes that are not UTF-8 text (or hold NUL) make the file
//!   `binary`, never a lossy decode with replacement characters (no garbage shown as text).
//! - An image kind needs the image's magic bytes; a `.png` name on other bytes is not an image.

use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// Text shown from the start of a file (256 KiB); the rest is named, not loaded.
pub const TEXT_LIMIT: u64 = 256 * 1024;
/// Largest image shown in the window (as a `data:` URL).
pub const IMAGE_LIMIT: u64 = 20 * 1024 * 1024;

/// What the File tab shows a file as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    /// UTF-8 text (source code, JSON, CSV, logs): highlighted by the window when it knows the language.
    Text,
    /// Markdown (`.md`, `.markdown`): rendered by the window's safe Markdown module.
    Markdown,
    /// HTML (`.html`, `.htm`): shown by the isolated viewer; its text is the source view.
    Html,
    /// PNG, JPEG, GIF, WebP or SVG: shown as a picture.
    Image,
    /// PDF: shown by the isolated viewer's own PDF viewer.
    Pdf,
    /// Anything else: name, size, Show in folder, Open with default app.
    Binary,
}

impl FileKind {
    /// The kind a file NAME suggests, before reading (the resolver's quick answer; `read` decides from bytes).
    #[must_use]
    pub fn guess(name: &str) -> Self {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "md" | "markdown" => Self::Markdown,
            "html" | "htm" => Self::Html,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => Self::Image,
            "pdf" => Self::Pdf,
            "zip" | "gz" | "tgz" | "7z" | "rar" | "exe" | "dll" | "so" | "dylib" | "bin" | "o"
            | "a" | "class" | "jar" | "wasm" | "mp3" | "mp4" | "mov" | "webm" | "wav" | "ogg"
            | "flac" | "docx" | "xlsx" | "pptx" | "doc" | "xls" | "ppt" | "odt" | "ttf" | "otf"
            | "woff" | "woff2" | "ico" | "bmp" | "tif" | "tiff" | "avif" | "sqlite" | "db" => {
                Self::Binary
            }
            _ => Self::Text,
        }
    }
}

/// What reading a file gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRead {
    /// The kind decided from the bytes.
    pub kind: FileKind,
    /// Size in bytes.
    pub size: u64,
    /// Changes when the file's size or modification time does and, for a file read whole (at most [`TEXT_LIMIT`]
    /// bytes, everything an edit can touch), when its content does ([`fingerprint`]).
    pub fingerprint: String,
    /// Text (text, Markdown, HTML source): at most [`TEXT_LIMIT`] bytes, cut on a character boundary.
    pub text: Option<String>,
    /// True when `text` is only the start of the file.
    pub truncated: bool,
    /// An image as a `data:` URL.
    pub data_url: Option<String>,
    /// An image over [`IMAGE_LIMIT`] (not loaded).
    pub too_large: bool,
}

/// The fingerprint of a file: `<size>-<mtime in ns>` (mtime 0 when the OS has none), plus `-<content hash>` when
/// `whole` holds the file's complete bytes.
///
/// Size and time alone miss a rewrite that keeps both (same length, a coarse or restored timestamp), and a quick
/// edit is saved only when the file is still the version the user started from: a missed change would let an older
/// draft overwrite the agent's newer text. Every file small enough to edit (at most [`TEXT_LIMIT`] bytes) therefore
/// carries the first 16 bytes of its BLAKE3 hash, computed the same way by a read, a stat and the save's check.
#[must_use]
pub fn fingerprint(size: u64, modified: Option<SystemTime>, whole: Option<&[u8]>) -> String {
    let nanos = modified
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    match whole {
        Some(bytes) => {
            let hash = blake3::hash(bytes).to_hex();
            // 32 hex digits = the first 16 bytes; the hex text is ASCII, so the slice is on a character boundary.
            let hex = &hash.as_str()[..32];
            format!("{size}-{nanos}-{hex}")
        }
        None => format!("{size}-{nanos}"),
    }
}

/// The fingerprint of the regular file at `path`, read whole when it is at most [`TEXT_LIMIT`] bytes (the same rule
/// as [`read_file`], so a stat and a read of an unchanged file agree).
///
/// # Errors
/// The filesystem error (`NotFound` for a missing file or something that is not a regular file).
pub fn fingerprint_of(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not a regular file",
        ));
    }
    let size = meta.len();
    if size > TEXT_LIMIT {
        return Ok(fingerprint(size, meta.modified().ok(), None));
    }
    let bytes = read_capped(&mut file, TEXT_LIMIT + 1)?;
    // A file that grew between the metadata and the read is reported by its bytes: the next poll sees the change.
    Ok(fingerprint(
        bytes.len() as u64,
        meta.modified().ok(),
        Some(&bytes),
    ))
}

/// The image MIME type of `head`, or `None`.
fn image_mime(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP".as_slice()) {
        Some("image/webp")
    } else {
        None
    }
}

/// The text of `bytes` (a head of `size` bytes in all): `None` when it is not UTF-8 text.
fn text_of(bytes: &[u8], truncated: bool) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_owned()),
        // Cut inside a character at the end of a head: keep the whole characters before it.
        Err(error) if truncated && error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()])
                .ok()
                .map(str::to_owned)
        }
        Err(_) => None,
    }
}

fn read_capped(file: &mut std::fs::File, cap: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(cap).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Reads `path` (already allowed by the scope; opened once, checked on the handle).
///
/// # Errors
/// The filesystem error (`NotFound` for a missing file or something that is not a regular file).
pub fn read_file(path: &Path) -> std::io::Result<FileRead> {
    let mut file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not a regular file",
        ));
    }
    let size = meta.len();
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().to_ascii_lowercase());
    let head = read_capped(&mut file, TEXT_LIMIT)?;
    let truncated = size > head.len() as u64;
    let print = fingerprint(
        size,
        meta.modified().ok(),
        (!truncated).then_some(head.as_slice()),
    );
    let base = FileRead {
        kind: FileKind::Binary,
        size,
        fingerprint: print,
        text: None,
        truncated: false,
        data_url: None,
        too_large: false,
    };
    if head.starts_with(b"%PDF-") {
        return Ok(FileRead {
            kind: FileKind::Pdf,
            ..base
        });
    }
    if let Some(mime) = image_mime(&head) {
        if size > IMAGE_LIMIT {
            return Ok(FileRead {
                kind: FileKind::Image,
                too_large: true,
                ..base
            });
        }
        let mut bytes = head;
        bytes.extend(read_capped(
            &mut file,
            IMAGE_LIMIT + 1 - bytes.len() as u64,
        )?);
        if bytes.len() as u64 > IMAGE_LIMIT {
            // The file grew past the limit while being read.
            return Ok(FileRead {
                kind: FileKind::Image,
                too_large: true,
                ..base
            });
        }
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok(FileRead {
            kind: FileKind::Image,
            data_url: Some(format!("data:{mime};base64,{data}")),
            ..base
        });
    }
    let Some(text) = text_of(&head, truncated) else {
        return Ok(base);
    };
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    // SVG is text; shown as a picture when it is a whole SVG document small enough to load.
    if ext == "svg" && !truncated && text.contains("<svg") {
        let data = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        return Ok(FileRead {
            kind: FileKind::Image,
            data_url: Some(format!("data:image/svg+xml;base64,{data}")),
            text: Some(text),
            ..base
        });
    }
    let kind = match ext {
        "md" | "markdown" => FileKind::Markdown,
        "html" | "htm" => FileKind::Html,
        _ => FileKind::Text,
    };
    Ok(FileRead {
        kind,
        text: Some(text),
        truncated,
        ..base
    })
}
