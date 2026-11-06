//! Copies the original ordinary file's DACL to the owned temporary handle before content is written.
//! Uses Win32 security APIs; called only by the ordinary atomic writer. Private files use their own DACL policy.
#![allow(
    unsafe_code,
    reason = "Win32 handle security calls are confined to vw-platform"
)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;

use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree, WIN32_ERROR};
use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
    UNPROTECTED_DACL_SECURITY_INFORMATION,
};
use windows::Win32::Storage::FileSystem::{FILE_GENERIC_WRITE, WRITE_DAC};

pub(super) fn write_access(options: &mut OpenOptions) {
    options.access_mode(FILE_GENERIC_WRITE.0 | WRITE_DAC.0);
}

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: GetSecurityInfo allocated this descriptor with LocalAlloc; freed only here.
        let _ = unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
    }
}

fn status(code: WIN32_ERROR) -> io::Result<()> {
    if code.0 == 0 {
        return Ok(());
    }
    let code = i32::try_from(code.0).map_err(io::Error::other)?;
    Err(io::Error::from_raw_os_error(code))
}

pub(super) fn copy_dacl(original: &File, replacement: &File) -> io::Result<()> {
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: original is a live file handle; out-pointers are valid; the descriptor owns the returned ACL.
    status(unsafe {
        GetSecurityInfo(
            HANDLE(original.as_raw_handle()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&raw mut dacl),
            None,
            Some(&raw mut descriptor),
        )
    })?;
    let descriptor = Descriptor(descriptor);
    let (mut control, mut revision) = (0_u16, 0_u32);
    // SAFETY: the descriptor is valid until the guard drops; both out-pointers are valid.
    unsafe { GetSecurityDescriptorControl(descriptor.0, &raw mut control, &raw mut revision) }
        .map_err(io::Error::other)?;
    let protection = if control & SE_DACL_PROTECTED.0 == 0 {
        UNPROTECTED_DACL_SECURITY_INFORMATION
    } else {
        PROTECTED_DACL_SECURITY_INFORMATION
    };
    // SAFETY: replacement is our live handle opened with WRITE_DAC; the ACL remains alive in descriptor.
    // No owner, SACL, or privilege-requiring metadata is changed. A null DACL is intentionally preserved.
    status(unsafe {
        SetSecurityInfo(
            HANDLE(replacement.as_raw_handle()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | protection,
            None,
            None,
            Some(dacl.cast_const()),
            None,
        )
    })
}

#[allow(
    clippy::permissions_set_readonly_false,
    reason = "Windows attribute on an owned temporary handle; no Unix permission bits are changed"
)]
pub(super) fn allow_cleanup(file: &File) {
    if let Ok(meta) = file.metadata() {
        let mut permissions = meta.permissions();
        permissions.set_readonly(false);
        let _ = file.set_permissions(permissions);
    }
}
