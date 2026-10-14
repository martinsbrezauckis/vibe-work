//! Where a preview's downloads go: the session folder's `downloads/` subfolder, under a cleaned-up file name,
//! never over an existing file and never anywhere else (brief: "never outside without a user choice").
//!
//! Depends on: serde, std. Called by: `service` (the engine's download hooks), tests.
//! Invariants:
//! - The engine never writes into the session folder: it writes a new file in the app's private staging folder
//!   ([`staging_file`]); [`deliver`] then moves it into `<session>/downloads/`. A `downloads` folder that is a
//!   symlink (or junction) is refused; the file is created with create-new semantics (`O_CREAT|O_EXCL`: an
//!   existing name, also a planted symlink, is never opened or followed), so nothing is ever overwritten or
//!   written through a link; files above the size cap are dropped.
//! - The destination is always `<downloads dir>/<one path component>`; the name is built from the suggested
//!   name's last component with separators, control characters, Windows-reserved characters and reserved device
//!   names removed, at most [`MAX_NAME`] bytes, never empty, never `.` or `..`. What an engine leaves of a path
//!   inside a name is removed too: Chromium (`WebView2`) hands over `../../x` as `_.._x` (observed on Windows 11).
//! - An existing file is never overwritten: ` (2)`, ` (3)` ... are added before the extension.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Longest file name in bytes (keeps the full path well under every OS limit).
pub const MAX_NAME: usize = 120;
/// Downloads kept in a preview's list.
pub const MAX_LISTED: usize = 50;

/// Windows device names that cannot be file names (with any extension).
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The state of one download.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DownloadState {
    /// Being written.
    Running,
    /// Written completely.
    Done,
    /// Failed or cancelled.
    Failed,
}

/// One download as listed in the tab and to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DownloadView {
    /// File name inside the downloads folder.
    pub file_name: String,
    /// Full destination path.
    pub path: String,
    /// The URL it came from (a loopback URL of the session).
    pub url: String,
    /// State.
    pub state: DownloadState,
}

/// Drops what an engine left of a path at the start of a suggested name. Chromium (so `WebView2`) replaces the
/// separators of `../../x` with `_` and trims the leading dots, handing over `_.._x`. A leading run of
/// `_`-separated parts that are empty or only dots is removed when one of them is only dots, so a name such as
/// `__init__.py` (no dot-only part) is kept as it is.
fn strip_mangled_dirs(name: &str) -> &str {
    let mut rest = name;
    let mut saw_dots = false;
    while let Some((part, after)) = rest.split_once('_') {
        if part.is_empty() {
            rest = after;
        } else if part.chars().all(|c| c == '.') {
            saw_dots = true;
            rest = after;
        } else {
            break;
        }
    }
    if saw_dots { rest } else { name }
}

/// A cleaned-up single-component file name for `suggested` (which comes from the page or the URL).
#[must_use]
pub fn clean_name(suggested: &str) -> String {
    let last = suggested
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or("");
    let mut name: String = last
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    name = String::from(strip_mangled_dirs(name.trim()).trim_end_matches(['.', ' ']));
    while name.starts_with('.') {
        name.remove(0);
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        name.insert(0, '_');
    }
    if name.len() > MAX_NAME {
        let mut cut = MAX_NAME;
        while !name.is_char_boundary(cut) {
            cut -= 1;
        }
        name.truncate(cut);
    }
    if name.is_empty() {
        "download".to_owned()
    } else {
        name
    }
}

/// Downloads bigger than this are dropped (default of `PreviewConfig::max_download_bytes`).
pub const DEFAULT_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// A new, unused file path for one download in `staging` (a private folder of the app; created user-only).
///
/// # Errors
/// A text reason when the folder cannot be prepared or no random name is available.
pub fn staging_file(staging: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(staging).map_err(|e| format!("staging folder: {e}"))?;
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).map_err(|e| format!("random name: {e}"))?;
    let mut name = String::with_capacity(29);
    for byte in bytes {
        let _ = std::fmt::Write::write_fmt(&mut name, format_args!("{byte:02x}"));
    }
    name.push_str(".part");
    Ok(staging.join(name))
}

/// Checks that `dir` (`<session>/downloads`) is a real folder (created when missing), not a symlink or junction,
/// and that it resolves to `<session>/downloads` itself.
fn real_downloads_dir(dir: &Path) -> Result<(), String> {
    let parent = dir.parent().ok_or("the downloads folder has no parent")?;
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err("the session's downloads folder is a link; nothing was saved".to_owned());
        }
        Ok(meta) if !meta.is_dir() => return Err("downloads is not a folder".to_owned()),
        Ok(_) => {}
        Err(_) => std::fs::create_dir(dir).map_err(|e| format!("downloads folder: {e}"))?,
    }
    let resolved = dir.canonicalize().map_err(|e| e.to_string())?;
    let expected = parent
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(dir.file_name().ok_or("bad downloads folder")?);
    if resolved == expected {
        Ok(())
    } else {
        Err("the session's downloads folder resolves elsewhere; nothing was saved".to_owned())
    }
}

/// Moves a finished download from `staged` into `dir` under `name` (cleaned again), never following or replacing
/// an existing entry: `name`, `name (2)`, ... are tried with create-new semantics. The staged file is removed in
/// every case. Returns the final path.
///
/// # Errors
/// A text reason (too large, a linked downloads folder, I/O); nothing is left in `dir` then.
pub fn deliver(staged: &Path, dir: &Path, name: &str, max_bytes: u64) -> Result<PathBuf, String> {
    let result = deliver_inner(staged, dir, name, max_bytes);
    let _ = std::fs::remove_file(staged);
    result
}

fn deliver_inner(staged: &Path, dir: &Path, name: &str, max_bytes: u64) -> Result<PathBuf, String> {
    let meta =
        std::fs::symlink_metadata(staged).map_err(|e| format!("the download is missing: {e}"))?;
    if !meta.is_file() {
        return Err("the download is not a file".to_owned());
    }
    if meta.len() > max_bytes {
        return Err(format!(
            "the download is larger than {max_bytes} bytes; it was not saved"
        ));
    }
    real_downloads_dir(dir)?;
    let name = clean_name(name);
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name.as_str(), ""),
    };
    for n in 1..10_000u32 {
        let candidate = if n == 1 {
            dir.join(&name)
        } else {
            dir.join(format!("{stem} ({n}){ext}"))
        };
        // create_new = O_CREAT|O_EXCL: fails on any existing entry, a dangling symlink included, never follows.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut out) => {
                let copied = std::fs::File::open(staged)
                    .and_then(|mut input| std::io::copy(&mut input, &mut out));
                return match copied {
                    Ok(_) => Ok(candidate),
                    Err(error) => {
                        drop(out);
                        let _ = std::fs::remove_file(&candidate);
                        Err(format!("saving the download failed: {error}"))
                    }
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("saving the download failed: {error}")),
        }
    }
    Err("no free file name in the downloads folder".to_owned())
}
