//! The OS store opened after start ([`super`]).
//!
//! PREDICT (written before the change): the app chose its store once at start; without a keyring then it kept the
//! keep-nothing fallback for the whole run, so a keyring set up later was never used (the browser connection kept
//! saying the keyring is missing until a restart). These tests fail without a store that tries again.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use super::LateDeviceStore;
use crate::SecretString;
use crate::remember::{CHROME_OWNER_PREFIX, DeviceSecretStore, MemoryDeviceStore};

fn store(
    ready: &Arc<AtomicBool>,
    opens: &Arc<AtomicUsize>,
    os: &Arc<MemoryDeviceStore>,
    retry: Duration,
) -> LateDeviceStore {
    let (ready, opens, os) = (Arc::clone(ready), Arc::clone(opens), Arc::clone(os));
    LateDeviceStore::with_retry(
        Box::new(move || {
            opens.fetch_add(1, Ordering::SeqCst);
            if ready.load(Ordering::SeqCst) {
                Ok(Arc::clone(&os) as Arc<dyn DeviceSecretStore>)
            } else {
                Err(crate::VaultError::Remember("no Secret Service".to_owned()))
            }
        }),
        retry,
    )
}

#[test]
fn a_keyring_set_up_later_is_used_without_a_restart() {
    let (ready, opens) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicUsize::new(0)),
    );
    let os = Arc::new(MemoryDeviceStore::default());
    let late = store(&ready, &opens, &os, Duration::ZERO);
    let account = format!("{CHROME_OWNER_PREFIX}fixture");
    // No keyring: the browser identity reads as the missing keyring, and nothing is kept.
    assert!(late.get(&account).is_err());
    assert!(late.set("device", &SecretString::new("kept")).is_err());
    // The person sets up the keyring while the app runs.
    ready.store(true, Ordering::SeqCst);
    assert_eq!(
        late.get(&account).unwrap().map(|s| s.expose().to_owned()),
        None
    );
    late.set(&account, &SecretString::new("owner")).unwrap();
    assert_eq!(
        os.get(&account)
            .unwrap()
            .map(|s| s.expose().to_owned())
            .as_deref(),
        Some("owner"),
        "written to the OS store"
    );
    let before = opens.load(Ordering::SeqCst);
    late.delete(&account).unwrap();
    assert_eq!(opens.load(Ordering::SeqCst), before, "open once, then kept");
}

#[test]
fn a_missing_keyring_is_not_asked_again_within_the_retry_time() {
    let (ready, opens) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicUsize::new(0)),
    );
    let os = Arc::new(MemoryDeviceStore::default());
    let late = store(&ready, &opens, &os, Duration::from_secs(3600));
    for _ in 0..5 {
        assert!(late.get("device").unwrap().is_none());
    }
    assert_eq!(
        opens.load(Ordering::SeqCst),
        0,
        "the start's own attempt counts"
    );
}
