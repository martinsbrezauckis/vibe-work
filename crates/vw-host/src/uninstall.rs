//! Credential scope for the installer's explicit session/vault cleanup. Uses the vault account
//! scheme and platform credential deletion; called before the uninstaller removes the executable.
#[cfg(windows)]
use std::path::Path;

#[cfg(any(windows, test))]
fn owned(target: &str, remembered: &[String]) -> bool {
    use vw_vault::remember::{CHROME_OWNER_PREFIX, KEYRING_SERVICE, NETWORK_KEY_PREFIX};
    if remembered.iter().any(|account| account == target) {
        return true;
    }
    let Some(account) = target.strip_suffix(&format!(".{KEYRING_SERVICE}")) else {
        return false;
    };
    [NETWORK_KEY_PREFIX, CHROME_OWNER_PREFIX]
        .iter()
        .any(|prefix| {
            account
                .strip_prefix(&format!("{prefix}-"))
                .is_some_and(|hash| {
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
        })
}

/// Clears this app's credentials for the installer's explicit sensitive-data choice.
/// The selected shared vault is never consulted; only the two private profile locations qualify.
///
/// # Errors
/// Reports a credential-store or path error; callers must not claim complete removal on failure.
#[cfg(windows)]
pub fn clear_credentials(data: &Path) -> Result<(), String> {
    use vw_vault::remember::{KEYRING_SERVICE, account_for};
    let private = vw_vault::private_location::private_vault_dir(data).map_err(|e| e.to_string())?;
    let remembered =
        [private, data.join("vault")].map(|p| format!("{}.{KEYRING_SERVICE}", account_for(&p)));
    vw_platform::credentials::delete_matching(|target| owned(target, &remembered))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vw_vault::remember::{KEYRING_SERVICE, account_for};

    #[test]
    fn cleanup_selects_only_owned_credentials() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().join("app.vibework.desktop");
        let private = vw_vault::private_location::private_vault_dir(&root).unwrap();
        let remembered =
            [private, root.join("vault")].map(|p| format!("{}.{KEYRING_SERVICE}", account_for(&p)));
        for prefix in ["vibe-work-network-key-v1", "vibe-work-chrome-owner-v1"] {
            let target = format!("{prefix}-{}.shellx-vault", "a".repeat(64));
            assert!(owned(&target, &remembered), "{target}");
            assert!(!owned(
                &target.replace("shellx-vault", "other-app"),
                &remembered
            ));
            assert!(!owned(&format!("{target}-extra"), &remembered));
        }
        assert!(remembered.iter().all(|t| owned(t, &remembered)));
        let shared = format!(
            "{}.shellx-vault",
            account_for(&folder.path().join("ShellX/vault"))
        );
        assert!(
            !owned(&shared, &remembered),
            "the person's shared vault stays"
        );
        assert!(!owned(
            "remembered-device-v1-unknown.shellx-vault",
            &remembered
        ));
    }
}
