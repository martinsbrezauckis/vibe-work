//! Git Changes retains each child's effect ownership. Uses a throwaway repository and real session-fact parsing; no server.
use serde_json::json;

use super::{Hub, Review, Tracker, build_view};
use crate::changes_facts::facts;
use crate::changes_view::{ChangesMode, ChangesSubagent};

#[test]
fn git_changes_labels_each_subagent_even_with_colliding_turn_ids() {
    check_child_authors(false);
}

#[cfg(unix)]
#[test]
fn git_changes_labels_subagent_files_opened_through_a_link() {
    check_child_authors(true);
}

fn check_child_authors(linked: bool) {
    let base = std::env::temp_dir().join(format!(
        "vw-subagent-authors-{}-{linked}-{}",
        std::process::id(),
        vw_changes::now_ms()
    ));
    std::fs::create_dir_all(base.join("repo")).unwrap();
    // The session names the folder as git does (macOS: the temp folder is under the /var link). A child's edits in
    // a folder opened through a link are a separate case (subagent paths there are not yet made relative).
    let root = std::fs::canonicalize(base.join("repo")).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    let written = base.join("linked");
    #[cfg(unix)]
    if linked {
        std::os::unix::fs::symlink(&root, &written).unwrap();
    }
    let written = if linked { &written } else { &root };
    let hub = Hub::new(Some(base.join("data")), None);
    let baseline = vw_changes::Baseline::take(
        vw_changes::discover(&root).unwrap().unwrap(),
        vw_changes::SessionDir::open(&base.join("data"), "parent").unwrap(),
    )
    .unwrap();
    let mut tracker = Tracker::unopened(&root);
    tracker.review = Some(Review::Git {
        baseline: Box::new(baseline),
        watcher: None,
    });
    let mut parent = facts(&json!({"session": {"cwd": written}, "turns": []}));
    for (id, name, file) in [
        ("child-1", "File helper", "one.txt"),
        ("child-2", "Second helper", "two.txt"),
    ] {
        std::fs::write(root.join(file), format!("{name}\n")).unwrap();
        let child = facts(
            &json!({"session": {"cwd": written}, "turns": [{"id": "t1", "startedAt": 1}], "history": [{"id": "edit", "type": "effect", "turnId": "t1", "detail": {"kind": "file_write"}, "state": {"status": "completed", "output": {"filePath": written.join(file), "fileExisted": false}}}]}),
        );
        parent.subagents.push((
            ChangesSubagent {
                session_id: id.into(),
                name: name.into(),
            },
            child,
        ));
    }
    let view = build_view(&hub, "parent", &mut tracker, &parent);
    assert_eq!(view.mode, ChangesMode::Git);
    assert_eq!(view.files.len(), 2);
    assert_eq!(view.turns.len(), 2, "Git review lost the child authors");
    for (turn, name, file) in [
        (&view.turns[0], "File helper", "one.txt"),
        (&view.turns[1], "Second helper", "two.txt"),
    ] {
        assert_eq!(turn.subagent.as_ref().unwrap().name, name);
        assert_eq!(turn.files[0].path, file);
        assert_eq!(turn.files[0].entry_ids, ["edit"]);
        assert!(turn.anchor_entry_id.is_none());
    }
    drop(tracker);
    drop(hub);
    std::fs::remove_dir_all(base).unwrap();
}
