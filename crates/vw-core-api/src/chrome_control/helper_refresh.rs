//! Keeps the helper copy Chrome starts (`<state>/bin/chrome-control`, named by the native messaging manifest) the
//! same as the helper this app version ships, so an app update reaches Chrome without Remove and Set up again.
//!
//! Invariants:
//! - Nothing happens before Set up: only a copy the setup receipt lists, at exactly `<state>/bin/<helper>`, is
//!   replaced, and only with the packaged helper whose digest matches the release pin.
//! - A copy that already has the packaged bytes is left untouched (no write).
//! - Links are never followed: when the state folder, its `bin` folder or the copy itself is a symbolic link,
//!   junction or other reparse point, nothing is written.
//! - The new bytes go to a fresh file in the same folder, are read back and checked against the pin, flushed to
//!   disk, and only then renamed over the old copy. On Windows a running helper keeps its file open: the old copy is
//!   renamed aside first (`chrome-control.old-<random>.exe`) and deleted at a later refresh; if even that fails,
//!   the old copy stays and the next start tries again. A failure never stops the app.
//! - The manifest files the receipt lists are rewritten the same way when their text differs from what this version
//!   writes at Set up.
//!
//! Depends on: `setup` (receipt, packaged helper, pin, manifest text). Called by: `chrome_control::refresh_helper`
//! (app start) and the Connect command.
use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::setup::{self, ChromeControlConfig};

/// What a refresh did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Refreshed {
    /// Chrome control is not set up (no receipt, or the receipt does not list the helper copy).
    NotSetUp,
    /// The copy already had the packaged bytes.
    Current,
    /// The copy was replaced with the packaged helper.
    Replaced,
}

/// Replaces `old` with `new` in one step (a rename). Tests substitute it to act as a locked Windows file.
pub(super) type Replace = dyn FnMut(&Path, &Path) -> std::io::Result<()>;

fn rename(new: &Path, old: &Path) -> std::io::Result<()> {
    std::fs::rename(new, old)
}

/// Brings the helper copy (and the manifests) up to this version, see the module header.
///
/// # Errors
/// A plain reason when the packaged helper is missing or changed, a link is in the way, or a write failed;
/// the setup receipt stays in place so the next start retries, even if the old copy could not be restored.
pub(super) fn refresh(config: &ChromeControlConfig, state: &Path) -> Result<Refreshed, String> {
    refresh_with(config, state, &mut rename)
}

pub(super) fn refresh_with(
    config: &ChromeControlConfig,
    state: &Path,
    replace: &mut Replace,
) -> Result<Refreshed, String> {
    let Some(files) = setup::receipt_files(state) else {
        return Ok(Refreshed::NotSetUp);
    };
    let bin = state.join("bin");
    let helper = bin.join(setup::helper_name());
    if !files.iter().any(|file| file == &helper) {
        return Ok(Refreshed::NotSetUp);
    }
    refuse_links(&[state, &bin])?;
    sweep_aside(&bin);
    let (_, packaged) = setup::packaged_helper(config)?;
    let installed = match std::fs::symlink_metadata(&helper) {
        Ok(meta) if is_link(&meta) || !meta.is_file() => {
            return Err(format!("{} is not a plain file", helper.display()));
        }
        Ok(meta) => Some(meta),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", helper.display())),
    };
    let current = installed.is_some() && std::fs::read(&helper).is_ok_and(|b| b == packaged);
    let outcome = if current {
        Refreshed::Current
    } else {
        let temp = write_checked(config, &bin, &packaged, installed.as_ref())?;
        if let Err(problem) = put_in_place(&temp, &helper, replace) {
            let _ = std::fs::remove_file(&temp);
            return Err(problem);
        }
        Refreshed::Replaced
    };
    setup::repair_registrations(config, state, None)?;
    Ok(outcome)
}

/// True when `meta` (from `symlink_metadata`) is a symbolic link or, on Windows, any reparse point.
fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

