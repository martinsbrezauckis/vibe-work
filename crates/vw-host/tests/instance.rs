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
