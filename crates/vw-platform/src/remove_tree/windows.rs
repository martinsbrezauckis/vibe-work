//! Deletes owned file entries without changing shared read-only attributes.
//! Uses cap-std no-follow opens and Win32 handle disposition; called only by the owned-tree walker.
//! Older filesystems may clear read-only only while a handle proves the file has exactly one link.

#![allow(unsafe_code)]

use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, OpenOptions, OpenOptionsExt};
use windows::Win32::Foundation::{
    ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_DISPOSITION_FLAG_DELETE,
    FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_INFO_EX_FLAGS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_WRITE_ATTRIBUTES,
    FileDispositionInfo, FileDispositionInfoEx, GetFileInformationByHandle,
    SetFileInformationByHandle,
};

pub(super) fn remove_file(parent: &Dir, name: &Path) -> io::Result<()> {
    remove_with(parent, name, delete_readonly)
}

fn remove_with(
    parent: &Dir,
    name: &Path,
    disposition: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options
        .access_mode((DELETE | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES).0)
        // Deny concurrent writers/link changes during the legacy link-count check and chmod.
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .follow(FollowSymlinks::No);
    let file = parent.open_with(name, &options)?;
    let meta = file.metadata()?;
    if super::linked(&meta) || !meta.is_file() {
        return Err(super::boundary());
    }
    match disposition(&file) {
        Ok(()) => Ok(()),
        Err(error) if unsupported(&error) => delete_legacy(&file),
        Err(error) => Err(error),
    }
}

fn delete_readonly(file: &File) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_INFO_EX_FLAGS(
            FILE_DISPOSITION_FLAG_DELETE.0
                | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0
                | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0,
        ),
    };
    // SAFETY: the file owns this live handle; info is the initialized structure for this class,
    // and its exact size is passed. The call does not retain the buffer.
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfoEx,
            (&raw const info).cast(),
            u32::try_from(std::mem::size_of_val(&info)).map_err(io::Error::other)?,
        )
    }
    .map_err(|error| win32_error(&error))
}

fn win32_error(error: &windows::core::Error) -> io::Error {
    // These APIs wrap GetLastError in an HRESULT; io::Error needs the original Win32 code.
    io::Error::from_raw_os_error(error.code().0 & 0xffff)
}

fn unsupported(error: &io::Error) -> bool {
    error.raw_os_error().is_some_and(|code| {
        [
            ERROR_INVALID_FUNCTION,
            ERROR_INVALID_PARAMETER,
            ERROR_NOT_SUPPORTED,
        ]
        .iter()
        .any(|unsupported| i32::try_from(unsupported.0).ok() == Some(code))
    })
}

fn delete_legacy(file: &File) -> io::Result<()> {
    let meta = file.metadata()?;
    if meta.permissions().readonly() {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the file owns this live handle and info is a writable buffer of the required type.
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &raw mut info) }
            .map_err(|error| win32_error(&error))?;
        if info.nNumberOfLinks != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cannot delete a read-only file with multiple hard links: this filesystem does not support deletion without changing shared attributes",
            ));
        }
        let mut perms = meta.permissions();
        perms.set_readonly(false);
        file.set_permissions(perms)?;
    }
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the live handle has DELETE access; info and its size match the requested class.
    // The file closes immediately after this call, completing deletion of its entry.
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            (&raw const info).cast(),
            u32::try_from(std::mem::size_of_val(&info)).map_err(io::Error::other)?,
        )
    }
    .map_err(|error| win32_error(&error))
}

#[cfg(test)]
mod tests;
