//! Regression tests for `extension_registry.rs`; uses the parent module and is called by Cargo tests.
use super::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Default)]
struct Contents {
    value: Option<String>,
    host_type: Option<&'static str>,
    other_value: bool,
    marker: Option<String>,
    child: bool,
    deleted: bool,
}
#[derive(Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent registry race injections"
)]
struct Fake {
    contents: Rc<RefCell<Contents>>,
    appears: bool,
    exists: bool,
    /// A foreign write between the first ownership query and the final query.
    change_value: bool,
    /// A child arrives at the deletion boundary.
    add_child: bool,
}
struct Handle {
    contents: Rc<RefCell<Contents>>,
    reads: Cell<usize>,
    change_value: bool,
    add_child: bool,
}
impl Fake {
    fn handle(&self) -> Handle {
        Handle {
            contents: self.contents.clone(),
            reads: Cell::new(0),
            change_value: self.change_value,
            add_child: self.add_child,
        }
    }
}
impl Registry for Fake {
    type Key = Handle;
    fn create(&self, _: &str) -> Result<(Handle, bool), String> {
        if self.appears {
            self.contents.borrow_mut().value = Some("foreign".into());
        }
        Ok((self.handle(), !self.appears && !self.exists))
    }
    fn open(&self, _: &str) -> Result<Option<Handle>, String> {
        Ok(Some(self.handle()))
    }
}
impl Key for Handle {
    fn host_matches(&self, path: &str) -> Result<bool, String> {
        let c = self.contents.borrow();
        Ok(c.value.as_deref() == Some(path) && c.host_type.is_none())
    }
    fn host_value(&self) -> Result<Option<String>, String> {
        Ok(self.contents.borrow().value.clone())
    }
    fn set_host(&self, path: &str) -> Result<(), String> {
        let mut c = self.contents.borrow_mut();
        c.value = Some(path.into());
        c.host_type = None;
        Ok(())
    }
    fn host_only(&self) -> Result<bool, String> {
        let c = self.contents.borrow();
        Ok(!c.other_value && !c.child && c.marker.is_none())
    }
    fn set_url(&self) -> Result<(), String> {
        self.contents.borrow_mut().value = Some(UPDATE_URL.into());
        Ok(())
    }
    fn marker_exists(&self) -> Result<bool, String> {
        Ok(self.contents.borrow().marker.is_some())
    }
    fn lock(&self) -> Result<bool, String> {
        // Model the transaction's marker write and removal before ownership is queried.
        self.contents.borrow_mut().marker = Some("1".into());
        self.contents.borrow_mut().marker = None;
        Ok(true)
    }
    fn owned(&self) -> Result<bool, String> {
        self.reads.set(self.reads.get() + 1);
        let mut c = self.contents.borrow_mut();
        let owned = !c.child
            && !c.other_value
            && c.marker.is_none()
            && c.value.as_deref().is_none_or(|v| v == UPDATE_URL);
        if self.change_value && self.reads.get() == 1 {
            c.value = Some("foreign".into());
        }
        Ok(owned)
    }
    fn delete(&self) -> Result<(), String> {
        let mut c = self.contents.borrow_mut();
        if self.add_child {
            c.child = true;
        }
        if c.child {
            return Err(FAILED.into());
        }
        c.deleted = true;
        c.value = None;
        Ok(())
    }
    fn commit(self) -> Result<(), String> {
        Ok(())
    }
}
#[test]
fn native_host_restores_a_missing_default_value_in_a_receipted_key() {
    let registry = Fake {
        exists: true,
        ..Default::default()
    };
    host("HKCU\\fixture", r"C:\state\host.json", true, &registry).unwrap();
    assert_eq!(
        registry.contents.borrow().value.as_deref(),
        Some(r"C:\state\host.json")
    );
}

#[test]
fn native_host_restores_the_string_type_for_an_owned_value() {
    let registry = Fake {
        exists: true,
        ..Default::default()
    };
    registry.contents.borrow_mut().value = Some(r"C:\state\host.json".into());
    registry.contents.borrow_mut().host_type = Some("REG_EXPAND_SZ");
    host("HKCU\\fixture", r"C:\state\host.json", true, &registry).unwrap();
    assert_eq!(registry.contents.borrow().host_type, None);
}

