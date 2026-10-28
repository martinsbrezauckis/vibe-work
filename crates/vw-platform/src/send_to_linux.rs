//! Linux "Send to Vibe Work" entries, written per user: an application entry (Open with > Send to Vibe Work), a
//! Nautilus script (Scripts > Send to Vibe Work) and a Dolphin service menu (Actions > Send to Vibe Work). Each
//! runs `<app> --attach <the selected paths>`.
//!
//! Depends on: std. Called by: `send_to` (the user's data folder) and its tests (a scratch folder).
//! Invariants: every file carries [`MARKER`]; removal deletes only files that carry it, so a file of the same name
//! the user wrote stays. Writing is all-or-nothing: a failure removes what this call wrote.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::SendToError;

/// The line every entry file carries; removal checks it.
pub const MARKER: &str = "Written by Vibe Work for its right-click menus (Settings > General); removed when they are turned off.";

/// The three files, relative to the data home (`$XDG_DATA_HOME`, else `~/.local/share`).
const DESKTOP: &str = "applications/vibe-work-send-to.desktop";
const NAUTILUS: &str = "nautilus/scripts/Send to Vibe Work";
const DOLPHIN: &str = "kio/servicemenus/vibe-work-send-to.desktop";

/// The user's data home, where the entries go.
pub(super) fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local/share"))
        })
}

/// The program the entries run: the `AppImage` file when running from one (the running binary lives in a
/// temporary mount that is gone after exit), else this binary.
pub(super) fn launcher() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .or_else(|| std::env::current_exe().ok())
}

/// The entry files under `home`.
#[must_use]
pub fn entry_paths(home: &Path) -> [PathBuf; 3] {
    [home.join(DESKTOP), home.join(NAUTILUS), home.join(DOLPHIN)]
}

/// Whether any entry Vibe Work wrote is present under `home`.
#[must_use]
pub fn installed_in(home: &Path) -> bool {
    entry_paths(home).iter().any(|path| ours(path))
}

/// Writes the three entries under `home` for `exe` (replacing earlier ones of ours); all or nothing.
///
/// # Errors
/// [`SendToError::Write`] with the first failure; nothing of this call is left behind, and a file of the same name
/// that Vibe Work did not write is not replaced.
pub fn install_in(home: &Path, exe: &Path) -> Result<(), SendToError> {
    let [desktop, nautilus, dolphin] = entry_paths(home);
    let files = [
        (desktop, desktop_entry(exe), false),
        (nautilus, nautilus_script(exe), true),
        // Dolphin runs service menus from the user's folder only when they are executable.
        (dolphin, service_menu(exe), true),
    ];
    let mut written = Vec::new();
    for (path, content, executable) in &files {
        if let Err(error) = write_entry(path, content, *executable) {
            for done in &written {
                let _ = std::fs::remove_file(done);
            }
            return Err(error);
        }
        written.push(path.clone());
    }
    Ok(())
}

/// Removes the entries Vibe Work wrote under `home`; other files stay.
///
/// # Errors
/// [`SendToError::Write`] when one of ours could not be deleted (the others are still removed).
pub fn remove_in(home: &Path) -> Result<(), SendToError> {
    let mut failure = None;
    for path in entry_paths(home) {
        if ours(&path)
            && let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            failure.get_or_insert(SendToError::Write(format!("{}: {error}", path.display())));
        }
    }
    failure.map_or(Ok(()), Err)
}

fn ours(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| text.contains(MARKER))
}

fn write_entry(path: &Path, content: &str, executable: bool) -> Result<(), SendToError> {
    let fail = |error: std::io::Error| SendToError::Write(format!("{}: {error}", path.display()));
    if path.exists() && !ours(path) {
        return Err(SendToError::Write(format!(
            "{} already exists and was not written by Vibe Work",
            path.display()
        )));
    }
    let folder = path
        .parent()
        .ok_or_else(|| fail(std::io::ErrorKind::NotFound.into()))?;
    std::fs::create_dir_all(folder).map_err(fail)?;
    let temporary = folder.join(format!(
        ".{}.vibe-work-tmp",
        path.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(if executable { 0o700 } else { 0o600 });
    }
    let result = options
        .open(&temporary)
        .and_then(|mut file| file.write_all(content.as_bytes()))
        .and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(fail)
}

/// `Exec` quoting of the desktop entry specification: the program in double quotes with `"`, `` ` ``, `$` and `\`
/// escaped, and `%` doubled.
fn exec_program(exe: &Path) -> String {
    let mut quoted = String::from("\"");
    for c in exe.to_string_lossy().chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n# {MARKER}\nType=Application\nName=Send to Vibe Work\n\
         Comment=Adds the files to the message box of Vibe Work\nExec={} --attach %F\nIcon=vibe-work\n\
         Terminal=false\nNoDisplay=true\nMimeType=application/octet-stream;inode/directory;text/plain;\n",
        exec_program(exe)
    )
}

fn service_menu(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n# {MARKER}\nType=Service\nMimeType=all/all;\nX-KDE-ServiceTypes=KonqPopupMenu/Plugin\n\
         Actions=sendToVibeWork\n\n[Desktop Action sendToVibeWork]\nName=Send to Vibe Work\nIcon=vibe-work\n\
         Exec={} --attach %F\n",
        exec_program(exe)
    )
}

/// A shell script: the program in single quotes (a `'` written as `'\''`); Nautilus runs it in the folder it shows
/// with the selected names as arguments.
fn nautilus_script(exe: &Path) -> String {
    let program = exe.to_string_lossy().replace('\'', r"'\''");
    format!("#!/bin/sh\n# {MARKER}\nexec '{program}' --attach \"$@\"\n")
}

#[cfg(test)]
#[path = "send_to_linux_tests.rs"]
mod tests;
