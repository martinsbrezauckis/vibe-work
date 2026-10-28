//! The switch and the one-time question against a stand-in for the system's menus.

use std::cell::{Cell, RefCell};

use vw_platform::send_to::{SendToError, SendToStatus};

use super::{Menus, SendToRequest, apply};

#[derive(Default)]
struct FakeMenus {
    on: Cell<bool>,
    fail: RefCell<Option<String>>,
    installer_asked: Cell<bool>,
    calls: RefCell<Vec<&'static str>>,
}

impl Menus for FakeMenus {
    fn status(&self) -> SendToStatus {
        SendToStatus {
            supported: true,
            on: self.on.get(),
            under_more_options: false,
            installer_asked: self.installer_asked.get(),
        }
    }
    fn install(&self) -> Result<(), SendToError> {
        self.calls.borrow_mut().push("install");
        if let Some(reason) = self.fail.borrow().clone() {
            return Err(SendToError::Write(reason));
        }
        self.on.set(true);
        Ok(())
    }
    fn remove(&self) -> Result<(), SendToError> {
        self.calls.borrow_mut().push("remove");
        self.on.set(false);
        Ok(())
    }
}

#[test]
fn reading_changes_nothing_and_offers_the_question_once() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("settings.json");
    let menus = FakeMenus::default();
    let view = apply(&settings, SendToRequest::Read, &menus).unwrap();
    assert!(!view.on);
    assert_eq!(*menus.calls.borrow(), Vec::<&str>::new());
    assert!(!settings.exists());
    assert!(view.offer);
    // "Not now" ends the question; the menus stay as they were.
    let view = apply(&settings, SendToRequest::NotNow, &menus).unwrap();
    assert!(!view.offer);
    assert!(!view.on);
    assert_eq!(*menus.calls.borrow(), Vec::<&str>::new());
    assert!(!apply(&settings, SendToRequest::Read, &menus).unwrap().offer);
}

#[test]
fn the_switch_writes_and_removes_and_answers_the_question() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("settings.json");
    let menus = FakeMenus::default();
    let view = apply(&settings, SendToRequest::Set { on: true }, &menus).unwrap();
    assert!(view.on);
    assert!(!view.offer);
    let view = apply(&settings, SendToRequest::Set { on: false }, &menus).unwrap();
    assert!(!view.on);
    // Turned off later: not asked again.
    assert!(!view.offer);
    assert_eq!(*menus.calls.borrow(), vec!["install", "remove"]);
}

#[test]
fn a_failed_write_says_why_and_that_nothing_changed() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("settings.json");
    let menus = FakeMenus::default();
    menus.fail.replace(Some("permission denied".to_owned()));
    let error = apply(&settings, SendToRequest::Set { on: true }, &menus).unwrap_err();
    assert_eq!(
        error.message,
        "Could not add the menu entry: permission denied. Nothing was changed."
    );
    // The question stays: nothing was answered.
    assert!(apply(&settings, SendToRequest::Read, &menus).unwrap().offer);
}

#[test]
fn a_question_the_installer_asked_is_not_asked_again() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("settings.json");
    let menus = FakeMenus::default();
    menus.installer_asked.set(true);
    assert!(!apply(&settings, SendToRequest::Read, &menus).unwrap().offer);
}

#[test]
fn requests_with_extra_fields_are_refused() {
    assert!(serde_json::from_str::<SendToRequest>(r#"{"op":"set","on":true}"#).is_ok());
    assert!(serde_json::from_str::<SendToRequest>(r#"{"op":"set","on":true,"exe":"/x"}"#).is_err());
}
