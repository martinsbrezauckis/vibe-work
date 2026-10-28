//! Missing skill parents are captured without mutation and created only after a user approves.
//! Fixtures come from `support::tempdir` (the resolved temporary folder): proposals refuse a root below a
//! link, and the macOS temporary folder lies below `/var`.
mod support;

use vw_proposals::{Change, Decision, Proposal, Request};
fn request() -> Request {
    Request {
        session_id: "session".into(),
        reason: "Create a skill".into(),
        changes: vec![Change::Add {
            path: "skills/new/SKILL.md".into(),
            content: b"skill\n".to_vec(),
        }],
    }
}
#[test]
fn new_parent_waits_for_approval_and_decline_creates_nothing() {
    let root = support::tempdir();
    Proposal::with_new_directories(root.path(), request())
        .unwrap()
        .apply(Decision::Decline)
        .unwrap();
    assert!(!root.path().join("skills").exists());
    let p = Proposal::with_new_directories(root.path(), request()).unwrap();
    assert!(!root.path().join("skills").exists());
    p.apply(Decision::Approve).unwrap();
    assert_eq!(
        std::fs::read(root.path().join("skills/new/SKILL.md")).unwrap(),
        b"skill\n"
    );
}
#[cfg(unix)]
#[test]
fn a_new_parent_replaced_by_a_link_never_applies() {
    let root = support::tempdir();
    let outside = support::tempdir();
    let p = Proposal::with_new_directories(root.path(), request()).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("skills")).unwrap();
    assert!(p.apply(Decision::Approve).is_err());
    assert!(!outside.path().join("new").exists());
}
