//! Real loopback handovers acknowledge delivery even when the owner's window cannot take focus.
//! Uses disposable data folders and the host's public instance API, without a GUI or agent.

#![allow(clippy::unwrap_used)]

use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};

use vw_host::hand_over::HandOver;
use vw_host::instance::{self, InstanceGuard, InstanceStart};

fn owner(data: &std::path::Path) -> (InstanceGuard, Receiver<HandOver>, Arc<AtomicUsize>) {
    let (sender, received) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let focused = calls.clone();
    let InstanceStart::First(guard) = instance::start(
        data,
        None,
        move || {
            focused.fetch_add(1, Ordering::SeqCst);
            false
        },
        move |hand_over| sender.send(hand_over).unwrap(),
    )
    .unwrap() else {
        panic!("the disposable folder must have no owner");
    };
    (guard, received, calls)
}

#[test]
fn preflight_acknowledges_files_once_when_focus_fails() {
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("selected file.txt");
    std::fs::write(&file, "selected").unwrap();
    let (_guard, received, calls) = owner(data.path());
    let hand_over = HandOver::from_args(
        [OsString::from("--attach"), file.clone().into_os_string()],
        data.path(),
        &b""[..],
    )
    .unwrap();

    let accepted = instance::focus_existing(data.path(), Some(&hand_over)).unwrap();
    assert_eq!(received.try_recv().unwrap().paths, vec![file]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(received.try_recv().is_err());
    assert_eq!(accepted, Some(true), "delivered files must end preflight");
}

#[test]
fn admission_acknowledges_text_when_focus_fails() {
    let data = tempfile::tempdir().unwrap();
    let (_guard, received, calls) = owner(data.path());
    let hand_over = HandOver::from_args(
        [OsString::from("--attach-text"), OsString::from("-")],
        data.path(),
        &b"https://example.invalid/shared"[..],
    )
    .unwrap();
    let outcome = instance::start(
        data.path(),
        Some(hand_over),
        || panic!("not the owner"),
        |_| {
            panic!("a second instance must not deliver locally");
        },
    )
    .unwrap();

    assert_eq!(
        received.try_recv().unwrap().text.unwrap().text,
        "https://example.invalid/shared"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(outcome, InstanceStart::Existing { focused: true }));
    assert!(received.try_recv().is_err());
}

#[test]
fn empty_and_plain_launches_still_require_focus() {
    let data = tempfile::tempdir().unwrap();
    let (_guard, received, calls) = owner(data.path());
    let empty = HandOver::default();
    for hand_over in [None, Some(&empty)] {
        assert_eq!(
            instance::focus_existing(data.path(), hand_over).unwrap(),
            Some(false)
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(received.try_recv().is_err());
}

#[test]
fn missing_file_refusal_is_delivered_without_focus() {
    let data = tempfile::tempdir().unwrap();
    let (_guard, received, calls) = owner(data.path());
    let hand_over = HandOver::from_args(
        [OsString::from("--attach"), OsString::from("missing.txt")],
        data.path(),
        &b""[..],
    )
    .unwrap();
    let accepted = instance::focus_existing(data.path(), Some(&hand_over)).unwrap();
    let delivered = received.try_recv().unwrap();
    assert_eq!(delivered.paths, Vec::<std::path::PathBuf>::new());
    assert_eq!(delivered.refused.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        accepted,
        Some(true),
        "the owner received the refusal for the composer"
    );
}

/// Restart: the relaunched process starts while the old one is still shutting down. It must wait for the old
/// owner's lock (never focus the closing window or refuse the folder as "already running").
#[test]
fn a_restart_waits_for_the_closing_owner() {
    let data = tempfile::tempdir().unwrap();
    let (guard, _received, calls) = owner(data.path());
    instance::mark_restart(data.path()).unwrap();
    assert_eq!(
        instance::focus_existing(data.path(), None).unwrap(),
        None,
        "a restart never asks the closing window to focus"
    );
    let closing = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        drop(guard);
    });
    let outcome = instance::start(data.path(), None, || true, |_| ()).unwrap();
    closing.join().unwrap();
    assert!(
        matches!(outcome, InstanceStart::First(_)),
        "the relaunched app owns the folder"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(outcome);
    // The mark is used once: the next launch into a running app is an ordinary second launch again.
    let (_guard, _received, calls) = owner(data.path());
    assert_eq!(
        instance::focus_existing(data.path(), None).unwrap(),
        Some(false)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// An old restart mark (a relaunch that never came) does not change a later ordinary second launch.
#[test]
fn a_stale_restart_mark_is_ignored() {
    let data = tempfile::tempdir().unwrap();
    let (_guard, _received, calls) = owner(data.path());
    std::fs::write(data.path().join(instance::RESTART_MARK), "1000").unwrap();
    assert_eq!(
        instance::focus_existing(data.path(), None).unwrap(),
        Some(false)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
