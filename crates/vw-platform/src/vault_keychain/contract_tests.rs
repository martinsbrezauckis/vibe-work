//! Regression guard for the macOS vault's native boundary; runs on every build host.
//! The vault must never request an existing item's access/owner change, which prompts on every start.

#[test]
fn vault_never_changes_an_existing_items_access() {
    for boundary in [
        include_str!("../vault_keychain.rs"),
        include_str!("native.rs"),
        include_str!("access.rs"),
        include_str!("ffi.rs"),
    ] {
        assert!(
            !boundary.contains("SecKeychainItemSetAccess("),
            "vault reads/writes must not request Keychain access changes"
        );
    }
}
