//! Opens a directory for path traversal only, without reading its contents. Browser native-host setup uses
//! this to create the exempt child of a protected macOS app-data directory. Links anywhere in the path fail.
use std::{fs::File, io, path::Path};

/// Retains a directory for relative filesystem operations without requesting directory read access.
///
/// # Errors
/// Returns the OS refusal for a missing directory, a symbolic link, or denied traversal.
pub fn open(path: &Path) -> io::Result<File> {
    use rustix::fs::{Mode, OFlags};
    #[cfg(target_os = "macos")]
    let descriptor = rustix::fs::open(
        path,
        OFlags::from_bits_retain(libc::O_SEARCH.cast_unsigned())
            | OFlags::NOFOLLOW_ANY
            | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    #[cfg(target_os = "linux")]
    let descriptor = rustix::fs::openat2(
        rustix::fs::CWD,
        path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS,
    )?;
    Ok(File::from(descriptor))
}
