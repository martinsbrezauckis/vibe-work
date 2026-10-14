//! How many pages a document has, told before anything is sent to OCR (the user decides with this
//! number, since OCR is billed per page).
//!
//! Depends on: `kind`, `pdf_pages`, `zipinfo`. Called by: `store` (on add).
//! Invariants: `None` means "unknown" and is shown as such, never guessed. Sources: PDF page tree,
//! PPTX slide parts, the page number Word or `LibreOffice` wrote into a DOCX (`docProps/app.xml`
//! `<Pages>`), one page per image.

use crate::kind::DocKind;
use crate::pdf_pages::count_pages;
use crate::zipinfo::{entry_names, entry_text};

/// The number in `<Pages>N</Pages>` of an `app.xml`.
fn app_xml_pages(xml: &str) -> Option<u32> {
    let start = xml.find("<Pages>")? + "<Pages>".len();
    let end = start + xml[start..].find("</Pages>")?;
    xml[start..end].trim().parse().ok().filter(|&n| n > 0)
}

/// Slide parts of a PPTX: `ppt/slides/slide<N>.xml`.
fn slide_count(bytes: &[u8]) -> Option<u32> {
    let names = entry_names(bytes).ok()?;
    let slides = names
        .iter()
        .filter(|n| {
            n.strip_prefix("ppt/slides/slide")
                .and_then(|rest| rest.strip_suffix(".xml"))
                .is_some_and(|num| !num.is_empty() && num.bytes().all(|b| b.is_ascii_digit()))
        })
        .count();
    u32::try_from(slides).ok().filter(|&n| n > 0)
}

/// The page count of a document of `kind`, when it can be told; `None` for local kinds (no pages are
/// sent) and when unknown.
#[must_use]
pub fn page_count(kind: DocKind, bytes: &[u8]) -> Option<u32> {
    match kind {
        DocKind::Pdf => count_pages(bytes),
        DocKind::Pptx => slide_count(bytes),
        DocKind::Docx => entry_text(bytes, "docProps/app.xml").and_then(|x| app_xml_pages(&x)),
        DocKind::Xlsx | DocKind::Csv | DocKind::Text | DocKind::Binary => None,
        DocKind::Png
        | DocKind::Jpeg
        | DocKind::Webp
        | DocKind::Gif
        | DocKind::Tiff
        | DocKind::Bmp
        | DocKind::Avif => Some(1),
    }
}
