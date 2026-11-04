//! External-extension registration with atomic creation and conditional, non-recursive removal.
//! Setup supplies the permitted HKCU key. Windows uses winreg handles and a short registry transaction;
//! tests substitute handles to interleave competing writes without requiring Windows.
#[cfg(any(windows, test))]
const UPDATE_URL: &str = "https://clients2.google.com/service/update2/crx";
const FAILED: &str = "Chrome registration failed";

pub(super) trait Registry {
    type Key: Key;
    /// The boolean is the OS creation disposition, never a prior absence query.
    fn create(&self, path: &str) -> Result<(Self::Key, bool), String>;
    fn open(&self, path: &str) -> Result<Option<Self::Key>, String>;
}

pub(super) trait Key {
    fn host_value(&self) -> Result<Option<String>, String>;
    fn host_matches(&self, path: &str) -> Result<bool, String>;
    fn set_host(&self, path: &str) -> Result<(), String>;
    fn host_only(&self) -> Result<bool, String>;
    fn set_url(&self) -> Result<(), String>;
    fn marker_exists(&self) -> Result<bool, String>;
    fn lock(&self) -> Result<bool, String>;
    fn owned(&self) -> Result<bool, String>;
    /// Must fail if a child appeared; never recurse. Commit must reject intervening writes.
    fn delete(&self) -> Result<(), String>;
    fn commit(self) -> Result<(), String>;
}

pub(super) fn create(path: &str, registry: &impl Registry) -> Result<bool, String> {
    let (key, created) = registry.create(path)?;
    if !created {
        return Ok(false);
    }
    key.set_url()?;
    key.commit()?;
    Ok(true)
}

pub(super) fn undo(path: Option<&str>, registry: &impl Registry) -> Result<bool, String> {
    let Some(path) = path else { return Ok(false) };
    let Some(key) = registry.open(path)? else {
        return Ok(false);
    };
    // Recheck through the same handle. A transaction covers the last read-to-delete interval too.
    // Foreign values or children leave the whole registration alone, including a pre-existing removal marker.
    if key.marker_exists()? || !key.lock()? || !key.owned()? || !key.owned()? {
        return Ok(false);
    }
    key.delete()?;
    key.commit()?;
    Ok(true)
}

pub(super) struct Native;

#[cfg(not(windows))]
impl Registry for Native {
    type Key = Self;
    fn create(&self, _: &str) -> Result<(Self, bool), String> {
        Err(FAILED.into())
    }
    fn open(&self, _: &str) -> Result<Option<Self>, String> {
        Err(FAILED.into())
    }
}
#[cfg(not(windows))]
impl Key for Native {
    fn host_matches(&self, _: &str) -> Result<bool, String> {
        Err(FAILED.into())
    }
    fn host_value(&self) -> Result<Option<String>, String> {
        Err(FAILED.into())
    }
    fn set_host(&self, _: &str) -> Result<(), String> {
        Err(FAILED.into())
    }
    fn host_only(&self) -> Result<bool, String> {
        Err(FAILED.into())
    }
    fn set_url(&self) -> Result<(), String> {
        Err(FAILED.into())
    }
    fn marker_exists(&self) -> Result<bool, String> {
        Err(FAILED.into())
    }
    fn lock(&self) -> Result<bool, String> {
        Err(FAILED.into())
    }
    fn owned(&self) -> Result<bool, String> {
        Err(FAILED.into())
    }
    fn delete(&self) -> Result<(), String> {
        Err(FAILED.into())
    }
    fn commit(self) -> Result<(), String> {
        Err(FAILED.into())
    }
}

#[cfg(windows)]
mod windows {
    use super::{FAILED, Key, Native, Registry, UPDATE_URL};
    use winreg::{
        HKCU, RegKey,
        enums::{DELETE, KEY_READ, KEY_WRITE, REG_CREATED_NEW_KEY},
        transaction::Transaction,
        types::ToRegValue,
    };

    /// The transaction-only marker value of [`Registry::open`].
    const LOCK: &str = "vibe-work-removal-lock";

    pub(crate) struct Handle {
        key: RegKey,
        path: String,
        transaction: Transaction,
    }
    fn path(path: &str) -> Result<&str, String> {
        path.strip_prefix("HKCU\\")
            .filter(|p| !p.is_empty() && !p.contains('\0'))
            .ok_or_else(|| FAILED.into())
    }
    fn error(_: std::io::Error) -> String {
        FAILED.into()
    }

