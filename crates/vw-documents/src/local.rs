//! Reading on this computer the kinds that need no OCR: Word/PowerPoint through `office`, CSV and
//! text as they are, Excel through `calamine` (sheet names and CSV lines of saved cell values).
//!
//! Why locally: exact cell values (OCR would render the sheet as pages and read
//! the digits back), no OCR quota, and nothing leaves the computer before the message is sent.
//!
//! Depends on: calamine, `kind` (`is_text`). Called by: `store` (on add).
//! Invariants: text kinds must be valid UTF-8 (a byte-order mark is dropped); formulas give their
//! saved value (what Excel shows), not the formula; errors are reported in plain words; a workbook is parsed only
//! when it unpacks to at most `zipinfo::MAX_WORKBOOK_BYTES`.

use std::borrow::Cow;
use std::io::Cursor;

use calamine::{DataRef, Reader, ReaderRef, Xlsx};

use crate::kind::{DocKind, is_text};
use crate::text_budget::{MAX_EXTRACTED_BYTES, TOO_MUCH_TEXT, append};

/// Why a file could not be read locally.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocalError {
    /// The complete extraction would exceed the local reader's memory budget.
    #[error("This file contains too much text to read locally.")]
    TooMuchText,
    /// Not UTF-8 text.
    #[error("This file is not UTF-8 text. Save it as UTF-8 and try again.")]
    NotUtf8,
    /// The workbook could not be opened.
    #[error(
        "This Excel file could not be read ({0}). It may be damaged or protected with a password."
    )]
    Workbook(String),
    /// An Office document could not be opened.
    #[error("This {0} could not be read ({1}). It may be damaged or protected with a password.")]
    Office(&'static str, String),
    /// Not a local kind.
    #[error("This file type is read with OCR, not locally.")]
    NotLocal,
}

/// One CSV field, quoted when it needs to be (RFC 4180).
fn field(out: &mut String, value: &str) -> Result<(), String> {
    if value.contains([',', '"', '\n', '\r']) {
        append(out, "\"")?;
        for (index, piece) in value.split('"').enumerate() {
            if index > 0 {
                append(out, "\"\"")?;
            }
            append(out, piece)?;
        }
        append(out, "\"")
    } else {
        append(out, value)
    }
}

/// A cell as text: numbers as written, dates as ISO 8601, errors as `#ERROR`-style words.
fn cell<'a>(value: &'a DataRef<'_>) -> Cow<'a, str> {
    match value {
        DataRef::Empty => "".into(),
        DataRef::String(s) | DataRef::DateTimeIso(s) | DataRef::DurationIso(s) => s.as_str().into(),
        DataRef::SharedString(s) => (*s).into(),
        DataRef::DateTime(dt) => dt
            .as_datetime()
            .map_or_else(|| dt.as_f64().to_string(), |d| d.to_string())
            .trim_end_matches(" 00:00:00")
            .to_owned()
            .into(),
        other => calamine::Data::from(other.clone()).to_string().into(),
    }
}

/// Check actual cells before calamine builds a dense range. Shared strings stay borrowed, and
/// neither a forged dimension nor two distant cells can request an unbounded dense allocation.
fn check_sheet(book: &mut Xlsx<Cursor<&[u8]>>, name: &str, used: usize) -> Result<(), LocalError> {
    // About 2 million cells (a 100,000-row sheet with 20 columns): calamine's dense range stays below ~100 MB.
    const MAX_CELLS: u64 = 2_097_152;
    let mut reader = book
        .worksheet_cells_reader(name)
        .map_err(|e| LocalError::Workbook(e.to_string()))?;
    let (mut min_row, mut min_col) = (u32::MAX, u32::MAX);
    let (mut max_row, mut max_col) = (0, 0);
    let mut text_bytes = used;
    let mut cells = 0;
    while let Some(c) = reader
        .next_cell()
        .map_err(|e| LocalError::Workbook(e.to_string()))?
    {
        cells += 1;
        let value = cell(c.get_value());
        let quotes = value.bytes().filter(|b| *b == b'"').count();
        text_bytes = text_bytes
            .saturating_add(value.len())
            .saturating_add(quotes);
        if cells > MAX_CELLS || text_bytes > MAX_EXTRACTED_BYTES {
            return Err(LocalError::TooMuchText);
        }
        if matches!(c.get_value(), DataRef::Empty) {
            continue;
        }
        let (row, col) = c.get_position();
        min_row = min_row.min(row);
        min_col = min_col.min(col);
        max_row = max_row.max(row);
        max_col = max_col.max(col);
        let area = (u64::from(max_row - min_row) + 1) * (u64::from(max_col - min_col) + 1);
        if area > MAX_CELLS {
            return Err(LocalError::TooMuchText);
        }
    }
    Ok(())
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
        check_sheet(&mut book, &name, out.len())?;
        let range = book
            .worksheet_range_ref(&name)
            .map_err(|e| LocalError::Workbook(e.to_string()))?;
        if !out.is_empty() {
            append(&mut out, "\n").map_err(|_| LocalError::TooMuchText)?;
        }
        append(&mut out, "## Sheet: ").map_err(|_| LocalError::TooMuchText)?;
        append(&mut out, &name).map_err(|_| LocalError::TooMuchText)?;
        append(&mut out, "\n").map_err(|_| LocalError::TooMuchText)?;
        if range.is_empty() {
            append(&mut out, "(empty)\n").map_err(|_| LocalError::TooMuchText)?;
            continue;
        }
        for row in range.rows() {
            for (index, c) in row.iter().enumerate() {
                if index > 0 {
                    append(&mut out, ",").map_err(|_| LocalError::TooMuchText)?;
                }
                field(&mut out, &cell(c)).map_err(|_| LocalError::TooMuchText)?;
            }
            append(&mut out, "\n").map_err(|_| LocalError::TooMuchText)?;
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
            if body.len() > MAX_EXTRACTED_BYTES {
                return Err(LocalError::TooMuchText);
            }
            if !is_text(body) {
                return Err(LocalError::NotUtf8);
            }
            String::from_utf8(body.to_vec()).map_err(|_| LocalError::NotUtf8)
        }
        DocKind::Xlsx => workbook_text(bytes),
        DocKind::Docx | DocKind::Pptx => crate::office::read(kind, bytes).map_err(|e| {
            if e == TOO_MUCH_TEXT {
                LocalError::TooMuchText
            } else {
                LocalError::Office(kind.label(), e)
            }
        }),
        _ => Err(LocalError::NotLocal),
    }
}
