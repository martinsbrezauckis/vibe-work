//! Review and bounded-record fixtures; uses the public API with isolated files.
//! Falsifiers catch binary leakage, truncated live reviews and content-bearing records.

#![allow(clippy::unwrap_used)]

use std::fs;

use tempfile::TempDir;
use vw_proposals::{
    Change, ContentKind, Decision, Error, Kind, MAX_REASON_BYTES, MAX_RECORD_DIFF_BYTES, Proposal,
    Record, Request, Status,
};

fn request(changes: Vec<Change>) -> Request {
    Request {
        session_id: "record-session".into(),
        reason: "Update the protected profile".into(),
        changes,
    }
}

fn add(path: &str, content: Vec<u8>) -> Change {
    Change::Add {
        path: path.into(),
        content,
    }
}

#[test]
fn text_review_has_unified_headers_counts_sizes_and_missing_newline_markers() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("edited"), b"before\nkept\n").unwrap();
    fs::write(root.path().join("removed"), b"gone\n").unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![
            add("added", b"new".to_vec()),
            Change::Edit {
                path: "edited".into(),
                content: b"after\nkept\n".to_vec(),
            },
            Change::Remove {
                path: "removed".into(),
            },
        ]),
    )
    .unwrap();
    let review = proposal.review();
    assert_eq!(review[0].kind, Kind::Add);
    assert_eq!(review[0].content_kind, ContentKind::Text);
    assert_eq!((review[0].before_bytes, review[0].after_bytes), (0, 3));
    assert_eq!((review[0].added_lines, review[0].removed_lines), (1, 0));
    let diff = review[0].diff.as_ref().unwrap();
    assert!(diff.starts_with("--- /dev/null\n+++ b/added\n@@"));
    assert!(diff.contains("+new\n\\ No newline at end of file\n"));
    assert_eq!((review[1].added_lines, review[1].removed_lines), (1, 1));
    assert!(
        review[1]
            .diff
            .as_ref()
            .unwrap()
            .contains("--- a/edited\n+++ b/edited\n")
    );
    assert!(review[1].diff.as_ref().unwrap().contains(" kept\n"));
    assert_eq!((review[2].before_bytes, review[2].after_bytes), (5, 0));
    assert_eq!((review[2].added_lines, review[2].removed_lines), (0, 1));
    assert!(
        review[2]
            .diff
            .as_ref()
            .unwrap()
            .starts_with("--- a/removed\n+++ /dev/null\n")
    );
}

#[test]
fn binary_reviews_never_include_diffs_or_lossy_file_bytes() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("edited"), b"text\n").unwrap();
    fs::write(root.path().join("removed"), [0xff, 0xfe]).unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![
            add("added", b"binary\0bytes".to_vec()),
            Change::Edit {
                path: "edited".into(),
                content: vec![0xff],
            },
            Change::Remove {
                path: "removed".into(),
            },
        ]),
    )
    .unwrap();
    let review = proposal.review();
    for file in &review {
        assert_eq!(file.content_kind, ContentKind::Binary);
        assert!(file.diff.is_none());
        assert_eq!((file.added_lines, file.removed_lines), (0, 0));
    }
    assert_eq!(review[0].summary(), "binary, 0 bytes -> 12 bytes");
    assert_eq!(review[1].summary(), "binary, 5 bytes -> 1 bytes");
    assert_eq!(review[2].summary(), "binary, 2 bytes -> 0 bytes");
    let record = proposal.apply(Decision::Decline).unwrap();
    let json = serde_json::to_string(&record).unwrap();
    assert!(!json.contains("binary\\u0000bytes"));
    for file in record.files {
        assert!(file.diff.is_none());
        assert_eq!(file.diff_bytes, 0);
    }
}

#[test]
fn binary_old_side_makes_text_replacement_binary_and_bytes_apply_exactly() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("edited"), b"old\0bytes").unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![
            add("binary", vec![0xff, 0, 0xfe]),
            Change::Edit {
                path: "edited".into(),
                content: b"text\n".to_vec(),
            },
        ]),
    )
    .unwrap();
    assert_eq!(proposal.review()[1].content_kind, ContentKind::Binary);
    proposal.apply(Decision::Approve).unwrap();
    assert_eq!(
        fs::read(root.path().join("binary")).unwrap(),
        [0xff, 0, 0xfe]
    );
    assert_eq!(fs::read(root.path().join("edited")).unwrap(), b"text\n");
}

#[test]
fn record_roundtrips_identity_metadata_decision_time_and_diff_sizes() {
    let root = TempDir::new().unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![add("new", b"proposed\n".to_vec())]),
    )
    .unwrap();
    let id = proposal.id().to_owned();
    let created = proposal.created_at_ms();
    assert_eq!(id.len(), 32);
    assert!(created > 0);
    assert_eq!(proposal.session_id(), "record-session");
    assert_eq!(proposal.reason(), "Update the protected profile");
    let diff = proposal.review()[0].diff.clone().unwrap();
    let record = proposal.apply(Decision::Decline).unwrap();
    assert_eq!(record.proposal_id, id);
    assert_eq!(record.session_id, "record-session");
    assert_eq!(record.created_at_ms, created);
    assert!(record.decided_at_ms >= created);
    assert_eq!(record.files[0].diff_bytes, diff.len());
    assert_eq!(record.files[0].diff.as_deref(), Some(diff.as_str()));
    assert!(!record.files[0].diff_truncated);
    let value = serde_json::to_value(&record).unwrap();
    assert_eq!(value["decision"], "declined");
    assert_eq!(value["files"][0]["kind"], "add");
    let object = value["files"][0].as_object().unwrap();
    assert!(!object.contains_key("content"));
    assert!(!object.contains_key("base"));
    assert!(!object.contains_key("base_hash"));
    assert!(!object.contains_key("permissions"));
    assert_eq!(serde_json::from_value::<Record>(value).unwrap(), record);
    assert!(!root.path().join("new").exists());
}

