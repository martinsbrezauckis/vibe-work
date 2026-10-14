//! Synthetic PDF and workbook inputs exercise document reader limits.

use vw_documents::{DocKind, count_pages, read_local};

fn pdf_with_object_stream(n: usize, header: &str, body: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut inflated = header.as_bytes().to_vec();
    inflated.extend_from_slice(body);
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(&inflated).unwrap();
    let packed = z.finish().unwrap();
    let mut pdf = format!(
        "%PDF-1.7\n1 0 obj\n<< /Type /ObjStm /N {n} /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
        header.len(),
        packed.len()
    )
    .into_bytes();
    pdf.extend_from_slice(&packed);
    pdf.extend_from_slice(b"\nendstream\nendobj\ntrailer\n%%EOF\n");
    pdf
}

#[test]
fn overlapping_object_offsets_are_not_trusted() {
    let tree = b"<< /Type /Pages /Count 7 /Kids [] >> ";
    let sane = pdf_with_object_stream(1, "2 0 ", tree);
    assert_eq!(
        count_pages(&sane),
        Some(7),
        "control: a normal object stream counts"
    );
    let overlapping = pdf_with_object_stream(2, "2 0 3 0 ", tree);
    assert_eq!(count_pages(&overlapping), None);
}

#[test]
fn oversized_object_stream_makes_the_count_unknown() {
    let mut body = b"<< /Type /Pages /Count 4 /Kids [] >> ".to_vec();
    // The per-stream budget (`pdf_pages::MAX_INFLATED`, 64 MiB; the module is private).
    let pad = 64 * 1024 * 1024;
    body.resize(pad + 1024, b' ');
    let pdf = pdf_with_object_stream(1, "2 0 ", &body);
    assert!(
        pdf.len() < 1024 * 1024,
        "the fixture is small on disk ({} bytes)",
        pdf.len()
    );
    assert_eq!(count_pages(&pdf), None);
}

#[test]
fn workbook_over_the_unpacked_budget_is_refused_before_parsing() {
    use std::io::Write as _;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    zip.start_file("xl/worksheets/sheet1.xml", options).unwrap();
    let chunk = vec![0_u8; 1024 * 1024];
    for _ in 0..300 {
        zip.write_all(&chunk).unwrap();
    }
    let bomb = zip.finish().unwrap().into_inner();
    assert!(
        bomb.len() < 2 * 1024 * 1024,
        "the fixture is small ({} bytes)",
        bomb.len()
    );
    let refused = read_local(DocKind::Xlsx, &bomb);
    let message = refused
        .as_ref()
        .map_err(ToString::to_string)
        .err()
        .unwrap_or_default();
    assert!(
        message.contains("unpacks to more than 256 MB"),
        "{refused:?}"
    );
}
