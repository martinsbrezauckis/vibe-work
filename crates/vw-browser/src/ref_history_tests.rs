//! Bounded host ref authority, distinct from the latest receipt baseline. Uses session observations;
//! checks document replacement, unlisted refs, recent-first eviction and explicit revocation without a window.
#![allow(clippy::unwrap_used)]
use crate::{
    model::{MAX_ALL_REFS, PageModel, RefItem},
    observe,
    sessions::Sessions,
};

fn seen(start: usize, count: usize, document: &str) -> crate::sessions::Seen {
    let model = PageModel {
        document: document.into(),
        url: "https://example.test/form".into(),
        refs: (start..start + count)
            .map(|n| RefItem {
                id: format!("e{n:08x}"),
                name: format!("Field {n}"),
                ..RefItem::default()
            })
            .collect(),
        all_refs: vec!["effffffff".into()],
        ..PageModel::default()
    };
    let mut seen = observe::seen(
        &model,
        vw_preview::MountId {
            session_id: "s".into(),
            label: "mount".into(),
        },
    );
    seen.loads = Some(1);
    seen
}

#[test]
fn bounded_union_keeps_latest_baseline_and_evicts_oldest_observations() {
    let sessions = Sessions::default();
    for batch in 0..6 {
        sessions.set_seen("s", seen(batch * 200, 200, "document"));
    }
    let current = sessions.seen("s").unwrap();
    assert_eq!(current.items.len(), MAX_ALL_REFS);
    assert_eq!(current.refs.len(), MAX_ALL_REFS);
    assert_eq!(current.names.len(), MAX_ALL_REFS);
    assert_eq!(current.model.refs.len(), 200);
    assert!(!current.refs.contains("e00000000"));
    assert!(current.refs.contains("e000000c8"));
    assert!(
        !current.refs.contains("effffffff"),
        "a scan alone never authorizes an unseen ref"
    );
    sessions.forget_ref("s", "e000000c8");
    assert!(!sessions.seen("s").unwrap().refs.contains("e000000c8"));
}

#[test]
fn unknown_load_new_document_new_load_and_new_mount_drop_history() {
    for change in 0..4 {
        let sessions = Sessions::default();
        sessions.set_seen("s", seen(0, 1, "before"));
        let mut next = seen(1, 1, "before");
        match change {
            0 => next.loads = None,
            1 => next.model.document = "after".into(),
            2 => next.loads = Some(2),
            _ => next.mount.label = "replacement".into(),
        }
        sessions.set_seen("s", next);
        assert!(!sessions.seen("s").unwrap().refs.contains("e00000000"));
    }
}
