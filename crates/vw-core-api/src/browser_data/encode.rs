//! CSV and XLSX text-cell encoding for rendered browser datasets; called by the chat-folder exporter.
//! CSV neutralizes spreadsheet formulas; XLSX uses inline strings and contains no executable formulas.
use std::io::{Cursor, Write};
use zip::{ZipWriter, write::SimpleFileOptions};

pub(super) fn csv(columns: &[String], rows: &[Vec<String>]) -> Vec<u8> {
    let mut output = String::new();
    for row in std::iter::once(columns).chain(rows.iter().map(Vec::as_slice)) {
        for (index, value) in row.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            output.push('"');
            if value.trim_start().starts_with(['=', '+', '-', '@'])
                || value.starts_with(['\t', '\r'])
            {
                output.push('\'');
            }
            output.push_str(&value.replace('"', "\"\""));
            output.push('"');
        }
        output.push_str("\r\n");
    }
    output.into_bytes()
}

fn xml(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            matches!(*c, '\t' | '\n' | '\r') || *c >= ' ' && !matches!(*c, '\u{fffe}' | '\u{ffff}')
        })
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(super) fn xlsx(columns: &[String], rows: &[Vec<String>]) -> Result<Vec<u8>, String> {
    let mut sheet = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>",
    );
    for row in std::iter::once(columns).chain(rows.iter().map(Vec::as_slice)) {
        sheet.push_str("<row>");
        for value in row {
            sheet.push_str("<c t=\"inlineStr\"><is><t xml:space=\"preserve\">");
            sheet.push_str(&xml(value));
            sheet.push_str("</t></is></c>");
        }
        sheet.push_str("</row>");
    }
    sheet.push_str("</sheetData></worksheet>");
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let entries = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Browser data" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        ("xl/worksheets/sheet1.xml", &sheet),
    ];
    for (name, body) in entries {
        archive
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .map_err(|_| "Spreadsheet encoding failed; retry the export.")?;
        archive
            .write_all(body.as_bytes())
            .map_err(|_| "Spreadsheet encoding failed; retry the export.")?;
    }
    archive
        .finish()
        .map(Cursor::into_inner)
        .map_err(|_| "Spreadsheet encoding failed; retry the export.".into())
}
