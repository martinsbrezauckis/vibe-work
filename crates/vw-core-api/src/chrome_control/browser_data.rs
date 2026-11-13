//! Whether the profile-name search may read a Chrome or Edge user data folder (`profile_presence`, `profile_names`).
//! On macOS that folder is another app's protected data: the system may refuse the read without asking, and the
//! person allows it in System Settings (Privacy & Security, Files & Folders). So on macOS each folder is checked by
//! listing it, the one access the system decides on, and the answer is remembered: a refusal stops every later read of
//! that folder until the person may have changed the setting. The browser's Settings card then says how to allow it
//! (`refused`) and opens System Settings there (`open_settings`); when the person comes back to the window the card
//! asks for a `recheck`, which forgets the refusals and searches again, so the names appear without a restart.
//! The search starts for a paired identity, and also for an identity that asks to pair, only to name its profile in the
//! pairing dialog (`profile_presence::asking`): that read is local, the dialog waits for it at most two seconds,
//! and it fills the dialog text; only after the person approves does the folder it found name the paired profile
//! (`profiles::paired`), as the search for a paired identity would. The dialog's "needs your approval" line still
//! says nothing is allowed before the person answers. Setup and status keep their own no-read boundary on macOS (`probe::profiles_for`). Windows and
//! Linux read as before, without this check, and never show the card line.
use super::{ChromeControl, agent_browser::Browser, setup, setup::files};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::PoisonError,
};

/// System Settings at Privacy & Security, Files & Folders, where the person allows Vibe Work to read a browser's data.
pub(super) const FILES_AND_FOLDERS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders";
/// Privacy & Security itself, when the Files & Folders pane cannot be opened directly.
pub(super) const PRIVACY: &str = "x-apple.systempreferences:com.apple.preference.security";

/// Each checked user data folder: `true` readable, `false` refused by the system.
pub(super) type Checked = BTreeMap<PathBuf, bool>;

/// Whether `data` may be read now. Only an allowed listing or a permission refusal is remembered; a missing
/// folder is checked again later (the browser may create it).
pub(super) fn readable(chrome: &ChromeControl, data: &Path) -> bool {
    if !chrome.config.is_macos() {
        return true;
    }
    if let Some(known) = chrome
        .name_folders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(data)
    {
        return *known;
    }
    // Not under the lock: the system's permission check may take a while.
    let listed = files::access(&chrome.config, files::Access::ReadDir, data)
        .and_then(|()| std::fs::read_dir(data).map(drop));
    let known = match listed {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            tracing::info!(
                "macOS refused a browser profile folder; its profile names stay numbered until it is allowed"
            );
            false
        }
        Err(_) => return false,
    };
    chrome
        .name_folders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(data.to_owned(), known);
    known
}

/// macOS refused a user data folder of `browser` that the name search needed, and a profile of that browser is still
/// paired: its card shows how to allow it (`Some(true)`). Always `None` on Windows and Linux, which never check.
pub(super) fn refused(chrome: &ChromeControl, browser: Browser) -> Option<bool> {
    if !chrome.config.is_macos() {
        return None;
    }
    let refused: Vec<PathBuf> = chrome
        .name_folders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|(_, readable)| !**readable)
        .map(|(data, _)| data.clone())
        .collect();
    if refused.is_empty() {
        return None;
    }
    let paired = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .records
        .iter()
        .any(|(id, record)| browser.owns_profile(id) && !record.removed && !record.deleted);
    let name = if browser == Browser::Edge {
        "Microsoft Edge"
    } else {
        "Google Chrome"
    };
    (paired
        && setup::user_data(&chrome.config, chrome.dir(), name)
            .iter()
            .any(|data| refused.contains(data)))
    .then_some(true)
}

/// The person may have allowed a refused folder in System Settings: forget every refusal and search again at once,
/// so allowed names appear now. A folder that is still refused is listed once more and stays refused. Nothing happens
/// when nothing was refused. Blocking file work: callers run it off the async workers.
pub(super) fn recheck(shared: &crate::core::Shared) {
    let chrome = &shared.chrome_control;
    let forgotten = {
        let mut checked = chrome
            .name_folders
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = checked.len();
        checked.retain(|_, readable| *readable);
        checked.len() != before
    };
    if !forgotten {
        return;
    }
    // The unproven identities are searched now, not after the usual retry delay.
    chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .scanned = None;
    super::profile_presence::reconcile(shared);
    super::owner::publish(shared);
}

/// Open System Settings at Files & Folders (Privacy & Security when that pane does not open). macOS only.
pub(super) fn open_settings(config: &setup::ChromeControlConfig) -> Result<(), String> {
    if !config.is_macos() {
        return Err("System Settings is on macOS only.".to_owned());
    }
    let open = |url: &str| -> Result<(), String> {
        #[cfg(test)]
        if let Some(opener) = &config.settings_opener {
            return opener(url);
        }
        let status = vw_platform::process_command::hidden("/usr/bin/open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| error.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("System Settings did not open ({status})"))
        }
    };
    open(FILES_AND_FOLDERS).or_else(|_| open(PRIVACY))
}
