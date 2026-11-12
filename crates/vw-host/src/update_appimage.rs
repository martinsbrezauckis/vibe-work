//! Before the Linux `AppImage` self-update: the user cache folder exists.
//!
//! The updater plugin (tauri-plugin-updater 2.13.2, `install_appimage`) backs up the running `AppImage` into a
//! temporary folder on the `AppImage`'s own disk, trying the system temp folder, then the user cache folder
//! (`$XDG_CACHE_HOME`, else `~/.cache`), then the `AppImage`'s folder. It stops at the first folder it cannot make its
//! temporary folder in, so a missing cache folder (a minimal desktop, a fresh home) failed the update whenever the
//! temp folder is on another disk, before the `AppImage`'s own folder was tried. The app makes the cache folder first.
//!
//! Depends on: std. Called by: `app` (`updater.rs`, Linux, before the install).

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

/// The user cache folder as the updater finds it (the XDG Base Directory rule): `XDG_CACHE_HOME` when it is an
/// absolute path, else `<home>/.cache`; `None` without either.
#[must_use]
pub fn cache_dir(xdg_cache_home: Option<&OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    xdg_cache_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home.join(".cache")))
}

/// Makes the user cache folder (`cache_dir`) when it is missing, readable only by the user (0700, as the XDG rule
/// asks for folders it creates); an existing folder is left as it is. Returns the folder.
///
/// # Errors
/// The folder could not be made.
pub fn ensure_cache_dir(
    xdg_cache_home: Option<&OsStr>,
    home: Option<&Path>,
) -> io::Result<Option<PathBuf>> {
    let Some(dir) = cache_dir(xdg_cache_home, home) else {
        return Ok(None);
    };
    if !dir.is_dir() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir)?;
    }
    Ok(Some(dir))
}
