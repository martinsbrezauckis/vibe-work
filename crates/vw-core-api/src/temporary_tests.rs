//! Scratch-folder cleanup and record writes; memory-only test cores own their temporary directory fixtures.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::TemporarySessions;

/// Owns the scratch root of one memory-only unit-test core, independently of sibling test cores.
pub(super) fn scratch_fixture() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("vibe-work-temporary-folders-")
        .tempdir()
        .unwrap()
}

/// Independent memory-only cores must not sweep a peer's unfinished temporary chat.
#[test]
fn memory_only_fixtures_own_their_scratch_folders() {
    let first = TemporarySessions::load(None);
    let made = first.new_scratch().unwrap();
    let root = made.parent().unwrap().to_path_buf();
    std::fs::write(made.join("keep.txt"), "first fixture").unwrap();
    {
        let second = TemporarySessions::load(None);
        let other = second.new_scratch().unwrap();
        let other_root = other.parent().unwrap().to_path_buf();
        second.sweep_scratch();
        assert!(
            made.join("keep.txt").is_file(),
            "another memory-only core swept this fixture's chat folder: {made:?}"
        );
        assert!(!other.exists(), "the second fixture cleans its own folder");
        drop(second);
        assert!(!other_root.exists(), "only its fixture removes its root");
    }
    assert!(made.is_dir());
    drop(first);
    assert!(
        !root.exists(),
        "the last fixture owner removes its own root"
    );
}

/// Individual chat cleanup retains the shared app root; a missing root is recreated before use.
#[test]
fn chat_cleanup_keeps_the_app_root_and_creation_recreates_a_missing_root() {
    let data = scratch_fixture();
    let record = TemporarySessions::load(Some(data.path().join("sessions")));
    let first = record.new_scratch().unwrap();
    let second = record.new_scratch().unwrap();
    let root = first.parent().unwrap().to_path_buf();
    std::fs::write(second.join("keep.txt"), "other chat").unwrap();
    record.remove_scratch(&first);
    assert!(root.is_dir(), "chat cleanup never removes the shared root");
    assert!(second.join("keep.txt").is_file());
    record.remove_scratch(&second);
    assert!(root.is_dir(), "an empty current root remains available");
    std::fs::remove_dir(&root).unwrap();
    let recreated = record.new_scratch().unwrap();
    assert_eq!(recreated.parent(), Some(root.as_path()));
    assert!(vw_platform::is_user_only(&root).unwrap());
    assert!(vw_platform::is_user_only(&recreated).unwrap());
    record.remove_scratch(&recreated);
}

