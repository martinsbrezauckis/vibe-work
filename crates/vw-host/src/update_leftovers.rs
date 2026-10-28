//! Removes the installer copies a Windows update leaves in the temp folder.
//!
//! The updater plugin (tauri-plugin-updater 2.13.2, `make_temp_dir` / `write_to_temp`) writes the downloaded
//! installer to `<temp>/<app>-<version>-updater-<random>/<app>-<version>-installer.exe` (`.msi` for an MSI) and keeps
//! it on purpose: the installer runs after the app has exited. Each update therefore left about 21 MB behind. The
//! next start of the app (the version that installer installed, or a later one) removes those folders here.
//!
//! Invariants (fail safe: anything unexpected stays):
//! - only direct children of `temp` named exactly `<app>-<version>-updater-<suffix>`, with a version that parses and
//!   is not newer than the running one (a newer one may be about to install), and a non-empty alphanumeric suffix;
//! - only real folders: a link or junction is never followed or removed (`symlink_metadata`);
//! - the folder holds nothing but regular files named `<app>-<version>-installer.exe|.msi` for that same version;
//!   those files are deleted one by one, then the empty folder (never a recursive delete);
//! - errors (a file still in use by a running installer, permissions) leave the folder and are logged only.
//!
//! Depends on: semver, tracing. Called by: `app` (`updater.rs`, Windows, a while after start), the public tests.

use std::path::{Path, PathBuf};

use semver::Version;

/// Removes the leftover updater folders of versions up to `running` from `temp`; returns the removed folders.
/// A missing or unreadable `temp`, or a `running` that is not a version, removes nothing.
#[must_use]
pub fn remove_installer_copies(temp: &Path, app_name: &str, running: &str) -> Vec<PathBuf> {
    let Ok(running) = Version::parse(running) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(temp) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(version) = leftover_version(&name, app_name) else {
            continue;
        };
        if version > running {
            continue;
        }
        let dir = entry.path();
        match remove_one(&dir, app_name, &version) {
            Ok(true) => removed.push(dir),
            Ok(false) => {}
            Err(error) => {
                tracing::info!(folder = %dir.display(), %error, "update: installer copy not removed yet");
            }
        }
    }
    removed
}

/// The version in `<app>-<version>-updater-<suffix>`, when the name has exactly that shape.
fn leftover_version(name: &str, app_name: &str) -> Option<Version> {
    let rest = name.strip_prefix(app_name)?.strip_prefix('-')?;
    let (version, suffix) = rest.rsplit_once("-updater-")?;
    if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Version::parse(version).ok()
}

/// Deletes the installer file(s) of one folder, then the folder; `Ok(false)` when the folder is not exactly ours.
fn remove_one(dir: &Path, app_name: &str, version: &Version) -> std::io::Result<bool> {
    // Never through a link or junction (std reports both as symlinks), never a plain file.
    if !std::fs::symlink_metadata(dir)?.file_type().is_dir() {
        return Ok(false);
    }
    let allowed = [
        format!("{app_name}-{version}-installer.exe"),
        format!("{app_name}-{version}-installer.msi"),
    ];
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let named = entry
            .file_name()
            .to_str()
            .is_some_and(|n| allowed.iter().any(|a| a == n));
        if !named || !entry.file_type()?.is_file() {
            return Ok(false);
        }
        files.push(entry.path());
    }
    for file in files {
        std::fs::remove_file(file)?;
    }
    std::fs::remove_dir(dir)?;
    Ok(true)
}
