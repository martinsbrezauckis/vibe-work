//! Local Word and `PowerPoint` text in document/slide order, with tables, lists and notes.
//! Uses bounded OOXML parts; called by `local::read_local`. No rendering, OCR, network or external objects.

use crate::kind::DocKind;
use crate::office_xml::{Node, part, related};
use crate::text_budget::append;

fn word_paragraph(p: &Node, out: &mut String) -> Result<(), String> {
    let style = p.value("pStyle").to_ascii_lowercase();
    let heading = style
        .strip_prefix("heading")
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|&n| (1..=6).contains(&n));
    let prefix = if style == "title" {
        "# ".to_owned()
    } else if let Some(level) = heading {
        format!("{} ", "#".repeat(level))
    } else if !p.all("numPr").is_empty()
        || style.starts_with("listbullet")
        || style.starts_with("listnumber")
    {
        "- ".to_owned()
    } else {
        String::new()
    };
    append(out, &prefix)?;
    p.inline(out)?;
    append(out, "\n")
}

fn word_blocks(node: &Node, out: &mut String) -> Result<(), String> {
    for child in &node.children {
        match child.name.as_str() {
            "p" => word_paragraph(child, out)?,
            "tbl" => {
                for row in child.children.iter().filter(|n| n.name == "tr") {
                    append(out, "| ")?;
                    for (index, cell) in row.children.iter().filter(|n| n.name == "tc").enumerate()
                    {
                        if index > 0 {
                            append(out, " | ")?;
                        }
                        let mut text = String::new();
                        word_blocks(cell, &mut text)?;
                        for c in text.trim().chars() {
                            match c {
                                '|' => append(out, "\\|")?,
                                '\n' => append(out, " / ")?,
                                _ => append(out, c.encode_utf8(&mut [0; 4]))?,
                            }
                        }
                    }
                    append(out, " |\n")?;
                }
                append(out, "\n")?;
            }
            "del" => {}
            _ => word_blocks(child, out)?,
        }
    }
    Ok(())
}

fn word(bytes: &[u8]) -> Result<String, String> {
    let root = part(bytes, "word/document.xml")?;
    let body = root.child("body").ok_or("Word body is missing")?;
    let mut out = String::new();
    word_blocks(body, &mut out)?;
    Ok(out)
}

fn drawing_text(node: &Node, out: &mut String) -> Result<(), String> {
    for child in &node.children {
        match child.name.as_str() {
            "p" => {
                let mut text = String::new();
                child.inline(&mut text)?;
                if text.trim().is_empty() {
                    continue;
                }
                let props = child.child("pPr");
                let bullet = props
                    .is_some_and(|p| p.child("buChar").is_some() || p.child("buAutoNum").is_some());
                if bullet {
                    append(out, "- ")?;
                }
                append(out, &text)?;
                append(out, "\n")?;
            }
            // Notes placeholders repeat slide numbers, headers and slide images; keep the notes body.
            "sp" if child
                .all("ph")
                .iter()
                .any(|p| matches!(p.attr("type"), "sldNum" | "hdr" | "ftr" | "dt" | "sldImg")) => {}
            _ => drawing_text(child, out)?,
        }
    }
    Ok(())
}

fn slides(bytes: &[u8]) -> Result<String, String> {
    let source = "ppt/presentation.xml";
    let root = part(bytes, source)?;
    let links = related(bytes, source)?;
    let mut out = String::new();
    for (index, slide) in root.all("sldId").iter().enumerate() {
        let (path, _) = links
            .get(slide.attr("r:id"))
            .ok_or("slide relationship is missing")?;
        append(&mut out, &format!("## Slide {}\n", index + 1))?;
        drawing_text(&part(bytes, path)?, &mut out)?;
        for (notes, kind) in related(bytes, path)?.values() {
            if kind.ends_with("/notesSlide") {
                append(&mut out, "\n### Speaker notes\n")?;
                drawing_text(&part(bytes, notes)?, &mut out)?;
            }
        }
        append(&mut out, "\n")?;
    }
    Ok(out)
}

pub(crate) fn read(kind: DocKind, bytes: &[u8]) -> Result<String, String> {
    crate::zipinfo::inflates_within(bytes, crate::zipinfo::MAX_WORKBOOK_BYTES)?;
    let text = if kind == DocKind::Docx {
        word(bytes)?
    } else {
        slides(bytes)?
    };
    if text.trim().is_empty() {
        return Err("no readable text was found".to_owned());
    }
    Ok(text)
}