    impl Registry for Native {
        type Key = Handle;
        fn create(&self, name: &str) -> Result<(Handle, bool), String> {
            let path = path(name)?.to_owned();
            let transaction = Transaction::new().map_err(error)?;
            let (key, disposition) = HKCU
                .create_subkey_transacted_with_flags(&path, &transaction, KEY_READ | KEY_WRITE)
                .map_err(error)?;
            Ok((
                Handle {
                    key,
                    path,
                    transaction,
                },
                disposition == REG_CREATED_NEW_KEY,
            ))
        }
        fn open(&self, name: &str) -> Result<Option<Handle>, String> {
            let path = path(name)?.to_owned();
            let transaction = Transaction::new().map_err(error)?;
            let key = match HKCU.open_subkey_transacted_with_flags(
                &path,
                &transaction,
                KEY_READ | KEY_WRITE | DELETE,
            ) {
                Ok(key) => key,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(error(e)),
            };
            Ok(Some(Handle {
                key,
                path,
                transaction,
            }))
        }
    }
    impl Key for Handle {
        fn host_matches(&self, path: &str) -> Result<bool, String> {
            Ok(self
                .key
                .get_raw_value("")
                .is_ok_and(|value| value == path.to_reg_value()))
        }
        fn host_value(&self) -> Result<Option<String>, String> {
            match self.key.get_value("") {
                Ok(value) => Ok(Some(value)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(error(e)),
            }
        }
        fn set_host(&self, path: &str) -> Result<(), String> {
            self.key.set_value("", &path).map_err(error)
        }
        fn host_only(&self) -> Result<bool, String> {
            let info = self.key.query_info().map_err(error)?;
            Ok(info.sub_keys == 0
                && (info.values == 0 || (info.values == 1 && self.key.get_raw_value("").is_ok())))
        }
        fn set_url(&self) -> Result<(), String> {
            self.key.set_value("update_url", &UPDATE_URL).map_err(error)
        }
        fn marker_exists(&self) -> Result<bool, String> {
            match self.key.get_raw_value(LOCK) {
                Ok(_) => Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(error(e)),
            }
        }
        fn lock(&self) -> Result<bool, String> {
            // Windows does not stop a plain (non-transacted) value write to a key this transaction has only read,
            // and the delete would then remove that foreign value. A write inside the transaction (a marker value
            // set and removed again) makes Windows refuse other writers to the key until the transaction ends, so
            // the ownership checks that follow still hold at the delete. The marker never reaches the registry.
            self.key.set_value(LOCK, &1_u32).map_err(error)?;
            self.key.delete_value(LOCK).map_err(error)?;
            Ok(true)
        }
        fn owned(&self) -> Result<bool, String> {
            let info = self.key.query_info().map_err(error)?;
            Ok(info.sub_keys == 0
                && (info.values == 0
                    || (info.values == 1
                        && self
                            .key
                            .get_raw_value("update_url")
                            .is_ok_and(|v| v == UPDATE_URL.to_reg_value()))))
        }
        fn delete(&self) -> Result<(), String> {
            // RegDeleteKeyTransacted is non-recursive: a new child makes it fail.
            // The transaction binds this name to the key whose handle was checked above.
            HKCU.delete_subkey_transacted(&self.path, &self.transaction)
                .map_err(error)
        }
        fn commit(self) -> Result<(), String> {
            self.transaction.commit().map_err(error)
        }
    }
}

#[cfg(test)]
mod tests {
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

    #[cfg(windows)]
    #[test]
    fn native_removal_preserves_a_preexisting_marker_value() {
        use winreg::HKCU;
        let root = super::super::registry_native_tests::TestRoot::new("removal-marker");
        let name = format!(r"{}\extension", root.key());
        assert!(create(&name, &Native).unwrap());
        let path = name.trim_start_matches("HKCU\\");
        let key = HKCU
            .open_subkey_with_flags(path, winreg::enums::KEY_ALL_ACCESS)
            .unwrap();
        key.set_value("vibe-work-removal-lock", &"foreign marker")
            .unwrap();
        assert!(!undo(Some(&name), &Native).unwrap(), "not ours");
        let key = HKCU.open_subkey(path).unwrap();
        assert_eq!(
            key.get_value::<String, _>("vibe-work-removal-lock")
                .unwrap(),
            "foreign marker"
        );
        assert_eq!(
            key.get_value::<String, _>("update_url").unwrap(),
            UPDATE_URL
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_registry_creation_and_conditional_removal() {
        use winreg::{
            HKCU,
            enums::{KEY_READ, KEY_WRITE},
        };
        // Deletes its key and the empty shared test parent when dropped, also after a failed assertion.
        let test_root = super::super::registry_native_tests::TestRoot::new("external-cleanup");
        let root = test_root.key().trim_start_matches("HKCU\\").to_owned();
        let name = format!(r"HKCU\{root}\extension");
        let path = format!(r"{root}\extension");
        assert!(create(&name, &Native).unwrap());
        assert!(!create(&name, &Native).unwrap());
        assert!(undo(Some(&name), &Native).unwrap());
        for foreign in ["value", "child", "empty"] {
            let (key, _) = HKCU.create_subkey(&path).unwrap();
            if foreign == "value" {
                key.set_value("update_url", &"foreign").unwrap();
            }
            if foreign == "child" {
                key.create_subkey("child").unwrap();
            }
            assert!(
                !create(&name, &Native).unwrap(),
                "never adopt even an empty existing key"
            );
            drop(key);
            HKCU.delete_subkey_all(&path).unwrap(); // Test fixture cleanup only.
        }
        // A writer after the final ownership read must either be refused by Windows' transaction isolation,
        // or survive because our transaction refuses the conflicting delete/commit.
        for child in [false, true] {
            assert!(create(&name, &Native).unwrap());
            let checked = Native.open(&name).unwrap().unwrap();
            assert!(!checked.marker_exists().unwrap());
            assert!(checked.lock().unwrap());
            assert!(checked.owned().unwrap());
            assert!(checked.owned().unwrap());
            let writer = HKCU
                .open_subkey_with_flags(&path, KEY_READ | KEY_WRITE)
                .unwrap();
            let wrote = if child {
                writer.create_subkey("child").map(|_| ())
            } else {
                writer.set_value("update_url", &"foreign")
            };
            if checked.delete().is_ok() {
                let _ = checked.commit();
            } else {
                drop(checked);
            }
            if wrote.is_ok() {
                if child {
                    assert!(writer.open_subkey("child").is_ok());
                } else {
                    assert_eq!(
                        writer.get_value::<String, _>("update_url").unwrap(),
                        "foreign"
                    );
                }
            }
            drop(writer);
            let _ = HKCU.delete_subkey_all(&path);
        }
        drop(test_root);
    }
}

#[path = "setup_registry.rs"]
mod host_registration;
pub(super) use host_registration::{host, remove_host};
