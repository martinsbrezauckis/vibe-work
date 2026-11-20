//! Setting the app's own vault aside when its password is forgotten, so the person can start a new one.
//!
//! Nobody can recover a vault password. What the app can do safely: move its own private vault folder out of the
//! way with one same-disk rename into a sibling folder of the same `Vibe Work Vault` container. The selection record
//! stays as it is (it names the private folder, now absent), so the next status is "selected vault missing": setup
//! offers creating a new private vault, where the person pastes their Mistral key again, and nothing is chosen for
//! them silently (an absent record would let a start pick a shared vault on its own). Nothing is deleted or copied; the old vault keeps every item, still sealed with its old
//! password, and opens again with that password if it is moved back.
//!
//! Refused before anything moves ([`SetAsideRefusal`]): an open vault; a shared ShellX or Vault-app vault (other
//! apps own it); a vault this computer still keeps an unlock key for (a new vault's kept key would take the same
//! credential-store entry, which may be the last way into the old vault).
//!
//! Depends on: `managed` (selection record and private folder), `vault` (session, chain guard). Called by: vw-keys.
//! Invariant: the vault folder is renamed, never removed or copied; no other file changes.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{Vault, VaultKind, VaultStatus};
use crate::error::{SetAsideRefusal, VaultError};

/// The kept folder's name prefix inside the private container.
pub const SET_ASIDE_PREFIX: &str = "vault-set-aside-";

impl Vault {
    /// Checks that [`Vault::set_aside`] would move the selected vault, and returns the folder it would move:
    /// [`VaultError::SetAsideRefused`] with the reason, or selection read errors.
    fn set_aside_source(&self) -> Result<PathBuf, VaultError> {
        let refuse = |why| Err(VaultError::SetAsideRefused(why));
        match self.status()? {
            VaultStatus::Unlocked { .. } => return refuse(SetAsideRefusal::VaultOpen),
            VaultStatus::Locked {
                remembered_device: true,
                ..
            } => return refuse(SetAsideRefusal::RememberedVault),
            VaultStatus::Locked {
                kind: VaultKind::VibeWork,
                remembered_device: false,
            } => {}
            VaultStatus::Locked { .. } => return refuse(SetAsideRefusal::SharedVault),
            _ => return Err(VaultError::NoVault),
        }
        let Some(managed) = &self.managed else {
            return refuse(SetAsideRefusal::SharedVault);
        };
        if managed.selected(&self.config.location)? != Some(VaultKind::VibeWork) {
            // A private vault that is present but not chosen (another vault is selected) is not this unlock's vault.
            return refuse(SetAsideRefusal::SharedVault);
        }
        Ok(managed.private_dir.clone())
    }

    /// Moves the selected, locked private vault folder aside (one rename), so the next status offers creating a new
    /// vault. Returns where the old vault is kept now.
    ///
    /// # Errors
    /// [`VaultError::SetAsideRefused`] (an open vault, a shared ShellX or Vault-app vault, or one this computer keeps
    /// an unlock key for), [`VaultError::NoVault`] without a locked selected vault, or the rename's I/O error (then
    /// nothing moved).
    pub fn set_aside(&self) -> Result<PathBuf, VaultError> {
        let source = self.set_aside_source()?;
        let container = source
            .parent()
            .ok_or_else(|| VaultError::Corrupt("the vault folder has no parent".to_owned()))?;
        let kept = free_name(container, &stamp(SystemTime::now()))?;
        // Neither folder may be, or sit under, a link or reparse point (the rename never follows one).
        crate::private_location::check_path(&source)?;
        crate::private_location::check_path(&kept)?;
        std::fs::rename(&source, &kept)
            .map_err(|e| VaultError::io("set the vault aside", &source, e))?;
        // The old vault's newest head is no longer this folder's; a new vault here starts its own chain.
        self.chain.forget();
        Ok(kept)
    }
}

/// `vault-set-aside-<stamp>`, with `-2`, `-3`, ... when a folder of that name already exists.
fn free_name(container: &Path, stamp: &str) -> Result<PathBuf, VaultError> {
    for n in 1..=99u32 {
        let name = if n == 1 {
            format!("{SET_ASIDE_PREFIX}{stamp}")
        } else {
            format!("{SET_ASIDE_PREFIX}{stamp}-{n}")
        };
        let candidate = container.join(name);
        match std::fs::symlink_metadata(&candidate) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Err(e) => return Err(VaultError::io("check the set-aside folder", &candidate, e)),
            Ok(_) => {}
        }
    }
    Err(VaultError::Corrupt(
        "too many set-aside vault folders with the same time".to_owned(),
    ))
}

/// `YYYYMMDD-HHMMSS` in UTC.
fn stamp(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stamp_is_the_utc_date_and_time() {
        assert_eq!(stamp(UNIX_EPOCH), "19700101-000000");
        // 2026-11-20 05:07:12 UTC
        assert_eq!(
            stamp(UNIX_EPOCH + Duration::from_secs(1_795_151_232)),
            "20261120-050712"
        );
        // 2024-02-29 23:59:59 UTC (leap day)
        assert_eq!(
            stamp(UNIX_EPOCH + Duration::from_secs(1_709_251_199)),
            "20240229-235959"
        );
    }
}
