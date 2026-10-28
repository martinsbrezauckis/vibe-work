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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "vw-appimage-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    // The XDG rule is a Linux rule (the AppImage update runs on Linux only); the fixture paths are POSIX, and on
    // Windows `/data/cache` has no drive, is not absolute and is ignored like a relative value.
    #[cfg(unix)]
    #[test]
    fn the_cache_folder_follows_the_xdg_rule() {
        let home = Path::new("/home/someone");
        assert_eq!(
            cache_dir(None, Some(home)),
            Some(PathBuf::from("/home/someone/.cache"))
        );
        assert_eq!(
            cache_dir(Some(OsStr::new("/data/cache")), Some(home)),
            Some(PathBuf::from("/data/cache"))
        );
        // A relative XDG_CACHE_HOME is ignored, as the spec and the updater's lookup do.
        assert_eq!(
            cache_dir(Some(OsStr::new("cache")), Some(home)),
            Some(PathBuf::from("/home/someone/.cache"))
        );
        assert_eq!(cache_dir(None, None), None);
    }

    /// The updater makes its temporary folder inside the cache folder: in a home without `.cache` that failed.
    #[test]
    fn a_home_without_a_cache_folder_gets_one_the_updater_can_use() {
        let home = scratch("home");
        let made = ensure_cache_dir(None, Some(&home)).unwrap().unwrap();
        assert_eq!(made, home.join(".cache"));
        std::fs::create_dir(made.join("tauri_current_app_test"))
            .expect("the updater's temporary folder");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bits = std::fs::metadata(&made).unwrap().permissions().mode() & 0o777;
            assert_eq!(bits, 0o700);
        }
        // An existing folder is kept as it is.
        let again = ensure_cache_dir(None, Some(&home)).unwrap().unwrap();
        assert!(again.join("tauri_current_app_test").is_dir());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_missing_xdg_cache_folder_is_made_with_its_parents() {
        let root = scratch("xdg");
        let xdg = root.join("deep/cache");
        let made = ensure_cache_dir(Some(xdg.as_os_str()), None)
            .unwrap()
            .unwrap();
        assert!(made.is_dir());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
