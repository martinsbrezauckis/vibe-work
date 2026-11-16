//! The move from ShellX's service name to Vibe Work's own, against a recording in-memory store with faults.
//!
//! PREDICT (written before the move existed, when the store read and wrote one name only): every test that
//! starts from an entry under the old name fails, because the store never looks there (the remembered vault
//! starts locked, Chrome Control gets a new owner identity, the network key is lost); the name test fails
//! because the service is still `shellx-vault`. The shared-vault and plain round-trip tests pass either way.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::*;
use crate::remember::{account_for, chrome_owner_account, network_key_account};

const NEW: &str = KEYRING_SERVICE;
const OLD: &str = SHELLX_KEYRING_SERVICE;

/// Shared contents, so a second [`Renamed`] over the same data is "the next start".
type Entries = Arc<Mutex<BTreeMap<(String, String), String>>>;

#[derive(Default)]
struct Fake {
    entries: Entries,
    /// Operations that fail, as `"<op> <service>"` (e.g. `"remove shellx-vault"`).
    failing: HashSet<String>,
    /// The copy reads back a different value.
    corrupt_copy: bool,
    calls: Mutex<Vec<String>>,
}

impl Fake {
    fn over(entries: &Entries) -> Self {
        Self {
            entries: Arc::clone(entries),
            ..Self::default()
        }
    }
    fn failing(mut self, op: &str) -> Self {
        self.failing.insert(op.to_owned());
        self
    }
    fn step(&self, op: &str, service: &str) -> Result<(), VaultError> {
        let call = format!("{op} {service}");
        self.calls.lock().unwrap().push(call.clone());
        if self.failing.contains(&call) {
            return Err(VaultError::Remember(format!("refused {call}")));
        }
        Ok(())
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl NamedStore for Fake {
    fn read(&self, service: &str, account: &str) -> Result<Option<SecretString>, VaultError> {
        self.step("read", service)?;
        let value = self
            .entries
            .lock()
            .unwrap()
            .get(&(service.to_owned(), account.to_owned()))
            .cloned();
        Ok(value.map(|v| {
            if self.corrupt_copy && service == NEW {
                SecretString::new(format!("{v}-damaged"))
            } else {
                SecretString::new(v)
            }
        }))
    }
    fn write(&self, service: &str, account: &str, secret: &SecretString) -> Result<(), VaultError> {
        self.step("write", service)?;
        self.entries.lock().unwrap().insert(
            (service.to_owned(), account.to_owned()),
            secret.expose().to_owned(),
        );
        Ok(())
    }
    fn remove(&self, service: &str, account: &str) -> Result<(), VaultError> {
        self.step("remove", service)?;
        self.entries
            .lock()
            .unwrap()
            .remove(&(service.to_owned(), account.to_owned()));
        Ok(())
    }
}

fn entry(entries: &Entries, service: &str, account: &str) -> Option<String> {
    entries
        .lock()
        .unwrap()
        .get(&(service.to_owned(), account.to_owned()))
        .cloned()
}

fn put(entries: &Entries, service: &str, account: &str, value: &str) {
    entries
        .lock()
        .unwrap()
        .insert((service.to_owned(), account.to_owned()), value.to_owned());
}

/// One account of every kind the app keeps.
fn accounts() -> [String; 3] {
    let profile = Path::new("/home/me/.local/share/app.vibework.desktop/vault");
    [
        account_for(profile),
        chrome_owner_account(Path::new(
            "/home/me/.local/share/app.vibework.desktop/keys.json",
        )),
        network_key_account(profile),
    ]
}

#[test]
fn the_service_is_vibe_works_own_and_never_vibes() {
    assert_eq!(KEYRING_SERVICE, "app.vibework.desktop");
    assert_eq!(SHELLX_KEYRING_SERVICE, "shellx-vault");
    // Vibe keeps its items under `ai.mistral.vibe` (and `ai.mistral.vibe:<user>:part<n>` for long values) and
    // the older `vibe`; Vibe Work never reads, writes or deletes those.
    for vibe in ["ai.mistral.vibe", "vibe"] {
        assert_ne!(KEYRING_SERVICE, vibe);
        assert!(!KEYRING_SERVICE.starts_with(&format!("{vibe}:")));
    }
}

#[test]
fn every_kind_of_old_entry_moves_and_keeps_its_secret() {
    for account in accounts() {
        let entries = Entries::default();
        put(&entries, OLD, &account, "kept secret");
        let store = Renamed::new(Fake::over(&entries));
        assert_eq!(
            store.get(&account).unwrap().unwrap().expose(),
            "kept secret",
            "{account}"
        );
        assert_eq!(
            entry(&entries, NEW, &account).as_deref(),
            Some("kept secret")
        );
        assert_eq!(entry(&entries, OLD, &account), None, "{account}");
        // Write, read back, then delete: the old entry goes only after the copy was proven.
        assert_eq!(
            store.inner().calls(),
            [
                format!("read {NEW}"),
                format!("read {OLD}"),
                format!("write {NEW}"),
                format!("read {NEW}"),
                format!("remove {OLD}"),
            ]
        );
        // The next read stays on the new name.
        assert_eq!(
            store.get(&account).unwrap().unwrap().expose(),
            "kept secret"
        );
        assert_eq!(store.inner().calls().len(), 6);
    }
}

#[test]
fn a_stop_between_the_copy_and_the_deletion_loses_nothing_and_finishes_next_start() {
    let account = account_for(Path::new("/vault"));
    let entries = Entries::default();
    put(&entries, OLD, &account, "kept secret");
    // First start: the deletion of the old entry does not happen (the app stops, or the store refuses).
    let first = Renamed::new(Fake::over(&entries).failing(&format!("remove {OLD}")));
    assert_eq!(
        first.get(&account).unwrap().unwrap().expose(),
        "kept secret"
    );
    assert_eq!(
        entry(&entries, OLD, &account).as_deref(),
        Some("kept secret")
    );
    assert_eq!(
        entry(&entries, NEW, &account).as_deref(),
        Some("kept secret")
    );
    // Next start: the copy wins and the old entry is deleted.
    let next = Renamed::new(Fake::over(&entries));
    assert_eq!(next.get(&account).unwrap().unwrap().expose(), "kept secret");
    assert_eq!(entry(&entries, OLD, &account), None);
    assert_eq!(
        entry(&entries, NEW, &account).as_deref(),
        Some("kept secret")
    );
    assert_eq!(
        next.inner().calls(),
        [format!("read {NEW}"), format!("remove {OLD}")]
    );
}

#[test]
fn a_failed_copy_keeps_the_old_entry_and_still_unlocks() {
    let account = account_for(Path::new("/vault"));
    let entries = Entries::default();
    put(&entries, OLD, &account, "kept secret");
    let store = Renamed::new(Fake::over(&entries).failing(&format!("write {NEW}")));
    assert_eq!(
        store.get(&account).unwrap().unwrap().expose(),
        "kept secret"
    );
    assert_eq!(
        entry(&entries, OLD, &account).as_deref(),
        Some("kept secret")
    );
    assert_eq!(entry(&entries, NEW, &account), None);
    // The next start moves it.
    let next = Renamed::new(Fake::over(&entries));
    assert_eq!(next.get(&account).unwrap().unwrap().expose(), "kept secret");
    assert_eq!(entry(&entries, OLD, &account), None);
    assert_eq!(
        entry(&entries, NEW, &account).as_deref(),
        Some("kept secret")
    );
}

#[test]
fn a_copy_that_reads_back_wrong_is_removed_and_the_old_entry_stays() {
    let account = account_for(Path::new("/vault"));
    let entries = Entries::default();
    put(&entries, OLD, &account, "kept secret");
    let mut fake = Fake::over(&entries);
    fake.corrupt_copy = true;
    let store = Renamed::new(fake);
    assert_eq!(
        store.get(&account).unwrap().unwrap().expose(),
        "kept secret"
    );
    assert_eq!(
        entry(&entries, OLD, &account).as_deref(),
        Some("kept secret")
    );
    assert_eq!(
        entry(&entries, NEW, &account),
        None,
        "unproven copy removed"
    );
}

#[test]
fn a_failed_read_of_the_new_name_is_reported_and_the_old_entry_is_not_touched() {
    let account = account_for(Path::new("/vault"));
    let entries = Entries::default();
    put(&entries, OLD, &account, "kept secret");
    let store = Renamed::new(Fake::over(&entries).failing(&format!("read {NEW}")));
    assert!(store.get(&account).is_err());
    assert_eq!(
        entry(&entries, OLD, &account).as_deref(),
        Some("kept secret")
    );
    assert_eq!(store.inner().calls(), [format!("read {NEW}")]);
}

#[test]
fn an_absent_entry_is_absent_under_both_names() {
    let account = account_for(Path::new("/vault"));
    let store = Renamed::new(Fake::default());
    assert!(store.get(&account).unwrap().is_none());
    // Looked up under the old name once per run.
    assert!(store.get(&account).unwrap().is_none());
    assert_eq!(
        store.inner().calls(),
        [
            format!("read {NEW}"),
            format!("read {OLD}"),
            format!("read {NEW}")
        ]
    );
}

#[test]
fn a_new_secret_goes_to_the_new_name_and_an_older_old_entry_never_wins() {
    let account = account_for(Path::new("/vault"));
    let entries = Entries::default();
    put(&entries, OLD, &account, "older secret");
    let store = Renamed::new(Fake::over(&entries));
    store
        .set(&account, &SecretString::new("newer secret"))
        .unwrap();
    assert_eq!(
        entry(&entries, NEW, &account).as_deref(),
        Some("newer secret")
    );
    assert_eq!(entry(&entries, OLD, &account), None);
    // Even when the old entry could not be deleted, the next start reads the newer secret.
    put(&entries, OLD, &account, "older secret");
    let next = Renamed::new(Fake::over(&entries));
    assert_eq!(
        next.get(&account).unwrap().unwrap().expose(),
        "newer secret"
    );
}

#[test]
fn a_failed_write_keeps_the_old_entry() {
    let account = account_for(Path::new("/vault"));
    let entries = Entries::default();
    put(&entries, OLD, &account, "older secret");
    let store = Renamed::new(Fake::over(&entries).failing(&format!("write {NEW}")));
    assert!(store.set(&account, &SecretString::new("newer")).is_err());
    assert_eq!(
        entry(&entries, OLD, &account).as_deref(),
        Some("older secret")
    );
}

#[test]
fn turning_remembering_off_deletes_both_names_old_first() {
    let account = account_for(Path::new("/vault"));
    let entries = Entries::default();
    put(&entries, OLD, &account, "stale");
    put(&entries, NEW, &account, "current");
    let store = Renamed::new(Fake::over(&entries));
    store.delete(&account).unwrap();
    assert_eq!(entry(&entries, OLD, &account), None);
    assert_eq!(entry(&entries, NEW, &account), None);
    assert_eq!(
        store.inner().calls(),
        [format!("remove {OLD}"), format!("remove {NEW}")]
    );
    assert!(store.get(&account).unwrap().is_none());
    // A refused deletion of the old entry is reported before the new one is touched.
    put(&entries, OLD, &account, "stale");
    put(&entries, NEW, &account, "current");
    let refusing = Renamed::new(Fake::over(&entries).failing(&format!("remove {OLD}")));
    assert!(refusing.delete(&account).is_err());
    assert_eq!(entry(&entries, NEW, &account).as_deref(), Some("current"));
}

#[test]
fn shellx_shared_vault_entry_keeps_shellx_name_and_never_moves() {
    let shared = account_for(Path::new("/home/me/.config/shellx-vault"));
    let entries = Entries::default();
    put(&entries, OLD, &shared, "shellx secret");
    let store = Renamed::new(Fake::over(&entries)).sharing(shared.clone());
    assert_eq!(
        store.get(&shared).unwrap().unwrap().expose(),
        "shellx secret"
    );
    store.set(&shared, &SecretString::new("rotated")).unwrap();
    assert_eq!(entry(&entries, OLD, &shared).as_deref(), Some("rotated"));
    assert_eq!(entry(&entries, NEW, &shared), None);
    store.delete(&shared).unwrap();
    assert_eq!(entry(&entries, OLD, &shared), None);
    assert!(
        store.inner().calls().iter().all(|call| call.ends_with(OLD)),
        "{:?}",
        store.inner().calls()
    );
    // The same vault's network key and the app's owner identity are Vibe Work's own: they still move.
    let network = network_key_account(Path::new("/home/me/.config/shellx-vault"));
    put(&entries, OLD, &network, "network key");
    assert_eq!(
        store.get(&network).unwrap().unwrap().expose(),
        "network key"
    );
    assert_eq!(
        entry(&entries, NEW, &network).as_deref(),
        Some("network key")
    );
    assert_eq!(entry(&entries, OLD, &network), None);
}