fn refuse_links(folders: &[&Path]) -> Result<(), String> {
    for folder in folders {
        let meta =
            std::fs::symlink_metadata(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        if is_link(&meta) || !meta.is_dir() {
            return Err(format!("{} is not a plain folder", folder.display()));
        }
    }
    Ok(())
}

fn random_name() -> Result<String, String> {
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|e| format!("no random name: {e}"))?;
    Ok(random
        .iter()
        .fold(String::with_capacity(16), |mut name, b| {
            use std::fmt::Write as _;
            let _ = write!(name, "{b:02x}");
            name
        }))
}

/// Names of the files a refresh leaves behind in `bin`: new copies being written and old copies set aside.
const TEMP_PREFIX: &str = ".chrome-control.new-";
const ASIDE_PREFIX: &str = "chrome-control.old-";

fn refresh_file_name(name: &str) -> bool {
    let extension = if cfg!(windows) { ".exe" } else { "" };
    let random = name
        .strip_prefix(TEMP_PREFIX)
        .or_else(|| name.strip_prefix(ASIDE_PREFIX)?.strip_suffix(extension));
    random.is_some_and(|random| {
        random.len() == 16
            && random
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}

/// Deletes old copies set aside by an earlier refresh and new copies an interrupted one left (plain files only;
/// one still in use stays until a later refresh).
fn sweep_aside(bin: &Path) {
    for entry in std::fs::read_dir(bin).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !refresh_file_name(&name) {
            continue;
        }
        if entry
            .path()
            .symlink_metadata()
            .is_ok_and(|meta| meta.is_file() && !is_link(&meta))
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Writes `bytes` to a new file in `folder`, keeps the permissions of the copy it replaces (owner-only on Unix
/// as Set up makes it), reads it back, checks the pin and flushes it. Returns its path.
fn write_checked(
    config: &ChromeControlConfig,
    folder: &Path,
    bytes: &[u8],
    old: Option<&std::fs::Metadata>,
) -> Result<PathBuf, String> {
    let temp = folder.join(format!("{TEMP_PREFIX}{}", random_name()?));
    let result = (|| -> Result<(), String> {
        let fail = |e: std::io::Error| format!("{}: {e}", temp.display());
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(fail)?;
        file.write_all(bytes).map_err(fail)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = old.map_or(0o700, |meta| meta.permissions().mode() & 0o7777);
            file.set_permissions(std::fs::Permissions::from_mode(mode))
                .map_err(fail)?;
        }
        #[cfg(not(unix))]
        let _ = old;
        file.sync_all().map_err(fail)?;
        drop(file);
        let written = std::fs::read(&temp).map_err(fail)?;
        if written != bytes {
            return Err(format!("{} was not written whole", temp.display()));
        }
        setup::check_helper_pin(config, &written)
    })();
    match result {
        Ok(()) => Ok(temp),
        Err(problem) => {
            let _ = std::fs::remove_file(&temp);
            Err(problem)
        }
    }
}

/// Renames `temp` over `target`; when that fails (a running Windows helper holds its file), renames the old copy
/// aside first and tries again, putting the old copy back when the second rename fails too.
fn put_in_place(temp: &Path, target: &Path, replace: &mut Replace) -> Result<(), String> {
    let Err(first) = replace(temp, target) else {
        return Ok(());
    };
    if !target.exists() {
        return Err(format!("{}: {first}", target.display()));
    }
    let folder = target.parent().unwrap_or(Path::new("."));
    let extension = target
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    let aside = folder.join(format!("{ASIDE_PREFIX}{}{extension}", random_name()?));
    replace(target, &aside).map_err(|e| {
        format!(
            "{}: {first}; setting it aside failed too: {e}",
            target.display()
        )
    })?;
    if let Err(e) = replace(temp, target) {
        let restoration = match replace(&aside, target) {
            Ok(()) => "old helper restored".to_owned(),
            Err(restore) => match replace(&aside, target) {
                Ok(()) => format!("restore failed: {restore}; old helper restored on retry"),
                Err(retry) => format!("restore failed: {restore}; restore retry failed: {retry}"),
            },
        };
        let host = if target.exists() {
            "host path exists"
        } else {
            "host path is absent; the next start will retry"
        };
        return Err(format!("{}: {e}; {restoration}; {host}", target.display()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "helper_refresh_tests.rs"]
mod tests;
