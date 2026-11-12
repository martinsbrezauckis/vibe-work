//! Login-Keychain storage for vault unlock and Chrome ownership; called by vw-vault.
//! Access is assigned only at creation. A legacy value is copied with a singleton trusted-app ACL
//! and verified before the old item is removed. Security.framework still authorizes every secret read.

#[cfg(target_os = "macos")]
mod access;
#[cfg(target_os = "macos")]
mod ffi;
#[cfg(target_os = "macos")]
mod native;
mod policy;

#[cfg(target_os = "macos")]
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use zeroize::Zeroizing;

// Serializes find/delete/create across all OsDeviceStore instances in this process.
#[cfg(target_os = "macos")]
static OPERATIONS: Mutex<policy::Attempts> = Mutex::new(policy::Attempts::new());

/// Reads an app-bound secret, migrating a legacy item once without changing its access.
/// # Errors
/// A Keychain read or migration is refused; the caller retains its vault-password fallback.
#[cfg(target_os = "macos")]
pub fn get(
    service: &str,
    account: &str,
    label: &str,
) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    let mut attempts = OPERATIONS
        .lock()
        .map_err(|_| "Keychain operation interrupted")?;
    attempts.run(service, account, || {
        policy::get(&mut native::Store::open(service, account, label)?)
    })
}

/// Saves with initial access for new items; existing bound items change only their secret data.
/// # Errors
/// The Keychain refuses the operation. A refused legacy read never deletes the existing item.
#[cfg(target_os = "macos")]
pub fn set(service: &str, account: &str, label: &str, secret: &[u8]) -> Result<(), String> {
    let mut attempts = OPERATIONS
        .lock()
        .map_err(|_| "Keychain operation interrupted")?;
    attempts.run(service, account, || {
        policy::set(&mut native::Store::open(service, account, label)?, secret)
    })
}

/// Deletes a credential from the same User-domain keychain used for reads and creation.
/// # Errors
/// The Keychain refuses deletion.
#[cfg(target_os = "macos")]
pub fn delete(service: &str, account: &str) -> Result<(), String> {
    let mut attempts = OPERATIONS
        .lock()
        .map_err(|_| "Keychain operation interrupted")?;
    native::Store::open(service, account, "")?.delete()?;
    attempts.removed(service, account);
    Ok(())
}