#[test]
fn native_host_repairs_deleted_keys_empty_values_and_path_spelling() {
    for (exists, value) in [
        (false, None),
        (true, Some("")),
        (true, Some("c:/state/host.json")),
    ] {
        let registry = Fake {
            exists,
            ..Default::default()
        };
        registry.contents.borrow_mut().value = value.map(str::to_owned);
        host("HKCU\\fixture", r"C:\state\host.json", true, &registry).unwrap();
        assert_eq!(
            registry.contents.borrow().value.as_deref(),
            Some(r"C:\state\host.json")
        );
    }
}

#[test]
fn native_host_preserves_foreign_values_children_markers_and_unreceipted_keys() {
    for kind in ["foreign", "child", "extra", "marker", "unreceipted"] {
        let registry = Fake {
            exists: true,
            ..Default::default()
        };
        {
            let mut c = registry.contents.borrow_mut();
            c.value = (kind == "foreign").then(|| "foreign".into());
            c.child = kind == "child";
            c.other_value = kind == "extra";
            c.marker = (kind == "marker").then(|| "keep".into());
        }
        assert!(
            host(
                "HKCU\\fixture",
                r"C:\state\host.json",
                kind != "unreceipted",
                &registry
            )
            .is_err(),
            "{kind}"
        );
        assert!(!registry.contents.borrow().deleted);
        assert_eq!(
            registry.contents.borrow().value.as_deref(),
            (kind == "foreign").then_some("foreign")
        );
    }
}

#[test]
fn a_key_appearing_during_creation_is_not_overwritten_or_claimed() {
    let registry = Fake {
        appears: true,
        ..Default::default()
    };
    assert!(!create("HKCU\\fixture", &registry).unwrap());
    assert_eq!(registry.contents.borrow().value.as_deref(), Some("foreign"));
    let registry = Fake::default();
    assert!(create("HKCU\\fixture", &registry).unwrap());
    assert_eq!(
        registry.contents.borrow().value.as_deref(),
        Some(UPDATE_URL)
    );
}
#[test]
fn removal_preserves_a_value_changed_after_the_query() {
    let registry = Fake {
        change_value: true,
        ..Default::default()
    };
    registry.contents.borrow_mut().value = Some(UPDATE_URL.into());
    assert!(!undo(Some("HKCU\\fixture"), &registry).unwrap());
    assert_eq!(registry.contents.borrow().value.as_deref(), Some("foreign"));
    assert!(!registry.contents.borrow().deleted);
}
#[test]
fn removal_preserves_a_child_added_after_the_empty_query() {
    let registry = Fake {
        add_child: true,
        ..Default::default()
    };
    assert!(undo(Some("HKCU\\fixture"), &registry).is_err());
    assert!(registry.contents.borrow().child);
    assert!(!registry.contents.borrow().deleted);
}
#[test]
fn removal_deletes_only_receipt_owned_keys_without_foreign_contents() {
    for (receipt, value, other_value, child, deleted) in [
        (true, Some(UPDATE_URL), false, false, true),
        (true, None, false, false, true),
        (false, Some(UPDATE_URL), false, false, false),
        (true, Some("foreign"), false, false, false),
        (true, Some(UPDATE_URL), true, false, false),
        (true, Some(UPDATE_URL), false, true, false),
    ] {
        let registry = Fake::default();
        *registry.contents.borrow_mut() = Contents {
            value: value.map(str::to_owned),
            host_type: None,
            other_value,
            marker: None,
            child,
            deleted: false,
        };
        assert_eq!(
            undo(receipt.then_some("HKCU\\fixture"), &registry).unwrap(),
            deleted
        );
        assert_eq!(registry.contents.borrow().deleted, deleted);
    }
}

#[test]
fn removal_preserves_a_preexisting_marker_value() {
    let registry = Fake::default();
    registry.contents.borrow_mut().value = Some(UPDATE_URL.into());
    registry.contents.borrow_mut().marker = Some("foreign marker".into());
    assert!(!undo(Some("HKCU\\fixture"), &registry).unwrap(), "not ours");
    let contents = registry.contents.borrow();
    assert!(!contents.deleted);
    assert_eq!(contents.value.as_deref(), Some(UPDATE_URL));
    assert_eq!(contents.marker.as_deref(), Some("foreign marker"));
}
