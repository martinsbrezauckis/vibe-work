//! External-extension registration with atomic creation and conditional, non-recursive removal.
//! Setup supplies the permitted HKCU key. Windows uses winreg handles and a short registry transaction;
//! tests substitute handles to interleave competing writes without requiring Windows.
#[cfg(any(windows, test))]
const UPDATE_URL: &str = "https://clients2.google.com/service/update2/crx";
const FAILED: &str = "Chrome registration failed";

/// Transactional per-user registration access; production uses [`Native`].
pub trait Registry {
    /// One open transaction and the exact key it owns.
    type Key: Key;
    /// The boolean is the OS creation disposition, never a prior absence query.
    /// Creates a key atomically and reports the OS creation disposition.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn create(&self, path: &str) -> Result<(Self::Key, bool), String>;
    /// Opens an existing key for a conditional transactional change.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn open(&self, path: &str) -> Result<Option<Self::Key>, String>;
}

/// An exact registration key held for conditional changes within one transaction.
pub trait Key {
    /// Reads the native-host default value.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn host_value(&self) -> Result<Option<String>, String>;
    /// Compares the raw default value with an exact manifest path.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn host_matches(&self, path: &str) -> Result<bool, String>;
    /// Sets the native-host default value.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn set_host(&self, path: &str) -> Result<(), String>;
    /// Checks that only the default host value and no child keys are present.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn host_only(&self) -> Result<bool, String>;
    /// Sets the extension update URL.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn set_url(&self) -> Result<(), String>;
    /// Checks for a pre-existing removal marker owned by another writer.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn marker_exists(&self) -> Result<bool, String>;
    /// Locks out competing writes for this transaction.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn lock(&self) -> Result<bool, String>;
    /// Checks that only the expected extension-registration values remain.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn owned(&self) -> Result<bool, String>;
    /// Must fail if a child appeared; never recurse. Commit must reject intervening writes.
    /// Deletes this exact key non-recursively within its transaction.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn delete(&self) -> Result<(), String>;
    /// Commits the transaction, refusing conflicting changes.
    ///
    /// # Errors
    /// Returns a registration error when the OS operation cannot complete.
    fn commit(self) -> Result<(), String>;
}

/// Creates a new extension key without adopting an existing key.
///
/// # Errors
/// Returns a registration error if creation, writing or commit fails.
pub fn create(path: &str, registry: &impl Registry) -> Result<bool, String> {
    let (key, created) = registry.create(path)?;
    if !created {
        return Ok(false);
    }
    key.set_url()?;
    key.commit()?;
    Ok(true)
}

/// Removes an owned extension key, preserving foreign values and children.
///
/// # Errors
/// Returns a registration error if the conditional transaction fails.
pub fn undo(path: Option<&str>, registry: &impl Registry) -> Result<bool, String> {
    let Some(path) = path else { return Ok(false) };
    let Some(key) = registry.open(path)? else {
        return Ok(false);
    };
    // Recheck through the same handle. A transaction covers the last read-to-delete interval too.
    // Foreign values or children leave the whole registration alone, including a pre-existing removal marker.
    if key.marker_exists()? || !key.lock()? || !key.owned()? || !key.owned()? {
        return Ok(false);
    }
    key.delete()?;
    key.commit()?;
    Ok(true)
}

/// Per-user OS registry implementation; unsupported platforms return the existing refusal.
pub struct Native;

#[cfg(not(windows))]
impl Registry for Native {
    type Key = Self;
    fn create(&self, _: &str) -> Result<(Self, bool), String> {
        Err(FAILED.into())
    }
    fn open(&self, _: &str) -> Result<Option<Self>, String> {
        Err(FAILED.into())
    }
}
#[cfg(not(windows))]
impl Key for Native {
    fn host_matches(&self, _: &str) -> Result<bool, String> {
        Err(FAILED.into())
    }
    fn host_value(&self) -> Result<Option<String>, String> {
        Err(FAILED.into())
    }
    fn set_host(&self, _: &str) -> Result<(), String> {
        Err(FAILED.into())
    }
    fn host_only(&self) -> Result<bool, String> {
        Err(FAILED.into())
    }
    fn set_url(&self) -> Result<(), String> {
        Err(FAILED.into())
    }
    fn marker_exists(&self) -> Result<bool, String> {
        Err(FAILED.into())
    }
    fn lock(&self) -> Result<bool, String> {
        Err(FAILED.into())
    }
    fn owned(&self) -> Result<bool, String> {
        Err(FAILED.into())
    }
    fn delete(&self) -> Result<(), String> {
        Err(FAILED.into())
    }
    fn commit(self) -> Result<(), String> {
        Err(FAILED.into())
    }
}

