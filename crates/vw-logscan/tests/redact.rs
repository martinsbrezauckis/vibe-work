//! Atomic redaction falsifiers on synthetic session files. Uses the public API and JSON parsers.
//! Never reads real session logs or secret stores; replacements must preserve untouched files.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;

use vw_logscan::{FileSkipReason, Secret, redact, scan};

const TOKEN: &str = "ghp_fixtureOnly012345";
const ESCAPED: &str = "fixture\"\\\nvalue+/?";

fn secrets() -> Vec<Secret> {
    vec![
        Secret::new("token", TOKEN.to_owned()),
        Secret::new("escaped", ESCAPED.to_owned()),
    ]
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("meta.json"),
        include_bytes!("fixtures/meta.json"),
    )
    .unwrap();
    fs::write(
        root.path().join("messages.jsonl"),
        include_bytes!("fixtures/messages.jsonl"),
    )
    .unwrap();
    root
}

#[test]
fn redacts_every_encoding_and_preserves_json_and_jsonl() {
    let root = fixture();
    let report = scan(root.path(), secrets()).unwrap();
    let result = redact(&report);
    assert_eq!(result.redacted_files.len(), 2);
    assert!(result.skipped_files.is_empty());
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("meta.json")).unwrap()).unwrap();
    assert_eq!(metadata["config"]["fixture_key"], "[redacted]");
    let messages = fs::read_to_string(root.path().join("messages.jsonl")).unwrap();
    assert_eq!(messages.lines().count(), 4);
    for line in messages.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(parsed["content"].as_str().unwrap().contains("[redacted]"));
    }
    assert!(messages.contains("surrounding-context"));
    let after = scan(root.path(), secrets()).unwrap();
    assert!(
        after
            .secrets
            .iter()
            .all(|secret| secret.locations.is_empty())
    );
    let mut files: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    files.sort();
    assert_eq!(files, vec!["messages.jsonl", "meta.json"]);
}

#[test]
fn changed_file_is_skipped_while_unchanged_file_is_redacted() {
    let root = fixture();
    let report = scan(root.path(), secrets()).unwrap();
    let path = root.path().join("messages.jsonl");
    let changed = format!("{{\"content\":\"{TOKEN} newly appended\"}}\n");
    fs::write(&path, &changed).unwrap();
    let result = redact(&report);
    assert_eq!(result.redacted_files, vec![root.path().join("meta.json")]);
    assert_eq!(result.skipped_files.len(), 1);
    assert_eq!(
        result.skipped_files[0].reason,
        FileSkipReason::ChangedSinceScan
    );
    assert_eq!(fs::read_to_string(path).unwrap(), changed);
}

#[test]
fn same_length_change_is_detected_without_relying_on_timestamp() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("meta.json");
    let original = format!("{{\"content\":\"{TOKEN}\",\"other\":1}}");
    fs::write(&path, &original).unwrap();
    let report = scan(root.path(), secrets()).unwrap();
    let modified = original.replace("\"other\":1", "\"other\":2");
    assert_eq!(original.len(), modified.len());
    fs::write(&path, &modified).unwrap();
    let result = redact(&report);
    assert_eq!(result.redacted_files, [] as [std::path::PathBuf; 0]);
    assert_eq!(
        result.skipped_files[0].reason,
        FileSkipReason::ChangedSinceScan
    );
    assert_eq!(fs::read_to_string(path).unwrap(), modified);
}

#[test]
fn overlapping_secrets_and_duplicate_values_are_redacted_once() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("messages.jsonl");
    fs::write(&path, "{\"content\":\"abcdefghijk aaaaaaaaaaaa\"}\n").unwrap();
    let report = scan(
        root.path(),
        vec![
            Secret::new("long", "abcdefghijk".to_owned()),
            Secret::new("suffix", "defghijk".to_owned()),
            Secret::new("duplicate", "abcdefghijk".to_owned()),
            Secret::new("repeated", "aaaaaaaa".to_owned()),
        ],
    )
    .unwrap();
    assert_eq!(report.secrets[3].locations.len(), 5);
    let result = redact(&report);
    assert_eq!(result.redacted_files.len(), 1);
    assert!(result.skipped_files.is_empty());
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "{\"content\":\"[redacted] [redacted]\"}\n"
    );
}

#[test]
fn invalid_json_or_syntax_matches_never_replace_the_original() {
    for (file, text, value) in [
        ("meta.json", "{\"content\":\"ghp_fixtureOnly012345\"", TOKEN),
        (
            "messages.jsonl",
            "{\"content\":\"ghp_fixtureOnly012345\"} {}\n",
            TOKEN,
        ),
        ("meta.json", "{\"count\":12345678}", "12345678"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(file);
        fs::write(&path, text).unwrap();
        let report = scan(root.path(), vec![Secret::new("id", value.to_owned())]).unwrap();
        assert_eq!(report.secrets[0].locations.len(), 1);
        let result = redact(&report);
        assert_eq!(result.redacted_files, [] as [std::path::PathBuf; 0]);
        assert_eq!(result.skipped_files[0].reason, FileSkipReason::InvalidJson);
        assert_eq!(fs::read_to_string(path).unwrap(), text);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

#[test]
fn truncated_and_deleted_files_are_reported_without_touching_others() {
    let root = fixture();
    let report = scan(root.path(), secrets()).unwrap();
    fs::write(root.path().join("meta.json"), "{}").unwrap();
    fs::remove_file(root.path().join("messages.jsonl")).unwrap();
    let result = redact(&report);
    assert_eq!(result.redacted_files, [] as [std::path::PathBuf; 0]);
    assert_eq!(result.skipped_files.len(), 2);
    assert!(
        result
            .skipped_files
            .iter()
            .any(|issue| issue.reason == FileSkipReason::ChangedSinceScan)
    );
    assert!(
        result
            .skipped_files
            .iter()
            .any(|issue| issue.reason == FileSkipReason::Io)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("meta.json")).unwrap(),
        "{}"
    );
}

#[cfg(unix)]
#[test]
fn preserves_owner_only_file_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = fixture();
    let path = root.path().join("meta.json");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let report = scan(root.path(), secrets()).unwrap();
    assert_eq!(redact(&report).redacted_files.len(), 2);
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
