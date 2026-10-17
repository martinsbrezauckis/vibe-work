//! Large synthetic records and boundary falsifiers. Uses standard encoders and vw-logscan.
//! Exercises bounded streaming through long lines, legacy JSON and all base64 variants.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, io::Write};

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use vw_logscan::{Encoding, Secret, redact, scan};

const CHUNK: usize = 64 * 1024;
const TOKEN: &str = "ghp_boundaryFixture012345";

fn input() -> Vec<Secret> {
    vec![Secret::new("boundary", TOKEN.to_owned())]
}

#[test]
fn matches_each_encoding_across_chunk_boundaries_without_duplicate_hits() {
    let root = tempfile::tempdir().unwrap();
    for (name, value, encoding) in [
        ("raw", TOKEN.to_owned(), Encoding::AsIs),
        ("base64", STANDARD.encode(TOKEN), Encoding::Base64),
        (
            "json",
            "fixture\"\\\nvalue+/?".to_owned(),
            Encoding::JsonEscaped,
        ),
        (
            "url",
            "fixture\"\\\nvalue+/?".to_owned(),
            Encoding::UrlEncoded,
        ),
    ] {
        let stored = match encoding {
            Encoding::JsonEscaped => {
                let json = serde_json::to_string(&value).unwrap();
                json[1..json.len() - 1].to_owned()
            }
            Encoding::UrlEncoded => "fixture%22%5C%0Avalue%2B%2F%3F".to_owned(),
            _ => value.clone(),
        };
        let path = root.path().join(format!("{name}.jsonl"));
        // Try every position within the representation around two successive buffer boundaries.
        for split in 1..stored.len() {
            let prefix = "{\"content\":\"";
            let padding = CHUNK * 2 - split - prefix.len();
            let mut file = fs::File::create(&path).unwrap();
            writeln!(file, "{}{}{}\"}}", prefix, "x".repeat(padding), stored).unwrap();
            drop(file);
            let value = match encoding {
                Encoding::Base64 => TOKEN,
                _ => &value,
            };
            let report = scan(root.path(), vec![Secret::new("id", value.to_owned())]).unwrap();
            let hits: Vec<_> = report.secrets[0]
                .locations
                .iter()
                .filter(|hit| hit.file == path)
                .collect();
            assert_eq!(hits.len(), 1, "{name}, split {split}");
            assert_eq!(hits[0].byte_offset, (CHUNK * 2 - split) as u64);
            assert_eq!(hits[0].line, 1);
            assert_eq!(hits[0].encoding, encoding);
        }
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn streams_large_lines_and_legacy_json_and_keeps_original_line_numbers() {
    let root = tempfile::tempdir().unwrap();
    let messages = root.path().join("messages.jsonl");
    let mut file = fs::File::create(&messages).unwrap();
    writeln!(file, "{{\"content\":\"unrelated\"}}").unwrap();
    write!(
        file,
        "{{\"content\":\"{} {TOKEN} tail\"}}",
        "é".repeat(CHUNK * 8)
    )
    .unwrap();
    drop(file);
    let legacy = root.path().join("session_legacy.json");
    let legacy_text = format!(
        "{{\n  \"metadata\": {{}},\n  \"messages\": [{{\"role\":\"tool\",\"content\":\"{TOKEN}\"}}]\n}}"
    );
    fs::write(&legacy, &legacy_text).unwrap();
    let report = scan(root.path(), input()).unwrap();
    assert_eq!(report.secrets[0].locations.len(), 2);
    assert_eq!(
        report.secrets[0]
            .locations
            .iter()
            .find(|hit| hit.file == messages)
            .unwrap()
            .line,
        2
    );
    assert_eq!(
        report.secrets[0]
            .locations
            .iter()
            .find(|hit| hit.file == legacy)
            .unwrap()
            .line,
        3
    );
    let result = redact(&report);
    assert_eq!(result.redacted_files.len(), 2);
    assert!(result.skipped_files.is_empty());
    assert_eq!(scan(root.path(), input()).unwrap().secrets[0].locations, []);
    for line in fs::read_to_string(messages).unwrap().lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    serde_json::from_slice::<serde_json::Value>(&fs::read(legacy).unwrap()).unwrap();
}

#[test]
fn finds_padded_and_unpadded_standard_and_url_safe_base64_once_each() {
    let root = tempfile::tempdir().unwrap();
    let value = "abcdefgh??>>>>";
    let encodings: Vec<_> = [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
        .iter()
        .map(|engine| engine.encode(value))
        .collect();
    assert!(encodings[0].contains('/') || encodings[0].contains('+'));
    assert!(encodings[0].ends_with('='));
    let path = root.path().join("messages.jsonl");
    let records: Vec<_> = encodings
        .iter()
        .map(|encoded| format!("{{\"content\":\"{encoded}\"}}\n"))
        .collect();
    fs::write(&path, records.concat()).unwrap();
    let report = scan(root.path(), vec![Secret::new("base64", value.to_owned())]).unwrap();
    assert_eq!(report.secrets[0].locations.len(), 4);
    for (hit, encoded) in report.secrets[0].locations.iter().zip(&encodings) {
        assert_eq!(hit.byte_length, encoded.len() as u64);
        assert_eq!(hit.encoding, Encoding::Base64);
    }
    assert_eq!(redact(&report).redacted_files.len(), 1);
    assert_eq!(
        fs::read_to_string(path)
            .unwrap()
            .matches("[redacted]")
            .count(),
        4
    );
}

#[test]
fn a_secret_longer_than_the_read_chunk_is_found_and_redacted() {
    let root = tempfile::tempdir().unwrap();
    let value = "q".repeat(CHUNK + 10);
    let path = root.path().join("meta.json");
    fs::write(&path, format!("{{\"content\":\"{value}\"}}")).unwrap();
    let report = scan(root.path(), vec![Secret::new("long", value)]).unwrap();
    assert_eq!(report.secrets[0].locations.len(), 1);
    assert_eq!(redact(&report).redacted_files.len(), 1);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "{\"content\":\"[redacted]\"}"
    );
}

#[test]
fn recognizes_url_query_forms_and_eight_character_unicode_values() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("messages.jsonl");
    fs::write(
        &path,
        concat!(
            "{\"content\":\"fixture+value%2B%2F%3F\"}\r\n",
            "{\"content\":\"éééééééé\"}\r\n",
        ),
    )
    .unwrap();
    let report = scan(
        root.path(),
        vec![
            Secret::new("form", "fixture value+/?".to_owned()),
            Secret::new("unicode", "éééééééé".to_owned()),
        ],
    )
    .unwrap();
    assert_eq!(report.secrets[0].locations.len(), 1);
    assert_eq!(
        report.secrets[0].locations[0].encoding,
        Encoding::UrlEncoded
    );
    assert_eq!(report.secrets[1].locations.len(), 1);
    assert_eq!(report.secrets[1].locations[0].line, 2);
    assert_eq!(redact(&report).redacted_files.len(), 1);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "{\"content\":\"[redacted]\"}\r\n{\"content\":\"[redacted]\"}\r\n"
    );
}
