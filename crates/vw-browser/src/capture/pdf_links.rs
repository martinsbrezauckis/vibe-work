//! Restores text-link annotations omitted by `WebKitGTK`'s native PDF printer. Reads glyph positions from the
//! completed native PDF, matches the page's link labels in reading order and adds only URI annotations.
//! Depends on pdf-extract and lopdf; called only for native Linux print output, never for arbitrary input files.
use lopdf::{Document, Object, dictionary};
use pdf_extract::{MediaBox, OutputDev, OutputError, Transform};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct Link {
    pub url: String,
    pub text: String,
}
#[derive(Debug, Deserialize)]
pub(super) struct Links {
    pub url: String,
    pub links: Vec<Link>,
}
struct Glyph {
    start: usize,
    end: usize,
    page: u32,
    rect: [f64; 4],
}
#[derive(Default)]
struct Geometry {
    text: String,
    glyphs: Vec<Glyph>,
    page: u32,
}
impl OutputDev for Geometry {
    fn begin_page(
        &mut self,
        page_num: u32,
        _: &MediaBox,
        _: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), OutputError> {
        self.page = page_num;
        Ok(())
    }
    fn end_page(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
    fn begin_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
    fn end_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
    fn end_line(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
    fn output_character(
        &mut self,
        trm: &Transform,
        width: f64,
        _: f64,
        font_size: f64,
        text: &str,
    ) -> Result<(), OutputError> {
        if self.glyphs.len() >= 500_000 {
            return Err(std::io::Error::other("the PDF has too much text to annotate").into());
        }
        let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if text.is_empty() {
            return Ok(());
        }
        let x = trm.m31;
        let y = trm.m32;
        let dx = width * font_size * trm.m11;
        let dy = font_size * trm.m22;
        let rect = [
            x.min(x + dx),
            (y - dy * 0.2).min(y + dy * 0.8),
            x.max(x + dx),
            (y - dy * 0.2).max(y + dy * 0.8),
        ];
        if !rect.iter().all(|v| v.is_finite()) {
            return Ok(());
        }
        let start = self.text.len();
        self.text.push_str(&text);
        self.glyphs.push(Glyph {
            start,
            end: self.text.len(),
            page: self.page,
            rect,
        });
        Ok(())
    }
}

pub(super) fn annotate(bytes: &[u8], links: &[Link]) -> Result<Vec<u8>, String> {
    if links.is_empty() {
        return Ok(bytes.to_vec());
    }
    // Native PDFs are trusted engine output, but an unsupported font must return an error rather than unwind
    // through a capture task. Parsing and geometry extraction never see a user-supplied PDF.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        annotate_inner(bytes, links)
    }))
    .map_err(|_| "the PDF's text could not be read to keep its links".to_owned())?
}

#[allow(clippy::cast_possible_truncation)] // PDF real coordinates use f32; finite engine glyph coordinates fit page points.
fn annotate_inner(bytes: &[u8], links: &[Link]) -> Result<Vec<u8>, String> {
    let mut doc = Document::load_mem(bytes).map_err(|e| e.to_string())?;
    let mut geometry = Geometry::default();
    pdf_extract::output_doc(&doc, &mut geometry).map_err(|e| e.to_string())?;
    let pages = doc.get_pages();
    let mut cursor = 0;
    for link in links {
        let text: String = link.text.chars().filter(|c| !c.is_whitespace()).collect();
        if text.is_empty() {
            continue;
        }
        let Some(at) = geometry.text[cursor..].find(&text).map(|at| cursor + at) else {
            continue;
        };
        let end = at + text.len();
        let glyphs: Vec<_> = geometry
            .glyphs
            .iter()
            .filter(|g| g.start < end && g.end > at)
            .collect();
        // One rectangle per line, retaining the native printer's exact page and text coordinates.
        let mut rows: Vec<(u32, [f64; 4])> = Vec::new();
        for glyph in glyphs {
            if let Some((page, rect)) = rows.last_mut()
                && *page == glyph.page
                && (rect[1] - glyph.rect[1]).abs() < 2.0
            {
                rect[0] = rect[0].min(glyph.rect[0]);
                rect[2] = rect[2].max(glyph.rect[2]);
                rect[3] = rect[3].max(glyph.rect[3]);
            } else {
                rows.push((glyph.page, glyph.rect));
            }
        }
        for (page, rect) in rows {
            let Some(page) = pages.get(&page) else {
                continue;
            };
            let annotation = doc.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Link", "Rect" => rect.iter().map(|v| Object::Real(*v as f32)).collect::<Vec<_>>(),
                "Border" => vec![0.into(), 0.into(), 0.into()],
                "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal(link.url.as_bytes()) },
            });
            let page = doc.get_dictionary_mut(*page).map_err(|e| e.to_string())?;
            if !page.has(b"Annots") {
                page.set("Annots", Vec::<Object>::new());
            }
            page.get_mut(b"Annots")
                .and_then(Object::as_array_mut)
                .map_err(|e| e.to_string())?
                .push(annotation.into());
        }
        cursor = end;
    }
    let mut output = Vec::new();
    doc.save_to(&mut output).map_err(|e| e.to_string())?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};

    #[test]
    fn native_pdf_text_gets_a_clickable_annotation_without_changing_its_page_count() {
        let mut doc = Document::with_version("1.4");
        let pages = doc.new_object_id();
        let font = doc.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
        );
        let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![50.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal("Read the docs")]),
                Operation::new("ET", vec![]),
            ],
        }
        .encode()
        .unwrap();
        let content = doc.add_object(lopdf::Stream::new(dictionary! {}, content));
        let page = doc.add_object(
            dictionary! { "Type" => "Page", "Parent" => pages, "Resources" => resources,
            "Contents" => content, "MediaBox" => vec![0.into(),0.into(),595.into(),842.into()] },
        );
        doc.objects.insert(
            pages,
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }.into(),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        let mut original = Vec::new();
        doc.save_to(&mut original).unwrap();
        let bytes = annotate(
            &original,
            &[Link {
                url: "https://example.test/docs".into(),
                text: "Read the docs".into(),
            }],
        )
        .unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        let doc = Document::load_mem(&bytes).unwrap();
        assert_eq!(doc.get_pages().len(), 1);
        let annotations = doc
            .get_dictionary(page)
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(annotations.len(), 1);
        let annotation = doc
            .get_dictionary(annotations[0].as_reference().unwrap())
            .unwrap();
        assert_eq!(
            annotation.get(b"Subtype").unwrap().as_name().unwrap(),
            b"Link"
        );
        let rect = annotation.get(b"Rect").unwrap().as_array().unwrap();
        assert!((rect[0].as_float().unwrap() - 50.0).abs() < 1.0);
        assert!(
            annotation
                .get(b"A")
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"URI")
                .unwrap()
                .as_str()
                .unwrap()
                .ends_with(b"/docs")
        );
    }
}
