//! Decision tests with a recording Keychain; no host credential store or secret is touched.
//! Checks operation ordering, durable migration, refusal preservation and creation-time access.
use super::*;

struct Fake {
    binding: Option<Binding>,
    value: Vec<u8>,
    calls: Vec<&'static str>,
    fail: Option<&'static str>,
    prepared: bool,
    marked: bool,
}
impl Fake {
    fn new(binding: Option<Binding>) -> Self {
        Self {
            binding,
            value: b"old owner key".to_vec(),
            calls: vec![],
            fail: None,
            prepared: false,
            marked: false,
        }
    }
    fn call(&mut self, call: &'static str) -> Result<(), String> {
        self.calls.push(call);
        if self.fail == Some(call) {
            Err(format!("refused {call}"))
        } else {
            Ok(())
        }
    }
}
impl Keychain for Fake {
    fn inspect(&mut self) -> Result<Option<Binding>, String> {
        self.call("inspect")?;
        Ok(self.binding)
    }
    fn read(&mut self) -> Result<Zeroizing<Vec<u8>>, String> {
        self.call("read")?;
        Ok(Zeroizing::new(self.value.clone()))
    }
    fn prepare_create(&mut self) -> Result<(), String> {
        self.call("prepare")?;
        self.prepared = true;
        Ok(())
    }
    fn create_bound(&mut self, secret: &[u8]) -> Result<(), String> {
        self.call("create with access and marker")?;
        assert!(self.prepared);
        assert!(self.binding.is_none());
        self.binding = Some(Binding::Bound);
        self.marked = true;
        self.value = secret.to_vec();
        Ok(())
    }
    fn update(&mut self, secret: &[u8]) -> Result<(), String> {
        self.call("update data only")?;
        self.value = secret.to_vec();
        Ok(())
    }
    fn delete(&mut self) -> Result<(), String> {
        self.call("delete")?;
        self.binding = None;
        Ok(())
    }
}

#[test]
fn bound_reads_never_change_access_or_recreate() {
    let mut store = Fake::new(Some(Binding::Bound));
    for _ in 0..3 {
        assert_eq!(&*get(&mut store).unwrap().unwrap(), b"old owner key");
    }
    assert_eq!(
        store.calls,
        ["inspect", "read", "inspect", "read", "inspect", "read"]
    );
}

#[test]
fn unbound_migrates_once_and_preserves_value_across_reopen() {
    let mut store = Fake::new(Some(Binding::Legacy));
    assert_eq!(&*get(&mut store).unwrap().unwrap(), b"old owner key");
    assert!(store.marked);
    assert_eq!(
        store.calls,
        [
            "inspect",
            "read",
            "prepare",
            "delete",
            "create with access and marker"
        ]
    );
    // Only persistent item state survives a new adapter/process, not an in-memory migration flag.
    let mut reopened = Fake::new(store.binding);
    reopened.marked = store.marked;
    reopened.value = store.value;
    assert_eq!(&*get(&mut reopened).unwrap().unwrap(), b"old owner key");
    assert_eq!(reopened.calls, ["inspect", "read"]);
}

#[test]
fn refused_read_returns_fallback_error_without_deleting_or_retrying() {
    for binding in [Binding::Bound, Binding::Legacy] {
        let mut store = Fake::new(Some(binding));
        store.fail = Some("read");
        assert_eq!(get(&mut store).unwrap_err(), "refused read");
        assert_eq!(store.binding, Some(binding));
        assert_eq!(store.calls, ["inspect", "read"]);
        assert_eq!(store.value, b"old owner key");
    }
}

#[test]
fn new_item_is_created_with_initial_access_and_marker() {
    let mut store = Fake::new(None);
    set(&mut store, b"new device key").unwrap();
    assert_eq!(
        store.calls,
        ["inspect", "prepare", "create with access and marker"]
    );
    assert_eq!(store.binding, Some(Binding::Bound));
    assert!(store.marked);
    assert_eq!(store.value, b"new device key");
}

#[test]
fn bound_write_only_updates_data() {
    let mut store = Fake::new(Some(Binding::Bound));
    set(&mut store, b"rotated device key").unwrap();
    assert_eq!(store.calls, ["inspect", "update data only"]);
}

#[test]
fn legacy_write_authorizes_old_value_before_replacement() {
    let mut store = Fake::new(Some(Binding::Legacy));
    set(&mut store, b"rotated device key").unwrap();
    assert_eq!(
        store.calls,
        [
            "inspect",
            "read",
            "prepare",
            "delete",
            "create with access and marker"
        ]
    );
    assert_eq!(store.value, b"rotated device key");
    let mut refused = Fake::new(Some(Binding::Legacy));
    refused.fail = Some("read");
    assert!(set(&mut refused, b"replacement").is_err());
    assert_eq!(refused.calls, ["inspect", "read"]);
    assert_eq!(refused.value, b"old owner key");
}

#[test]
fn missing_read_does_not_create_a_key() {
    let mut store = Fake::new(None);
    assert!(get(&mut store).unwrap().is_none());
    assert_eq!(store.calls, ["inspect"]);
}

#[test]
fn preparation_or_delete_failure_keeps_legacy_item_and_never_retries() {
    for failure in ["prepare", "delete"] {
        let mut store = Fake::new(Some(Binding::Legacy));
        store.fail = Some(failure);
        assert_eq!(get(&mut store).unwrap_err(), format!("refused {failure}"));
        assert_eq!(store.binding, Some(Binding::Legacy));
        assert_eq!(store.calls.last(), Some(&failure));
    }
}

#[test]
fn creation_failure_is_reported_once_without_returning_a_remembered_secret() {
    let mut store = Fake::new(Some(Binding::Legacy));
    store.fail = Some("create with access and marker");
    assert!(get(&mut store).is_err());
    assert_eq!(store.binding, None);
    assert_eq!(
        store
            .calls
            .iter()
            .filter(|call| **call == "create with access and marker")
            .count(),
        1
    );
}

#[test]
fn migration_marker_never_bypasses_acl_or_repeats_migration() {
    assert_eq!(classify(true, false).unwrap(), Binding::Bound);
    assert_eq!(classify(true, true).unwrap(), Binding::Bound);
    assert_eq!(classify(false, false).unwrap(), Binding::Legacy);
    assert!(classify(false, true).is_err());
}

#[test]
fn repeated_callers_do_not_repeat_a_refused_prompt() {
    let mut attempts = Attempts::new();
    let mut calls = 0;
    for _ in 0..3 {
        let result: Result<(), String> = attempts.run("vault", "device", || {
            calls += 1;
            Err("read refused".to_owned())
        });
        assert_eq!(result.unwrap_err(), "read refused");
    }
    assert_eq!(calls, 1);
    assert!(
        attempts
            .run("vault", "different account", || Ok(()))
            .is_ok()
    );
    // Explicit mode switching removes the old credential and permits fresh creation.
    attempts.removed("vault", "device");
    assert!(attempts.run("vault", "device", || Ok(())).is_ok());
}
