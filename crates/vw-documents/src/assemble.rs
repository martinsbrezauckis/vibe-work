//! The text that goes into the message for one document: a first line naming the file, then the
//! pages (OCR) or the lines (local reading), cut only at whole pages or whole lines when the text is
//! over [`MAX_TEXT_CHARS`], and always saying what was kept (never silently truncated).
//!
//! Depends on: serde. Called by: `store` (after OCR or a local read).
//! Invariants: the result is at most [`MAX_TEXT_CHARS`] characters (the first line and page markers
//! included), except when the very first page or line alone is longer: then that one unit is cut at a
//! character boundary and `kept` says "part of page 1" (`kept == 0`).

use serde::Serialize;

/// Most characters of text one document may add to a message (~100k tokens).
pub const MAX_TEXT_CHARS: usize = 400_000;

/// What part of a document was kept when it was over the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    /// `pages` (OCR) or `lines` (local).
    pub unit: KeptUnit,
    /// Whole units kept from the start (0: only part of the first one).
    pub kept: u32,
    /// Units in the document.
    pub of: u32,
}

/// The unit of [`Kept`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeptUnit {
    /// OCR pages.
    Pages,
    /// Text lines (CSV rows, spreadsheet rows, text lines).
    Lines,
}

/// A document's text, ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// The text (first line names the file).
    pub text: String,
    /// `Some` when the document was cut.
    pub kept: Option<Kept>,
}

impl Extracted {
    /// Characters of the text.
    #[must_use]
    pub fn chars(&self) -> usize {
        self.text.chars().count()
    }
}

/// The first `n` characters of `s` (a character boundary, never inside a UTF-8 sequence).
fn head(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}

fn u32_of(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Joins `units` after `title`, keeping whole units while the total stays within `limit`.
fn join(
    title: &str,
    units: &[String],
    separator: impl Fn(usize) -> String,
    unit: KeptUnit,
    limit: usize,
) -> Extracted {
    let mut text = String::from(title);
    let mut used = title.chars().count();
    for (i, body) in units.iter().enumerate() {
        let piece = format!("{}{body}", separator(i));
        let size = piece.chars().count();
        if used + size > limit {
            if i == 0 {
                text.push_str(head(&piece, limit.saturating_sub(used)));
            }
            return Extracted {
                text,
                kept: Some(Kept {
                    unit,
                    kept: u32_of(i),
                    of: u32_of(units.len()),
                }),
            };
        }
        text.push_str(&piece);
        used += size;
    }
    Extracted { text, kept: None }
}

/// The text of OCR pages (in page order) of the file `name`.
#[must_use]
pub fn from_pages(name: &str, pages: &[String]) -> Extracted {
    from_pages_with_limit(name, pages, MAX_TEXT_CHARS)
}

/// [`from_pages`] with another limit (tests).
#[must_use]
pub fn from_pages_with_limit(name: &str, pages: &[String], limit: usize) -> Extracted {
    let title = format!("File: {name} (text read with OCR)");
    join(
        &title,
        pages,
        |i| format!("\n\n--- Page {} ---\n\n", i + 1),
        KeptUnit::Pages,
        limit,
    )
}

/// The text of a locally read file `name` (CSV, text, a converted workbook).
#[must_use]
pub fn from_text(name: &str, text: &str) -> Extracted {
    from_text_with_limit(name, text, MAX_TEXT_CHARS)
}

/// [`from_text`] with another limit (tests).
#[must_use]
pub fn from_text_with_limit(name: &str, text: &str, limit: usize) -> Extracted {
    let title = format!("File: {name}\n\n");
    let lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    join(&title, &lines, |_| String::new(), KeptUnit::Lines, limit)
}
