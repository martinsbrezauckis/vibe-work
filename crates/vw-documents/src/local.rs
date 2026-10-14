//! Reading on this computer the kinds that need no OCR: CSV and text as they are, Excel workbooks
//! through `calamine` (each sheet as `## Sheet: <name>` and CSV lines of its cell values).
//!
//! Why locally: exact cell values (OCR would render the sheet as pages and read
//! the digits back), no OCR quota, and nothing leaves the computer before the message is sent.
//!
//! Depends on: calamine, `kind` (`is_text`). Called by: `store` (on add).
//! Invariants: text kinds must be valid UTF-8 (a byte-order mark is dropped); formulas give their
//! saved value (what Excel shows), not the formula; errors are reported in plain words; a workbook is parsed only
//! when it unpacks to at most `zipinfo::MAX_WORKBOOK_BYTES`.

use std::fmt::Write as _;
use std::io::Cursor;

use calamine::{Data, Reader, Xlsx};

use crate::kind::{DocKind, is_text};

/// Why a file could not be read locally.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocalError {
    /// Not UTF-8 text.
    #[error("This file is not UTF-8 text. Save it as UTF-8 and try again.")]
    NotUtf8,
    /// The workbook could not be opened.
    #[error(
        "This Excel file could not be read ({0}). It may be damaged or protected with a password."
    )]
    Workbook(String),
    /// Not a local kind.
    #[error("This file type is read with OCR, not locally.")]
    NotLocal,
}

/// One CSV field, quoted when it needs to be (RFC 4180).
fn field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// A cell as text: numbers as written, dates as ISO 8601, errors as `#ERROR`-style words.
fn cell(value: &Data) -> String {
    match value {
        Data::Empty => String::new(),
        Data::DateTime(dt) => dt
            .as_datetime()
            .map_or_else(|| dt.as_f64().to_string(), |d| d.to_string())
            .trim_end_matches(" 00:00:00")
            .to_owned(),
        other => other.to_string(),
    }
}

/// The workbook as text: every sheet in order, empty sheets named as empty.
fn workbook_text(bytes: &[u8]) -> Result<String, LocalError> {
    // The workbook parser keeps whole sheets in memory: a small hostile file that unpacks to gigabytes is refused
    // before it is parsed (its real unpacked size is counted, not the size it declares).
    crate::zipinfo::inflates_within(bytes, crate::zipinfo::MAX_WORKBOOK_BYTES)
        .map_err(LocalError::Workbook)?;
    let mut book: Xlsx<_> =
        Xlsx::new(Cursor::new(bytes)).map_err(|e| LocalError::Workbook(e.to_string()))?;
    let mut out = String::new();
    for name in book.sheet_names() {
        let range = book
            .worksheet_range(&name)
            .map_err(|e| LocalError::Workbook(e.to_string()))?;
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = writeln!(out, "## Sheet: {name}");
        if range.is_empty() {
            out.push_str("(empty)\n");
            continue;
        }
        for row in range.rows() {
            let line: Vec<String> = row.iter().map(|c| field(&cell(c))).collect();
            out.push_str(&line.join(","));
            out.push('\n');
        }
    }
    Ok(out)
}

/// The text of a local kind's bytes.
///
/// # Errors
/// [`LocalError`] in plain words.
pub fn read_local(kind: DocKind, bytes: &[u8]) -> Result<String, LocalError> {
    match kind {
        DocKind::Csv | DocKind::Text => {
            let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
            if !is_text(body) {
                return Err(LocalError::NotUtf8);
            }
            String::from_utf8(body.to_vec()).map_err(|_| LocalError::NotUtf8)
        }
        DocKind::Xlsx => workbook_text(bytes),
        _ => Err(LocalError::NotLocal),
    }
}
