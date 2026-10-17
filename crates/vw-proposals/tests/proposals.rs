//! End-to-end proposal fixtures; uses tempdir and the public core API without a window.
//! Falsifiers assert both the requested outcome and untouched files on refusals.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;

mod support;

use vw_proposals::{Change, Decision, Error, Proposal, Request, Status};

fn request(changes: Vec<Change>) -> Request {
    Request {
        session_id: "session-1".into(),
        reason: "Update the protected agent settings".into(),
        changes,
    }
}

fn add(path: &str, content: &[u8]) -> Change {
    Change::Add {
        path: path.into(),
        content: content.into(),
    }
}

fn edit(path: &str, content: &[u8]) -> Change {
    Change::Edit {
        path: path.into(),
        content: content.into(),
    }
}

#[test]
fn approve_add_edit_remove_only_after_review() {
    let root = support::tempdir();
    fs::create_dir(root.path().join("profiles")).unwrap();
    fs::write(root.path().join("settings"), b"old settings\n").unwrap();
    fs::write(root.path().join("obsolete"), b"remove this\n").unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![
            add("profiles/new", b"new profile\n"),
            edit("settings", b"new settings\n"),
            Change::Remove {
                path: "obsolete".into(),
            },
        ]),
    )
    .unwrap();
    assert!(!root.path().join("profiles/new").exists());
    assert_eq!(
        fs::read(root.path().join("settings")).unwrap(),
        b"old settings\n"
    );
    assert!(root.path().join("obsolete").exists());
    let review = proposal.review();
    assert!(review[0].diff.as_ref().unwrap().contains("+new profile"));
    assert!(review[1].diff.as_ref().unwrap().contains("-old settings"));
    assert!(review[2].diff.as_ref().unwrap().contains("-remove this"));
    let record = proposal.apply(Decision::Approve).unwrap();
    assert_eq!(record.decision, Status::Approved);
    assert_eq!(
        record.applied,
        vec![
            PathBuf::from("profiles/new"),
            "settings".into(),
            "obsolete".into()
        ]
    );
    assert_eq!(
        fs::read(root.path().join("profiles/new")).unwrap(),
        b"new profile\n"
    );
    assert_eq!(
        fs::read(root.path().join("settings")).unwrap(),
        b"new settings\n"
    );
    assert!(!root.path().join("obsolete").exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    assert_eq!(
        fs::read_dir(root.path().join("profiles")).unwrap().count(),
        1
    );
}

#[test]
fn changed_last_base_refuses_the_entire_proposal() {
    let root = support::tempdir();
    fs::write(root.path().join("edited"), b"original").unwrap();
    fs::write(root.path().join("removed"), b"original").unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![
            add("new", b"new"),
            edit("edited", b"proposed"),
            Change::Remove {
                path: "removed".into(),
            },
        ]),
    )
    .unwrap();
    fs::write(root.path().join("removed"), b"changed!").unwrap();
    let error = proposal.apply(Decision::Approve).unwrap_err();
    assert!(matches!(error.error, Error::Stale { ref path } if path == &PathBuf::from("removed")));
    assert_eq!(error.record.decision, Status::Stale);
    assert_eq!(error.record.applied, Vec::<PathBuf>::new());
    assert!(!root.path().join("new").exists());
    assert_eq!(fs::read(root.path().join("edited")).unwrap(), b"original");
    assert_eq!(fs::read(root.path().join("removed")).unwrap(), b"changed!");
}

#[test]
fn same_size_edit_is_stale_and_review_keeps_the_captured_bytes() {
    let root = support::tempdir();
    fs::write(root.path().join("settings"), b"old\n").unwrap();
    let proposal = Proposal::new(root.path(), request(vec![edit("settings", b"new\n")])).unwrap();
    let review = proposal.review();
    fs::write(root.path().join("settings"), b"bad\n").unwrap();
    assert_eq!(proposal.review(), review);
    let error = proposal.apply(Decision::Approve).unwrap_err();
    assert!(matches!(error.error, Error::Stale { .. }));
    assert_eq!(fs::read(root.path().join("settings")).unwrap(), b"bad\n");
}

#[test]
fn add_must_be_absent_at_capture_and_at_approval() {
    let root = support::tempdir();
    fs::write(root.path().join("exists"), b"existing").unwrap();
    assert!(matches!(
        Proposal::new(root.path(), request(vec![add("exists", b"overwrite")])),
        Err(Error::AlreadyExists { .. })
    ));
    let proposal = Proposal::new(root.path(), request(vec![add("new", b"proposed")])).unwrap();
    fs::write(root.path().join("new"), b"user added").unwrap();
    let error = proposal.apply(Decision::Approve).unwrap_err();
    assert!(matches!(error.error, Error::Stale { .. }));
    assert_eq!(fs::read(root.path().join("new")).unwrap(), b"user added");
}

#[test]
fn missing_edit_or_remove_is_refused_and_disappearance_is_stale() {
    let root = support::tempdir();
    for change in [
        edit("missing", b"new"),
        Change::Remove {
            path: "missing".into(),
        },
    ] {
        assert!(matches!(
            Proposal::new(root.path(), request(vec![change])),
            Err(Error::MissingFile { .. })
        ));
    }
    fs::write(root.path().join("settings"), b"original").unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![add("new", b"new"), edit("settings", b"proposed")]),
    )
    .unwrap();
    fs::remove_file(root.path().join("settings")).unwrap();
    let error = proposal.apply(Decision::Approve).unwrap_err();
    assert!(matches!(error.error, Error::Stale { .. }));
    assert!(!root.path().join("new").exists());
}

#[test]
fn decline_has_no_effect_even_when_the_base_changed() {
    let root = support::tempdir();
    fs::write(root.path().join("settings"), b"old").unwrap();
    fs::write(root.path().join("removed"), b"keep").unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![
            add("new", b"new"),
            edit("settings", b"proposed"),
            Change::Remove {
                path: "removed".into(),
            },
        ]),
    )
    .unwrap();
    fs::write(root.path().join("settings"), b"user edit").unwrap();
    let record = proposal.apply(Decision::Decline).unwrap();
    assert_eq!(record.decision, Status::Declined);
    assert_eq!(record.applied, Vec::<PathBuf>::new());
    assert!(!root.path().join("new").exists());
    assert_eq!(
        fs::read(root.path().join("settings")).unwrap(),
        b"user edit"
    );
    assert_eq!(fs::read(root.path().join("removed")).unwrap(), b"keep");
}

#[test]
fn deleted_parent_of_an_existing_base_is_stale_and_applies_nothing() {
    let root = support::tempdir();
    fs::create_dir(root.path().join("profiles")).unwrap();
    fs::write(root.path().join("profiles/base"), b"original").unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![add("new", b"new"), edit("profiles/base", b"proposed")]),
    )
    .unwrap();
    fs::remove_dir_all(root.path().join("profiles")).unwrap();
    let error = proposal.apply(Decision::Approve).unwrap_err();
    assert!(
        matches!(error.error, Error::Stale { ref path } if path == &PathBuf::from("profiles/base"))
    );
    assert_eq!(error.record.decision, Status::Stale);
    assert_eq!(error.record.applied, Vec::<PathBuf>::new());
    assert!(!root.path().join("new").exists());
    assert!(!root.path().join("profiles").exists());
}
