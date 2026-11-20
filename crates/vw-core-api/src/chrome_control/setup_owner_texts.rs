//! What the person reads when the owner key cannot be stored or read (keyring missing, locked or failing).

/// Whether the owner identity failed for lack of a usable system keyring.
pub(in crate::chrome_control) fn keyring_unavailable(error: &vw_keys::KeyError) -> bool {
    matches!(
        error,
        vw_keys::KeyError::Vault(vw_vault::VaultError::Remember(_))
    )
}

/// Why the owner identity could not be stored, for the person: an unavailable system keyring is named with the next
/// step (the identity must stay readable while the vault is locked, so the vault password cannot stand in for it).
/// A vocabulary template: the caller words `{yourBrowser}` ([`super::super::agent_browser::Browser::words`]).
pub(in crate::chrome_control) fn owner_store_failed(error: &vw_keys::KeyError) -> &'static str {
    if keyring_unavailable(error) {
        vw_vocabulary::texts::CHROME_OWNER_STORE_UNAVAILABLE
    } else {
        vw_vocabulary::texts::CHROME_OWNER_STORE_FAILED
    }
}

/// Why the owner identity could not be read: without a system keyring (the app's fallback store holds nothing) the
/// keyring is named with its next step, as when storing it; any other failure keeps the plain read text.
pub(in crate::chrome_control) fn owner_read_failed(error: &vw_keys::KeyError) -> &'static str {
    if keyring_unavailable(error) {
        owner_store_failed(error)
    } else {
        vw_vocabulary::texts::CHROME_OWNER_READ_FAILED
    }
}

/// [`owner_read_failed`] worded for `browser`.
pub(in crate::chrome_control) fn owner_read_text(
    browser: super::super::agent_browser::Browser,
    error: &vw_keys::KeyError,
) -> String {
    browser.words(owner_read_failed(error))
}

/// [`owner_store_failed`] worded for `browser`.
pub(in crate::chrome_control) fn owner_store_text(
    browser: super::super::agent_browser::Browser,
    error: &vw_keys::KeyError,
) -> String {
    browser.words(owner_store_failed(error))
}

/// PREDICT (written before the change): every failure to store the owner identity read "Could not store Your
/// Chrome's owner identity. Nothing was changed." with no cause and no way forward, also when the system keyring
/// (Linux: no default Secret Service collection) was the cause.
#[cfg(test)]
mod owner_store_text_tests {
    #[test]
    fn an_unavailable_keyring_is_named_with_the_next_step() {
        let keyring = vw_keys::KeyError::Vault(vw_vault::remember::store_failure(
            "Couldn't access platform storage: SS error: result not returned from SS API".into(),
        ));
        let text = super::super::owner_store_failed(&keyring);
        assert!(
            text.contains("system keyring") && text.contains("Secret Service"),
            "{text}"
        );
        assert!(text.contains("Set up or unlock the system keyring, then choose Connect again."));
        assert!(text.contains("Nothing was changed.") && !text.contains("SS error"));
        let other = vw_keys::KeyError::Vault(vw_vault::VaultError::Locked);
        assert_eq!(
            super::super::owner_store_failed(&other),
            vw_vocabulary::texts::CHROME_OWNER_STORE_FAILED
        );
    }
}
