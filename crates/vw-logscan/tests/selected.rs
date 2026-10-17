//! Selected-file scanning and redaction authority, using invented fixture values only.
//! Depends on vw-logscan and tempfile; proves integration can preserve manual-only matches.

use std::collections::BTreeSet;
use vw_logscan::{Secret, redact, scan_files};

#[test]
fn selected_file_and_secret_leave_other_files_and_password_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("messages.jsonl");
    let other = dir.path().join("other.json");
    let text = r#"{"key":"invented-token","password":"ordinary-password"}"#;
    std::fs::write(&path, text).unwrap();
    std::fs::write(&other, text).unwrap();
    let secrets = || {
        vec![
            Secret::new("key", "invented-token".to_owned()),
            Secret::new("password", "ordinary-password".to_owned()),
        ]
    };
    let report = scan_files(vec![path.clone()], secrets()).unwrap();
    assert_eq!(report.secrets[0].locations.len(), 1);
    let selected = report.select_secrets(&BTreeSet::from(["key".to_owned()]));
    assert_eq!(redact(&selected).redacted_files.len(), 1);
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("ordinary-password"));
    assert!(!after.contains("invented-token"));
    assert_eq!(std::fs::read_to_string(other).unwrap(), text);
    serde_json::from_str::<serde_json::Value>(&after).unwrap();
    assert_eq!(
        scan_files(vec![path], secrets()).unwrap().secrets[0]
            .locations
            .len(),
        0
    );
}

#[test]
fn a_selected_file_that_disappeared_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gone.json");
    let report = scan_files(
        vec![path.clone()],
        vec![Secret::new("key", "invented-token".to_owned())],
    )
    .unwrap();
    assert_eq!(report.skipped_files.len(), 1);
    assert_eq!(report.skipped_files[0].file, path);
}
