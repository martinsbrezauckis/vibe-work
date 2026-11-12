//! Bounded private profile stores. Missing files are fresh state; damaged or unreadable files fail closed.
//! Bindings and profile settings use this distinction before granting a new browser identity.
use serde::de::DeserializeOwned;
use std::path::Path;

pub(super) const UNREADABLE: &str = "Saved browser choices could not be read; no action ran. Restore the browser settings file before selecting a profile.";

pub(super) fn load<T: DeserializeOwned + Default>(
    dir: Option<&Path>,
    name: &str,
    limit: u64,
) -> Result<T, ()> {
    let Some(dir) = dir else {
        return Ok(T::default());
    };
    let path = dir.join(name);
    match std::fs::symlink_metadata(&path) {
        Err(reason) if reason.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Ok(_) => super::probe::read_regular(&path, limit)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or(()),
        Err(_) => Err(()),
    }
}
