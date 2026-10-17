//! Page-store mutation guards. Uses real private files and filesystem errors; called by cargo test.
//! Failed reads or failed corrupt-file moves must leave the original bytes intact.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use vw_pages::{AnswerStore, AnswerValue, CommentStore, PagePath, parse_html, project_key};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vw-page-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, kind: &str) -> PathBuf {
        self.0
            .join(kind)
            .join(format!("{}.json", project_key(&self.0)))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn page() -> PagePath {
    PagePath::parse("plan.html").unwrap()
}

fn set_answer(store: &AnswerStore, project: &Path) -> Result<(), vw_pages::AnswerError> {
    let facts = parse_html(
        r#"<div class="hb-input" id="why" data-input="text"><p class="hb-question">Why?</p></div>"#,
    );
    store
        .set(
            project,
            &page(),
            &facts,
            "why",
            &AnswerValue::Text {
                text: "Keep it".into(),
            },
        )
        .map(|_| ())
}

#[test]
fn missing_stores_allow_the_first_save() {
    let f = Fixture::new();
    let comments = CommentStore::new(f.0.join("comments"));
    comments
        .add(&f.0, &page(), ("s1", "Goal"), "Keep it", None)
        .unwrap();
    assert_eq!(comments.list(&f.0, &page(), None).1.len(), 1);
    let answers = AnswerStore::new(f.0.join("answers"));
    set_answer(&answers, &f.0).unwrap();
    assert_eq!(answers.list(&f.0, None).len(), 1);
}

#[test]
fn unreadable_text_is_never_replaced() {
    let f = Fixture::new();
    let bytes = [0xff, 0xfe, 0xfd];
    for kind in ["comments", "answers"] {
        std::fs::create_dir_all(f.0.join(kind)).unwrap();
        std::fs::write(f.file(kind), bytes).unwrap();
    }
    let comments = CommentStore::new(f.0.join("comments"));
    assert!(
        comments
            .add(&f.0, &page(), ("s1", "Goal"), "New", None)
            .is_err()
    );
    assert!(
        comments
            .delete(&f.0, "existing")
            .unwrap_err()
            .to_string()
            .contains("could not be saved")
    );
    assert!(comments.mark_sent(&f.0, &["existing".into()]).is_err());
    let answers = AnswerStore::new(f.0.join("answers"));
    assert!(set_answer(&answers, &f.0).is_err());
    assert!(answers.clear(&f.0, &page(), "why").is_err());
    assert!(answers.mark_sent(&f.0, &page()).is_err());
    for kind in ["comments", "answers"] {
        assert_eq!(std::fs::read(f.file(kind)).unwrap(), bytes);
    }
}

#[cfg(unix)]
#[test]
fn permission_denied_refuses_mutations_and_keeps_original_bytes() {
    use std::os::unix::fs::PermissionsExt as _;
    let f = Fixture::new();
    let comments = CommentStore::new(f.0.join("comments"));
    let comment = comments
        .add(&f.0, &page(), ("s1", "Goal"), "Original", None)
        .unwrap();
    let answers = AnswerStore::new(f.0.join("answers"));
    set_answer(&answers, &f.0).unwrap();
    let originals = ["comments", "answers"].map(|kind| std::fs::read(f.file(kind)).unwrap());
    for kind in ["comments", "answers"] {
        std::fs::set_permissions(f.file(kind), std::fs::Permissions::from_mode(0o0)).unwrap();
        assert_eq!(
            std::fs::read(f.file(kind)).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
    assert!(
        comments
            .add(&f.0, &page(), ("s1", "Goal"), "New", None)
            .is_err()
    );
    assert!(
        comments
            .delete(&f.0, &comment.id)
            .unwrap_err()
            .to_string()
            .contains("could not be saved")
    );
    assert!(
        comments
            .mark_sent(&f.0, std::slice::from_ref(&comment.id))
            .is_err()
    );
    assert!(set_answer(&answers, &f.0).is_err());
    assert!(answers.clear(&f.0, &page(), "why").is_err());
    assert!(answers.mark_sent(&f.0, &page()).is_err());
    for kind in ["comments", "answers"] {
        std::fs::set_permissions(f.file(kind), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    for (kind, original) in ["comments", "answers"].into_iter().zip(originals) {
        assert_eq!(std::fs::read(f.file(kind)).unwrap(), original);
    }
    assert_eq!(comments.list(&f.0, &page(), None).1, [comment]);
    assert_eq!(answers.list(&f.0, None).len(), 1);
}

#[cfg(unix)]
#[test]
fn a_corrupt_store_must_be_moved_aside_before_writing() {
    use std::os::unix::fs::PermissionsExt as _;
    let f = Fixture::new();
    let bytes = b"{not json";
    for kind in ["comments", "answers"] {
        std::fs::create_dir_all(f.0.join(kind)).unwrap();
        std::fs::write(f.file(kind), bytes).unwrap();
        std::fs::set_permissions(f.0.join(kind), std::fs::Permissions::from_mode(0o500)).unwrap();
    }
    let comments = CommentStore::new(f.0.join("comments"));
    let answers = AnswerStore::new(f.0.join("answers"));
    // Display reads stay tolerant, but do not permit a later destructive save.
    assert_eq!(comments.list(&f.0, &page(), None).1.len(), 0);
    assert_eq!(answers.list(&f.0, None).len(), 0);
    assert!(
        comments
            .add(&f.0, &page(), ("s1", "Goal"), "New", None)
            .is_err()
    );
    assert!(set_answer(&answers, &f.0).is_err());
    for kind in ["comments", "answers"] {
        assert_eq!(std::fs::read(f.file(kind)).unwrap(), bytes);
        std::fs::set_permissions(f.0.join(kind), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    comments
        .add(&f.0, &page(), ("s1", "Goal"), "New", None)
        .unwrap();
    set_answer(&answers, &f.0).unwrap();
    for kind in ["comments", "answers"] {
        let aside = std::fs::read_dir(f.0.join(kind))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.file_name().unwrap().to_string_lossy().contains(".bad-"))
            .unwrap();
        assert_eq!(std::fs::read(aside).unwrap(), bytes);
    }
}
