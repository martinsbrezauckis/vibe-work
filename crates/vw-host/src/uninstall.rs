//! Credential scope for the installer's explicit session/vault cleanup. Uses the vault account
//! scheme and platform credential deletion; called before the uninstaller removes the executable.
//! Entries count under the app's service name and under ShellX's, where older versions kept them; the
//! remembered device of ShellX's shared vault is never selected.
#[cfg(windows)]
use std::path::Path;

/// The service names the app's entries can carry (Windows target `<account>.<service>`).
#[cfg(any(windows, test))]
const SERVICES: [&str; 2] = [
    vw_vault::remember::KEYRING_SERVICE,
    vw_vault::remember::SHELLX_KEYRING_SERVICE,
];

/// The Windows targets of the remembered device of each private profile, under every service name.
#[cfg(any(windows, test))]
fn remembered_targets(profiles: &[std::path::PathBuf]) -> Vec<String> {
    profiles
        .iter()
        .flat_map(|profile| {
            let account = vw_vault::remember::account_for(profile);
            SERVICES.map(|service| format!("{account}.{service}"))
        })
        .collect()
}

#[cfg(any(windows, test))]
fn owned(target: &str, remembered: &[String]) -> bool {
    use vw_vault::remember::{CHROME_OWNER_PREFIX, NETWORK_KEY_PREFIX};
    if remembered.iter().any(|account| account == target) {
        return true;
    }
    let Some(account) = SERVICES
        .iter()
        .find_map(|service| target.strip_suffix(&format!(".{service}")))
    else {
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
    let private = vw_vault::private_location::private_vault_dir(data).map_err(|e| e.to_string())?;
    let remembered = remembered_targets(&[private, data.join("vault")]);
    vw_platform::credentials::delete_matching(|target| owned(target, &remembered))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vw_vault::remember::account_for;

    #[test]
    fn cleanup_selects_only_owned_credentials() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().join("app.vibework.desktop");
        let private = vw_vault::private_location::private_vault_dir(&root).unwrap();
        let remembered = remembered_targets(&[private.clone(), root.join("vault")]);
        // Both private profiles, each under the app's name and under the name older versions used.
        assert_eq!(remembered.len(), 4);
        for service in ["app.vibework.desktop", "shellx-vault"] {
            for profile in [&private, &root.join("vault")] {
                let target = format!("{}.{service}", account_for(profile));
                assert!(owned(&target, &remembered), "{target}");
            }
            for prefix in ["vibe-work-network-key-v1", "vibe-work-chrome-owner-v1"] {
                let target = format!("{prefix}-{}.{service}", "a".repeat(64));
                assert!(owned(&target, &remembered), "{target}");
                assert!(!owned(&target.replace(service, "other-app"), &remembered));
                assert!(!owned(&format!("{target}-extra"), &remembered));
            }
        }
        for service in ["shellx-vault", "app.vibework.desktop"] {
            let shared = format!(
                "{}.{service}",
                account_for(&folder.path().join("ShellX/vault"))
            );
            assert!(
                !owned(&shared, &remembered),
                "the person's shared vault stays"
            );
            assert!(!owned(
                &format!("remembered-device-v1-unknown.{service}"),
                &remembered
            ));
        }
        // Vibe's own entries are never the app's.
        assert!(!owned(
            &format!(
                "vibe-work-network-key-v1-{}.ai.mistral.vibe",
                "a".repeat(64)
            ),
            &remembered
        ));
    }
}
