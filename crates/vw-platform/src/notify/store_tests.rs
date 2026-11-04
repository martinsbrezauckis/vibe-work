//! Portable lifetime checks for the Windows toast store used by `notify::windows`.
//! Uses drop-observable handles and callbacks so no `WinRT` runtime or desktop is required.

use super::store::{MAX_TOASTS, ToastEvent, ToastStore};
use std::sync::Arc;

#[test]
fn resolving_one_ask_releases_only_its_toast_and_prevents_late_activation() {
    let mut store = ToastStore::new();
    let handle = Arc::new(());
    store.insert(1, ("answered", handle.clone()));
    store.insert(2, ("open", Arc::new(())));
    drop(store.remove_where(|(ask, _)| *ask == "answered"));
    assert_eq!(Arc::strong_count(&handle), 1);
    assert!(store.event(1, ToastEvent::Activated).is_none());
    assert!(store.event(2, ToastEvent::Activated).is_some());
}

#[test]
fn shown_toast_survives_the_callers_handle() {
    let mut store = ToastStore::new();
    let handle = Arc::new(());
    let weak = Arc::downgrade(&handle);
    assert!(store.insert(1, handle.clone()).is_none());
    drop(handle);
    assert!(weak.upgrade().is_some());
    drop(store.event(1, ToastEvent::Activated));
    assert!(weak.upgrade().is_none());
}

#[test]
fn newest_toasts_are_retained_and_oldest_is_released_at_the_bound() {
    assert_eq!(MAX_TOASTS, 32);
    let mut store = ToastStore::new();
    let handles: Vec<_> = (0..MAX_TOASTS + 3).map(|_| Arc::new(())).collect();
    for (id, handle) in handles.iter().enumerate() {
        drop(store.insert(id as u64, handle.clone()));
    }
    for (id, handle) in handles.iter().enumerate() {
        assert_eq!(Arc::strong_count(handle), if id < 3 { 1 } else { 2 });
    }
    assert!(store.event(0, ToastEvent::Activated).is_none());
    assert!(store.event(3, ToastEvent::Activated).is_some());
}

#[test]
fn removing_another_toast_does_not_change_the_oldest_eviction_order() {
    let mut store = ToastStore::new();
    for id in 0..32 {
        store.insert(id, id);
    }
    assert_eq!(store.event(15, ToastEvent::UserCanceled), Some(15));
    assert_eq!(store.insert(32, 32), None);
    assert_eq!(store.insert(33, 33), Some(0));
    assert_eq!(store.event(0, ToastEvent::Activated), None);
    assert_eq!(store.event(1, ToastEvent::Activated), Some(1));
    assert_eq!(store.event(33, ToastEvent::Activated), Some(33));
}

#[test]
fn timeout_releases_the_toast_and_late_activation_cannot_take_it() {
    let mut store = ToastStore::new();
    let handle = Arc::new(());
    store.insert(1, handle.clone());
    drop(store.event(1, ToastEvent::TimedOut));
    assert!(store.event(1, ToastEvent::TimedOut).is_none());
    assert_eq!(Arc::strong_count(&handle), 1);
    assert!(store.event(1, ToastEvent::Activated).is_none());
}

#[test]
fn terminal_events_remove_only_their_toast_and_release_its_handle() {
    for event in [
        ToastEvent::Activated,
        ToastEvent::UserCanceled,
        ToastEvent::Removed,
        ToastEvent::TimedOut,
    ] {
        let mut store = ToastStore::new();
        let handle = Arc::new(());
        store.insert(1, handle.clone());
        store.insert(2, Arc::new(()));
        drop(store.event(1, event));
        assert_eq!(Arc::strong_count(&handle), 1);
        assert!(store.event(1, event).is_none());
        assert!(store.event(1, ToastEvent::Activated).is_none());
        assert!(store.event(2, ToastEvent::Activated).is_some());
    }
}

#[test]
fn repeated_or_late_activation_cannot_take_a_callback_twice() {
    let mut store = ToastStore::new();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let called = calls.clone();
    let callback: Box<dyn FnOnce() + Send> = Box::new(move || {
        called.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
    store.insert(1, callback);
    if let Some(click) = store.event(1, ToastEvent::Activated) {
        click();
    }
    assert!(store.event(1, ToastEvent::Activated).is_none());
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
}
