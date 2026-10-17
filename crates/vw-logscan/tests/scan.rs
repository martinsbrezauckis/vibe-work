//! Synthetic Vibe session fixtures exercise encodings, masking and false-positive exclusions.
//! Depends on vw-logscan and tempfile; never reads a user's session logs or vault.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, path::Path};

use base64::{Engine, engine::general_purpose::STANDARD};
use vw_logscan::{Encoding, Secret, SecretSkipReason, scan};

const TOKEN: &str = "ghp_fixtureOnly012345";
const ESCAPED: &str = "fixture\"\\\nvalue+/?";

fn secret(id: &str, value: &str) -> Secret {
    Secret::new(id, value.to_owned())
}

fn fixtures() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("session_20261014_120000_12345678");
    fs::create_dir(&session).unwrap();
    fs::write(
        session.join("meta.json"),
        include_bytes!("fixtures/meta.json"),
    )
    .unwrap();
    fs::write(
        session.join("messages.jsonl"),
        include_bytes!("fixtures/messages.jsonl"),
    )
    .unwrap();
    root
}

#[test]
fn finds_every_encoding_with_exact_original_locations() {
    let root = fixtures();
    let report = scan(
        root.path(),
        vec![secret("token", TOKEN), secret("escaped", ESCAPED)],
    )
    .unwrap();
    assert!(report.skipped_files.is_empty());
    let tokens = &report.secrets[0].locations;
    assert_eq!(tokens.len(), 3);
    assert!(tokens.iter().any(|hit| hit.encoding == Encoding::AsIs));
    assert!(tokens.iter().any(|hit| hit.encoding == Encoding::Base64));
    let escaped = &report.secrets[1].locations;
    assert_eq!(escaped.len(), 2);
    assert!(
        escaped
            .iter()
            .any(|hit| hit.encoding == Encoding::JsonEscaped)
    );
    assert!(
        escaped
            .iter()
            .any(|hit| hit.encoding == Encoding::UrlEncoded)
    );
    for hit in tokens.iter().chain(escaped) {
        let bytes = fs::read(&hit.file).unwrap();
        let offset = usize::try_from(hit.byte_offset).unwrap();
        let length = usize::try_from(hit.byte_length).unwrap();
        let expected = match hit.encoding {
            Encoding::AsIs => TOKEN.to_owned(),
            Encoding::Base64 => STANDARD.encode(TOKEN),
            Encoding::JsonEscaped => {
                let encoded = serde_json::to_string(ESCAPED).unwrap();
                encoded[1..encoded.len() - 1].to_owned()
            }
            Encoding::UrlEncoded => "fixture%22%5C%0Avalue%2B%2F%3F".to_owned(),
        };
        assert_eq!(&bytes[offset..offset + length], expected.as_bytes());
        assert_eq!(
            hit.line,
            bytes[..offset].split(|&b| b == b'\n').count() as u64
        );
    }
}

#[test]
fn report_and_debug_output_only_show_masked_values() {
    let root = fixtures();
    let input = secret("opaque-id", TOKEN);
    assert!(!format!("{input:?}").contains(TOKEN));
    let report = scan(root.path(), vec![input]).unwrap();
    assert_eq!(report.secrets[0].masked_preview, "ghp_****");
    let serialized = serde_json::to_string(&report).unwrap();
    for text in [serialized, format!("{report:?}")] {
        assert!(!text.contains(TOKEN));
        assert!(!text.contains(&STANDARD.encode(TOKEN)));
        assert!(!text.contains("surrounding-context"));
        assert!(text.contains("opaque-id"));
    }
}

#[test]
fn skips_short_values_and_does_not_match_similar_text() {
    let root = fixtures();
    let report = scan(
        root.path(),
        vec![
            secret("short", "fixture"),
            secret("unicode-short", "ééééééé"),
            secret("absent", "ghp_fixtureOnly012346"),
        ],
    )
    .unwrap();
    for result in &report.secrets[..2] {
        assert_eq!(result.skipped, Some(SecretSkipReason::TooShort));
        assert_eq!(result.locations, []);
    }
    assert_eq!(report.secrets[2].skipped, None);
    assert_eq!(report.secrets[2].locations, []);
}

#[test]
fn a_value_equal_to_a_recognized_prefix_is_fully_masked() {
    let root = tempfile::tempdir().unwrap();
    let report = scan(root.path(), vec![secret("id", "github_pat_")]).unwrap();
    assert_eq!(report.secrets[0].masked_preview, "****");
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("github_pat_")
    );
}

#[test]
fn ignores_non_log_files_and_does_not_follow_symlinks() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("image.bin"), TOKEN).unwrap();
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("meta.json"), format!("\"{TOKEN}\"")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("meta.json"),
            root.path().join("linked.json"),
        )
        .unwrap();
        assert_eq!(
            scan(root.path(), vec![secret("id", TOKEN)])
                .unwrap()
                .secrets[0]
                .locations,
            []
        );
    }
    assert_eq!(
        scan(root.path(), vec![secret("id", TOKEN)])
            .unwrap()
            .secrets[0]
            .locations,
        []
    );
    assert!(
        scan(
            Path::new("missing-session-fixture-folder"),
            vec![secret("id", TOKEN)]
        )
        .is_err()
    );
}
