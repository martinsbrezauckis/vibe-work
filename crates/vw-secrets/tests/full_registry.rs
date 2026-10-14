//! A full registry: stored secrets are never evicted, and past the cap a new value is refused.
//! Its own test binary, so filling the process-wide registry cannot disturb the crate's other tests.
//!
//! PREDICT (written before the first run): after the real key is registered and 4096 + 10 connector values are
//! offered, the connector values stop at `MAX_ENTRIES - PINNED_RESERVE`; the first key and a key registered after
//! the flood are both still scrubbed (kind `mistralKey`); one more connector value is refused (not found).
//! Falsifier (mutation run): with the old "evict the oldest entry" rule back in `register`, the first key is
//! evicted and this test fails.

use vw_secrets::{MAX_ENTRIES, PINNED_RESERVE, SecretKind, count, find, register, scrub};

#[test]
fn a_full_registry_keeps_the_real_key_and_refuses_new_values() {
    let key = "vwFullRegistryRealKey0123456789";
    register(SecretKind::MistralKey, key);
    for n in 0..(MAX_ENTRIES + 10) {
        register(
            SecretKind::ConnectorCredential,
            &format!("vwFillerConnectorValue-{n:06}"),
        );
    }
    assert_eq!(
        count(),
        MAX_ENTRIES - PINNED_RESERVE,
        "other kinds stop at the reserve"
    );
    let late = "vwLateRealKeyAfterTheFlood98765";
    register(SecretKind::MistralKey, late);
    for (name, value) in [("first key", key), ("late key", late)] {
        let text = format!("{{\"text\": \"key {value} here\"}}");
        let (scrubbed, kinds) =
            scrub(text.as_bytes()).unwrap_or_else(|| panic!("{name} not found"));
        assert!(
            !String::from_utf8_lossy(&scrubbed).contains(value),
            "{name} is still redacted"
        );
        assert_eq!(kinds, vec![SecretKind::MistralKey], "{name}");
    }
    let refused = "vwFillerConnectorValue-REFUSED!";
    register(SecretKind::ConnectorCredential, refused);
    assert!(
        find(refused.as_bytes()).is_none(),
        "refused, nothing evicted"
    );
}