#[cfg(unix)]
fn fresh(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "vw-temporary-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Cleanup deletes only folders this code made (`chat-` + 16 hex digits) inside a real scratch root: another
/// entry in the root stays, a link in place of a scratch folder is removed without touching what it points to,
/// and a root replaced by a link to somewhere else is not cleaned at all.
#[cfg(unix)]
#[test]
fn cleanup_never_deletes_outside_its_own_scratch_folders() {
    let dir = fresh("sweep");
    let record = TemporarySessions::load(Some(dir.join("sessions")));
    let made = record.new_scratch().unwrap();
    std::fs::write(made.join("note.txt"), "x").unwrap();
    let root = made.parent().unwrap().to_path_buf();
    let outside = fresh("outside");
    std::fs::create_dir_all(outside.join("keep")).unwrap();
    std::fs::write(outside.join("keep/file.txt"), "keep").unwrap();
    std::fs::write(root.join("other.txt"), "not ours").unwrap();
    let link = root.join("chat-0123456789abcdef");
    std::os::unix::fs::symlink(outside.join("keep"), &link).unwrap();

    let removed = record.sweep_scratch();
    assert!(removed.contains(&made) && !made.exists(), "our folder goes");
    assert!(root.join("other.txt").exists(), "a foreign entry stays");
    assert!(
        std::fs::symlink_metadata(&link).is_err(),
        "the link itself goes"
    );
    assert!(outside.join("keep/file.txt").exists(), "its target stays");

    // The root itself replaced by a link to a folder elsewhere: nothing there is deleted.
    std::fs::remove_dir_all(&root).unwrap();
    let elsewhere = fresh("elsewhere");
    std::fs::create_dir_all(elsewhere.join("chat-fedcba9876543210")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &root).unwrap();
    assert_eq!(record.sweep_scratch(), Vec::<std::path::PathBuf>::new());
    record.remove_scratch(&root.join("chat-fedcba9876543210"));
    assert!(elsewhere.join("chat-fedcba9876543210").exists());

    for path in [&dir, &outside, &elsewhere] {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// The scratch folders live beside the session data, not in it (the agent's folder guard keeps the agent out of
/// the session data); folders an earlier build made inside it are swept with the same rules, and that old root
/// goes once empty. PREDICT (before the move): the new folder's parent is `<sessions>/temporary-folders`.
#[cfg(unix)]
#[test]
fn scratch_folders_live_beside_the_session_data_and_old_ones_inside_it_are_swept() {
    let data = fresh("beside");
    let sessions = data.join("sessions");
    let record = TemporarySessions::load(Some(sessions.clone()));
    let made = record.new_scratch().unwrap();
    assert_eq!(
        made.parent(),
        Some(data.join("temporary-folders").as_path())
    );
    assert!(!sessions.join("temporary-folders").exists());

    let old_root = sessions.join("temporary-folders");
    let old = old_root.join("chat-00112233445566aa");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("x.txt"), "left by a test build").unwrap();
    std::fs::write(sessions.join("keep.txt"), "kept").unwrap();
    let removed = record.sweep_scratch();
    assert!(
        removed.contains(&made) && removed.contains(&old),
        "{removed:?}"
    );
    assert!(!made.exists() && !old.exists());
    assert!(!old_root.exists(), "the empty old root goes too");
    assert!(
        sessions.join("keep.txt").exists(),
        "nothing else in the session data is touched"
    );

    // A foreign entry keeps the old root (never removed recursively); a link in place of the old root is not
    // followed.
    std::fs::create_dir_all(&old_root).unwrap();
    std::fs::write(old_root.join("other.txt"), "not ours").unwrap();
    record.sweep_scratch();
    assert!(old_root.join("other.txt").exists());
    std::fs::remove_dir_all(&old_root).unwrap();
    let elsewhere = fresh("old-elsewhere");
    std::fs::create_dir_all(elsewhere.join("chat-fedcba9876543210")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &old_root).unwrap();
    record.sweep_scratch();
    assert!(elsewhere.join("chat-fedcba9876543210").exists());
    for path in [&data, &elsewhere] {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// The record's writes wait for the disk (a synced replace): other async work on the same worker keeps running
/// meanwhile. One worker thread, a ticker task, 40 adds and removes: the ticker ran during them.
#[tokio::test(flavor = "current_thread")]
async fn writing_the_record_does_not_hold_the_async_worker() {
    let dir = std::env::temp_dir().join(format!(
        "vw-temporary-worker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let record = Arc::new(TemporarySessions::load(Some(dir.clone())));
    let ticks = Arc::new(AtomicUsize::new(0));
    let counter = ticks.clone();
    let ticker = tokio::spawn(async move {
        loop {
            counter.fetch_add(1, Ordering::Relaxed);
            tokio::task::yield_now().await;
        }
    });
    tokio::task::yield_now().await;
    let before = ticks.load(Ordering::Relaxed);
    for i in 0..20 {
        record.add_saved(&format!("s{i}")).await.unwrap();
    }
    for i in 0..20 {
        record.remove_saved(&format!("s{i}")).await;
    }
    let during = ticks.load(Ordering::Relaxed) - before;
    ticker.abort();
    assert_eq!(record.ids(), Vec::<String>::new());
    assert_eq!(
        std::fs::read_to_string(dir.join(super::FILE)).unwrap(),
        "[]",
        "the record on disk follows every write"
    );
    assert!(
        during >= 40,
        "the ticker ran only {during} times during 40 writes"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
