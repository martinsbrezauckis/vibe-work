//! Display-only spelling of secret-command paths, shared by consent and the use log.
//! Uses lexical Windows prefix rules on every host; never changes a path used to execute.

use std::borrow::Cow;

/// Everyday drive/UNC spelling. Device paths and literal embedded prefixes remain literal.
pub(crate) fn display(path: &str) -> Cow<'_, str> {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return Cow::Owned(format!(r"\\{unc}"));
    }
    if let Some(drive) = path.strip_prefix(r"\\?\")
        && drive
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && drive.as_bytes().get(1..3) == Some(b":\\")
    {
        return Cow::Borrowed(drive);
    }
    Cow::Borrowed(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_windows_paths_on_every_platform() {
        for (path, shown) in [
            (
                r"\\?\C:\Program Files\Git\usr\bin\bash.exe",
                r"C:\Program Files\Git\usr\bin\bash.exe",
            ),
            (r"\\?\UNC\server\share\run.py", r"\\server\share\run.py"),
            (r"C:\normal\file", r"C:\normal\file"),
            (r"\\server\share\file", r"\\server\share\file"),
            ("/work/file", "/work/file"),
            (r"--literal=\\?\C:\file", r"--literal=\\?\C:\file"),
            (r"\\.\pipe\name", r"\\.\pipe\name"),
        ] {
            assert_eq!(display(path), shown);
        }
    }

    #[cfg(windows)]
    #[test]
    fn canonical_windows_path_is_plain_only_for_display() {
        let pinned = std::env::current_exe().unwrap().canonicalize().unwrap();
        let original = pinned.to_string_lossy().into_owned();
        assert!(original.starts_with(r"\\?\"));
        let shown = display(&original);
        assert!(!shown.starts_with(r"\\?\"));
        assert_eq!(
            std::path::Path::new(shown.as_ref()).canonicalize().unwrap(),
            pinned
        );
        assert_eq!(pinned.to_string_lossy(), original);
    }
}