#[test]
fn record_caps_combined_diff_bytes_on_utf8_boundaries_but_not_live_review() {
    let root = TempDir::new().unwrap();
    let text = format!("{}\n", "é".repeat(MAX_RECORD_DIFF_BYTES));
    let proposal = Proposal::new(
        root.path(),
        request(vec![
            add("first", text.as_bytes().to_vec()),
            add("second", text.into_bytes()),
        ]),
    )
    .unwrap();
    let review = proposal.review();
    assert!(
        review
            .iter()
            .all(|file| file.diff.as_ref().unwrap().len() > MAX_RECORD_DIFF_BYTES)
    );
    let record = proposal.apply(Decision::Decline).unwrap();
    let retained: usize = record
        .files
        .iter()
        .map(|file| file.diff.as_ref().map_or(0, String::len))
        .sum();
    assert!(retained <= MAX_RECORD_DIFF_BYTES);
    assert!(retained >= MAX_RECORD_DIFF_BYTES - 3);
    for (file, original) in record.files.iter().zip(&review) {
        assert!(file.diff_truncated);
        assert_eq!(file.diff_bytes, original.diff.as_ref().unwrap().len());
        assert!(
            original
                .diff
                .as_ref()
                .unwrap()
                .starts_with(file.diff.as_ref().unwrap())
        );
    }
    let json = serde_json::to_string(&record).unwrap();
    assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), record);
}

#[test]
fn unchanged_file_contents_outside_diff_context_are_not_recorded() {
    let root = TempDir::new().unwrap();
    let old = format!("old\n{}outside-diff-context\n", "context\n".repeat(20));
    let new = old.replacen("old", "new", 1);
    fs::write(root.path().join("settings"), old).unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![Change::Edit {
            path: "settings".into(),
            content: new.into_bytes(),
        }]),
    )
    .unwrap();
    let record = proposal.apply(Decision::Decline).unwrap();
    let json = serde_json::to_string(&record).unwrap();
    assert!(!json.contains("outside-diff-context"));
    assert!(json.contains("-old\\n+new"));
}

#[test]
fn stale_and_approved_records_roundtrip_their_decisions() {
    let root = TempDir::new().unwrap();
    let first = Proposal::new(
        root.path(),
        request(vec![add("first", b"proposed".to_vec())]),
    )
    .unwrap();
    let second = Proposal::new(
        root.path(),
        request(vec![add("second", b"proposed".to_vec())]),
    )
    .unwrap();
    assert_ne!(first.id(), second.id());
    let approved = first.apply(Decision::Approve).unwrap();
    fs::write(root.path().join("second"), b"user file").unwrap();
    let stale = second.apply(Decision::Approve).unwrap_err().record;
    for (record, expected) in [(approved, Status::Approved), (*stale, Status::Stale)] {
        assert_eq!(record.decision, expected);
        let json = serde_json::to_string(&record).unwrap();
        assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), record);
    }
}

#[test]
fn reason_is_bounded_plain_text_and_every_proposal_requires_session_metadata() {
    let root = TempDir::new().unwrap();
    for reason in [
        String::new(),
        " ".into(),
        "x".repeat(MAX_REASON_BYTES + 1),
        "é".repeat(MAX_REASON_BYTES / 2 + 1),
        "control\0character".into(),
    ] {
        let mut changes = request(vec![add("new", vec![])]);
        changes.reason = reason;
        assert!(matches!(
            Proposal::new(root.path(), changes),
            Err(Error::InvalidReason)
        ));
    }
    for session in ["", " ", "session\nother"] {
        let mut changes = request(vec![add("new", vec![])]);
        changes.session_id = session.into();
        assert!(matches!(
            Proposal::new(root.path(), changes),
            Err(Error::InvalidSessionId)
        ));
    }
    assert!(matches!(
        Proposal::new(root.path(), request(vec![])),
        Err(Error::EmptyChanges)
    ));
    let mut changes = request(vec![add("new", vec![])]);
    changes.reason = "é".repeat(MAX_REASON_BYTES / 2);
    assert_eq!(
        Proposal::new(root.path(), changes).unwrap().reason().len(),
        MAX_REASON_BYTES
    );
    let mut changes = request(vec![add("new", vec![])]);
    changes.reason = "<b>literal plain text</b>\nnext line\ttext".into();
    assert_eq!(
        Proposal::new(root.path(), changes).unwrap().reason(),
        "<b>literal plain text</b>\nnext line\ttext"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
