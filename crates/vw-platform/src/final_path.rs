//! The path an open file handle actually names now, so a reader can prove the file it opened is the one a path check
//! allowed: a folder on the checked path swapped for a link or junction between the check and the open leaves the
//! handle on another file, and its final path then differs from the checked canonical path.
//!
//! Linux reads `/proc/self/fd/<fd>`, macOS asks `fcntl(F_GETPATH)` (through rustix), Windows asks
//! `GetFinalPathNameByHandleW` with DOS volume names, the form `std::fs::canonicalize` returns (`\\?\C:\...`).
//! Each form matches the platform's `std::fs::canonicalize` of the same file.
//!
//! Depends on: rustix (macOS), windows (Win32 `Storage_FileSystem`). Called by: vw-core-api `edit_preview` (the
//! read of a previewed file).

use std::fs::File;
use std::io;
use std::path::PathBuf;

/// The final path of the file `file` is open on.
///
/// # Errors
/// The OS error when the path cannot be read (on Linux also when `/proc` is not mounted), or `Unsupported` on other
/// platforms. A caller that proves containment treats every error as a refusal.
pub fn final_path(file: &File) -> io::Result<PathBuf> {
    imp::final_path(file)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::os::fd::AsRawFd;

    pub(super) fn final_path(file: &super::File) -> super::io::Result<super::PathBuf> {
        // A deleted file reads as "<path> (deleted)": a path that names no file, so it never equals a checked one.
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::os::unix::ffi::OsStringExt;

    pub(super) fn final_path(file: &super::File) -> super::io::Result<super::PathBuf> {
        let path = rustix::fs::getpath(file)?;
        Ok(std::ffi::OsString::from_vec(path.into_bytes()).into())
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{GetFinalPathNameByHandleW, VOLUME_NAME_DOS};

    pub(super) fn final_path(file: &super::File) -> super::io::Result<super::PathBuf> {
        let handle = HANDLE(file.as_raw_handle());
        let mut buffer = vec![0u16; 512];
        loop {
            // SAFETY: `handle` is the open handle `file` owns for the whole call, and the function writes at most
            // `buffer.len()` UTF-16 units into `buffer`.
            let written =
                unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, VOLUME_NAME_DOS) };
            let written = usize::try_from(written).unwrap_or(usize::MAX);
            if written == 0 {
                return Err(super::io::Error::last_os_error());
            }
            if written < buffer.len() {
                buffer.truncate(written);
                return Ok(std::ffi::OsString::from_wide(&buffer).into());
            }
            // Too small: `written` is the size needed, with the terminating null.
            if written > 32_768 {
                return Err(super::io::Error::other("final path too long"));
            }
            buffer = vec![0u16; written];
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    pub(super) fn final_path(_file: &super::File) -> super::io::Result<super::PathBuf> {
        Err(super::io::ErrorKind::Unsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use super::final_path;

    #[test]
    fn an_open_file_names_its_canonical_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "a").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        assert_eq!(final_path(&file).unwrap(), path.canonicalize().unwrap());
    }
}
