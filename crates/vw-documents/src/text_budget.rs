//! The local readers' total UTF-8 output bound, checked before every append or expansion.
//! Used by Office and workbook assembly; independent of the smaller message preview.

/// Most extracted UTF-8 bytes retained from one local document (including formatting): the same 50 MiB as the
/// per-file size limit (`store::MAX_FILE_BYTES`), so an ordinary large document, sheet or log reads in full and only
/// a package whose text expands past what any attachable file holds (repeated slide or string references) is refused.
pub const MAX_EXTRACTED_BYTES: usize = 50 * 1024 * 1024;
pub(crate) const TOO_MUCH_TEXT: &str = "This file contains too much text to read locally.";

pub(crate) fn append(out: &mut String, text: &str) -> Result<(), String> {
    if text.len() > MAX_EXTRACTED_BYTES.saturating_sub(out.len()) {
        return Err(TOO_MUCH_TEXT.into());
    }
    out.push_str(text);
    Ok(())
}