#[cfg(windows)]
mod windows {
    use super::{FAILED, Key, Native, Registry, UPDATE_URL};
    use winreg::{
        HKCU, RegKey,
        enums::{DELETE, KEY_READ, KEY_WRITE, REG_CREATED_NEW_KEY},
        transaction::Transaction,
        types::ToRegValue,
    };

    /// The transaction-only marker value of [`Registry::open`].
    const LOCK: &str = "vibe-work-removal-lock";

    /// An exact Windows registry key and its pending transaction.
    pub struct Handle {
        key: RegKey,
        path: String,
        transaction: Transaction,
    }
    fn path(path: &str) -> Result<&str, String> {
        path.strip_prefix("HKCU\\")
            .filter(|p| !p.is_empty() && !p.contains('\0'))
            .ok_or_else(|| FAILED.into())
    }
    fn error(_: std::io::Error) -> String {
        FAILED.into()
    }

    impl Registry for Native {
        type Key = Handle;
        fn create(&self, name: &str) -> Result<(Handle, bool), String> {
            let path = path(name)?.to_owned();
            let transaction = Transaction::new().map_err(error)?;
            let (key, disposition) = HKCU
                .create_subkey_transacted_with_flags(&path, &transaction, KEY_READ | KEY_WRITE)
                .map_err(error)?;
            Ok((
                Handle {
                    key,
                    path,
                    transaction,
                },
                disposition == REG_CREATED_NEW_KEY,
            ))
        }
        fn open(&self, name: &str) -> Result<Option<Handle>, String> {
            let path = path(name)?.to_owned();
            let transaction = Transaction::new().map_err(error)?;
            let key = match HKCU.open_subkey_transacted_with_flags(
                &path,
                &transaction,
                KEY_READ | KEY_WRITE | DELETE,
            ) {
                Ok(key) => key,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(error(e)),
            };
            Ok(Some(Handle {
                key,
                path,
                transaction,
            }))
        }
    }
    impl Key for Handle {
        fn host_matches(&self, path: &str) -> Result<bool, String> {
            Ok(self
                .key
                .get_raw_value("")
                .is_ok_and(|value| value == path.to_reg_value()))
        }
        fn host_value(&self) -> Result<Option<String>, String> {
            match self.key.get_value("") {
                Ok(value) => Ok(Some(value)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(error(e)),
            }
        }
        fn set_host(&self, path: &str) -> Result<(), String> {
            self.key.set_value("", &path).map_err(error)
        }
        fn host_only(&self) -> Result<bool, String> {
            let info = self.key.query_info().map_err(error)?;
            Ok(info.sub_keys == 0
                && (info.values == 0 || (info.values == 1 && self.key.get_raw_value("").is_ok())))
        }
        fn set_url(&self) -> Result<(), String> {
            self.key.set_value("update_url", &UPDATE_URL).map_err(error)
        }
        fn marker_exists(&self) -> Result<bool, String> {
            match self.key.get_raw_value(LOCK) {
                Ok(_) => Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(error(e)),
            }
        }
        fn lock(&self) -> Result<bool, String> {
            // Windows does not stop a plain (non-transacted) value write to a key this transaction has only read,
            // and the delete would then remove that foreign value. A write inside the transaction (a marker value
            // set and removed again) makes Windows refuse other writers to the key until the transaction ends, so
            // the ownership checks that follow still hold at the delete. The marker never reaches the registry.
            self.key.set_value(LOCK, &1_u32).map_err(error)?;
            self.key.delete_value(LOCK).map_err(error)?;
            Ok(true)
        }
        fn owned(&self) -> Result<bool, String> {
            let info = self.key.query_info().map_err(error)?;
            Ok(info.sub_keys == 0
                && (info.values == 0
                    || (info.values == 1
                        && self
                            .key
                            .get_raw_value("update_url")
                            .is_ok_and(|v| v == UPDATE_URL.to_reg_value()))))
        }
        fn delete(&self) -> Result<(), String> {
            // RegDeleteKeyTransacted is non-recursive: a new child makes it fail.
            // The transaction binds this name to the key whose handle was checked above.
            HKCU.delete_subkey_transacted(&self.path, &self.transaction)
                .map_err(error)
        }
        fn commit(self) -> Result<(), String> {
            self.transaction.commit().map_err(error)
        }
    }
}

#[cfg(test)]
#[path = "extension_registry_unit_tests.rs"]
mod tests;

#[path = "extension_host_registry.rs"]
mod host_registration;
pub use host_registration::{host, remove_host};
