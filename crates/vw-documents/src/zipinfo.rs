//! Small, bounded reads of ZIP containers (Office files): the entry names, and one small entry's text.
//!
//! Depends on: zip. Called by: `kind` (which Office kind), `pages` (DOCX page count, PPTX slides).
//! Invariants: never inflates more than [`MAX_ENTRY_BYTES`] of one metadata entry or [`MAX_CONTENT_PART_BYTES`] of one
//! content part (a ZIP bomb stops there),
//! never lists more than [`MAX_ENTRIES`] names; [`inflates_within`] inflates a whole container at most once, into a
//! counting sink (nothing kept), and stops at its limit.

use std::io::{Cursor, Read};

/// Most bytes read from one small metadata entry (page counts, relationships).
pub const MAX_ENTRY_BYTES: u64 = 4 * 1024 * 1024;
/// Most bytes read from one content part a local reader parses (a Word body, a slide): about 1,500 heavily formatted
/// pages; its parsed tree stays near 100 MB. The whole package is first checked by [`inflates_within`].
pub const MAX_CONTENT_PART_BYTES: u64 = 16 * 1024 * 1024;
/// Most entry names read.
pub const MAX_ENTRIES: usize = 50_000;
/// Most bytes all entries of one workbook may inflate to before it is parsed (see [`inflates_within`]).
pub const MAX_WORKBOOK_BYTES: u64 = 256 * 1024 * 1024;

/// Checks that the ZIP in `bytes` inflates to at most `limit` bytes in all, by inflating every entry once into a
/// counting sink. The sizes an archive declares are not trusted (a ZIP bomb can claim small entries): the real
/// output is counted, and the read stops as soon as the total passes the limit, so memory stays flat and work is
/// bounded by `limit`. A parser that keeps whole entries in memory (the workbook reader) runs only after this passes.
///
/// # Errors
/// The ZIP cannot be read (damaged, not a ZIP, too many entries) or inflates to more than `limit` bytes.
pub fn inflates_within(bytes: &[u8], limit: u64) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err("too many entries".to_owned());
    }
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let left = limit.saturating_sub(total);
        let read = std::io::copy(&mut entry.take(left + 1), &mut std::io::sink())
            .map_err(|e| e.to_string())?;
        total = total.saturating_add(read);
        if total > limit {
            return Err(format!(
                "it unpacks to more than {} MB",
                limit / (1024 * 1024)
            ));
        }
    }
    Ok(())
}

/// The entry names of the ZIP in `bytes`.
///
/// # Errors
/// The ZIP cannot be read (damaged, not a ZIP, too many entries).
pub fn entry_names(bytes: &[u8]) -> Result<Vec<String>, String> {
    let archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err("too many entries".to_owned());
    }
    Ok(archive.file_names().map(str::to_owned).collect())
}

/// The text of the entry `name` (UTF-8, at most [`MAX_ENTRY_BYTES`]), or `None` when it is missing,
/// unreadable, larger than the cap or not UTF-8.
#[must_use]
pub fn entry_text(bytes: &[u8], name: &str) -> Option<String> {
    entry_text_within(bytes, name, MAX_ENTRY_BYTES)
}

/// [`entry_text`] with its own cap (at most `cap` bytes inflated, so a ZIP bomb stops there).
#[must_use]
pub fn entry_text_within(bytes: &[u8], name: &str, cap: u64) -> Option<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let entry = archive.by_name(name).ok()?;
    let mut out = Vec::new();
    entry.take(cap + 1).read_to_end(&mut out).ok()?;
    if out.len() as u64 > cap {
        return None;
    }
    String::from_utf8(out).ok()
}
