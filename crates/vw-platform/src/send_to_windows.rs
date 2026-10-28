//! Windows "Send to Vibe Work" entries for the current user (no administrator rights): an Explorer verb for files
//! and for folders under `HKCU\Software\Classes` (Windows 11 lists it under "Show more options"), and a shortcut in
//! the user's Send to folder. Both run `vibe-work.exe --attach <paths>`. The installer writes and removes the same
//! keys and shortcut (`app/installer-hooks.nsh`), so either side can undo the other's.
//!
//! Depends on: windows-registry (the keys), the `windows` crate (the shortcut through `IShellLinkW`).
//! Called by: `send_to`.
//! Invariants: only the two verb keys named here and the one shortcut are written or removed; writing is
//! all-or-nothing (a failure removes what this call wrote).

use std::path::{Path, PathBuf};

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{HSTRING, Interface as _};
use windows_registry::CURRENT_USER;

use super::SendToError;

/// The verb keys (the installer uses the same names).
pub const FILE_VERB: &str = r"Software\Classes\*\shell\VibeWork.SendTo";
/// The folder verb key.
pub const FOLDER_VERB: &str = r"Software\Classes\Directory\shell\VibeWork.SendTo";
/// The installer's note that it asked the question (`app/installer-hooks.nsh`).
const ASKED: &str = r"Software\VibeWork.SendTo";
const LABEL: &str = "Send to Vibe Work";
const SHORTCUT: &str = "Vibe Work.lnk";

fn shortcut_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|appdata| appdata.join(r"Microsoft\Windows\SendTo").join(SHORTCUT))
}

/// Whether any of the entries is present.
#[must_use]
pub fn installed() -> bool {
    CURRENT_USER.open(FILE_VERB).is_ok()
        || CURRENT_USER.open(FOLDER_VERB).is_ok()
        || shortcut_path().is_some_and(|path| path.exists())
}

/// Whether the installer asked whether to add the entries (Yes or No).
#[must_use]
pub fn installer_asked() -> bool {
    CURRENT_USER
        .open(ASKED)
        .and_then(|key| key.get_string("Asked"))
        .is_ok_and(|value| value == "1")
}

/// Whether Explorer shows classic verbs under "Show more options" (Windows 11, build 22000 and later).
#[must_use]
pub fn under_more_options() -> bool {
    windows_registry::LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|key| key.get_string("CurrentBuildNumber"))
        .ok()
        .and_then(|build| build.trim().parse::<u32>().ok())
        .is_some_and(|build| build >= 22_000)
}

/// Writes both verbs and the shortcut for `exe`; all or nothing.
///
/// # Errors
/// [`SendToError::Write`] with the first failure; what this call wrote is removed again.
pub fn install(exe: &Path) -> Result<(), SendToError> {
    let result = write_verb(FILE_VERB, exe)
        .and_then(|()| write_verb(FOLDER_VERB, exe))
        .and_then(|()| write_shortcut(exe));
    if result.is_err() {
        let _ = remove();
    }
    result
}

/// Removes both verbs and the shortcut (absent ones are fine).
///
/// # Errors
/// [`SendToError::Write`] when one could not be removed (the others are still removed).
pub fn remove() -> Result<(), SendToError> {
    let mut failure = None;
    for key in [FILE_VERB, FOLDER_VERB] {
        if CURRENT_USER.open(key).is_ok()
            && let Err(error) = CURRENT_USER.remove_tree(key)
        {
            failure.get_or_insert(SendToError::Write(format!("{key}: {error}")));
        }
    }
    if let Some(path) = shortcut_path()
        && let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        failure.get_or_insert(SendToError::Write(format!("{}: {error}", path.display())));
    }
    failure.map_or(Ok(()), Err)
}

fn write_verb(key: &str, exe: &Path) -> Result<(), SendToError> {
    let fail = |error: windows::core::Error| SendToError::Write(format!("{key}: {error}"));
    let program = exe.to_string_lossy();
    let verb = CURRENT_USER.create(key).map_err(fail)?;
    verb.set_string("", LABEL).map_err(fail)?;
    verb.set_string("Icon", program.as_ref()).map_err(fail)?;
    verb.create("command")
        .and_then(|command| command.set_string("", format!("\"{program}\" --attach \"%1\"")))
        .map_err(fail)
}

/// Writes the Send to shortcut with the shell's own link object (what Explorer itself writes).
fn write_shortcut(exe: &Path) -> Result<(), SendToError> {
    let path = shortcut_path().ok_or_else(|| {
        SendToError::Write("the Send to folder is unknown (no APPDATA)".to_owned())
    })?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|error| SendToError::Write(format!("{}: {error}", folder.display())))?;
    }
    let fail = |error: windows::core::Error| {
        SendToError::Write(format!("{}: {}", path.display(), error.message()))
    };
    // SAFETY: COM is initialised for this thread for the duration of the calls and balanced below; every
    // interface pointer is owned by the `windows` wrappers, and the strings outlive the calls that read them.
    unsafe {
        let initialised = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let program = HSTRING::from(exe.as_os_str());
        let result = (|| {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&program)?;
            link.SetArguments(&HSTRING::from("--attach"))?;
            link.SetIconLocation(&program, 0)?;
            link.SetDescription(&HSTRING::from(LABEL))?;
            link.cast::<IPersistFile>()?
                .Save(&HSTRING::from(path.as_os_str()), true)
        })();
        if initialised {
            CoUninitialize();
        }
        result.map_err(fail)
    }
}
