//! Native-host values through the existing transactional registry abstraction. Setup/repair/remove use
//! the expected manifest path as ownership; foreign values, children and unreceipted keys that do not name this
//! installation's manifest are preserved.
use super::{Key, Registry};

fn ours(value: Option<&str>, manifest: &str) -> bool {
    value.is_none_or(|v| v.is_empty() || names(v, manifest))
}

fn names(value: &str, manifest: &str) -> bool {
    value.replace('/', "\\").eq_ignore_ascii_case(manifest)
}

/// The refusal for a key that names another setup's manifest, carries other values or children, or is being removed.
pub const FOREIGN_HOST: &str = "Browser helper registration belongs to another setup";

/// What [`host_replacing`] did with the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOutcome {
    /// The key already named the manifest.
    Unchanged,
    /// The key was created or repaired to name the manifest.
    Written,
    /// The key named the caller's `stale` value and now names the manifest.
    Replaced,
}

/// Reads a native-host key's default value without changing it; `None` when the key or its value is absent.
///
/// # Errors
/// Returns a registration error when the OS operation cannot complete.
pub fn host_value(path: &str, registry: &impl Registry) -> Result<Option<String>, String> {
    let Some(key) = registry.open(path)? else {
        return Ok(None);
    };
    // The handle's transaction is dropped uncommitted: nothing is written.
    key.host_value()
}

/// Registers or repairs the receipted native-host manifest path.
///
/// # Errors
/// Returns a registration error or refuses a key owned by another setup.
pub fn host(
    path: &str,
    manifest: &str,
    receipted: bool,
    registry: &impl Registry,
) -> Result<(), String> {
    host_replacing(path, manifest, receipted, None, registry).map(|_| ())
}

/// [`host`], and also takes over a key whose value is exactly `stale`: a value the caller found naming a manifest
/// file that no longer exists, or one whose helper program no longer exists. The value is compared again inside this
/// key's transaction, so a key another setup rewrote after the caller's check is still refused.
///
/// # Errors
/// Returns a registration error or refuses a key owned by another setup.
pub fn host_replacing(
    path: &str,
    manifest: &str,
    receipted: bool,
    stale: Option<&str>,
    registry: &impl Registry,
) -> Result<HostOutcome, String> {
    let (key, created) = registry.create(path)?;
    if key.marker_exists()? || !key.lock()? || !key.host_only()? {
        return Err(FOREIGN_HOST.into());
    }
    let value = key.host_value()?;
    let replace = stale.is_some_and(|stale| !stale.is_empty() && value.as_deref() == Some(stale));
    if !replace {
        // Without a receipt, an existing key is ours only when it already names this installation's manifest path
        // (left behind when the app's data was removed); an empty or foreign key stays refused.
        let unclaimed =
            !created && !receipted && !value.as_deref().is_some_and(|v| names(v, manifest));
        if unclaimed || !ours(value.as_deref(), manifest) {
            return Err(FOREIGN_HOST.into());
        }
        if key.host_matches(manifest)? {
            return Ok(HostOutcome::Unchanged);
        }
    }
    key.set_host(manifest)?;
    key.commit()?;
    Ok(if replace {
        HostOutcome::Replaced
    } else {
        HostOutcome::Written
    })
}

/// Removes only a native-host key still owned by the expected manifest.
///
/// # Errors
/// Returns a registration error or refuses a key owned by another setup.
pub fn remove_host(path: &str, manifest: &str, registry: &impl Registry) -> Result<(), String> {
    let Some(key) = registry.open(path)? else {
        return Ok(());
    };
    if key.marker_exists()?
        || !key.lock()?
        || !key.host_only()?
        || !ours(key.host_value()?.as_deref(), manifest)
    {
        return Err(FOREIGN_HOST.into());
    }
    key.delete()?;
    key.commit()
}
