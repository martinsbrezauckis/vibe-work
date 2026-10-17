//! Reviews only captured bytes; similar emits unified diffs with complete line counts.
//! Called by Proposal and records; binary sides never become lossy text diffs.

use similar::{ChangeTag, TextDiff};

use crate::{ContentKind, FileReview, Kind, Snapshot};

pub(crate) fn file(snapshot: &Snapshot) -> FileReview {
    let before = snapshot.base.as_deref().unwrap_or_default();
    let after = snapshot.content.as_deref().unwrap_or_default();
    let mut review = FileReview {
        kind: snapshot.kind,
        path: snapshot.path.clone(),
        content_kind: ContentKind::Binary,
        before_bytes: before.len(),
        after_bytes: after.len(),
        diff: None,
        added_lines: 0,
        removed_lines: 0,
    };
    if before.contains(&0) || after.contains(&0) {
        return review;
    }
    let (Ok(before), Ok(after)) = (std::str::from_utf8(before), std::str::from_utf8(after)) else {
        return review;
    };
    review.content_kind = ContentKind::Text;
    let diff = TextDiff::from_lines(before, after);
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => review.added_lines += 1,
            ChangeTag::Delete => review.removed_lines += 1,
            ChangeTag::Equal => {}
        }
    }
    let path = snapshot.path.to_string_lossy();
    let old = if snapshot.kind == Kind::Add {
        "/dev/null".into()
    } else {
        format!("a/{path}")
    };
    let new = if snapshot.kind == Kind::Remove {
        "/dev/null".into()
    } else {
        format!("b/{path}")
    };
    review.diff = Some(diff.unified_diff().header(&old, &new).to_string());
    review
}
