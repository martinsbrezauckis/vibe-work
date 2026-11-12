//! Native-host values through the existing transactional registry abstraction. Setup/repair/remove use
//! the expected manifest path as ownership; foreign values, children and unreceipted keys are preserved.
use super::{Key, Registry};

fn ours(value: Option<&str>, manifest: &str) -> bool {
    value.is_none_or(|v| v.is_empty() || v.replace('/', "\\").eq_ignore_ascii_case(manifest))
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
    let (key, created) = registry.create(path)?;
    if (!created && !receipted) || key.marker_exists()? || !key.lock()? || !key.host_only()? {
        return Err("Browser helper registration belongs to another setup".into());
    }
    let value = key.host_value()?;
    if !ours(value.as_deref(), manifest) {
        return Err("Browser helper registration belongs to another setup".into());
    }
    if key.host_matches(manifest)? {
        return Ok(());
    }
    key.set_host(manifest)?;
    key.commit()
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
        return Err("Browser helper registration belongs to another setup".into());
    }
    key.delete()?;
    key.commit()
}
