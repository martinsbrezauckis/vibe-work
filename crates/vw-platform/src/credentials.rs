//! Windows credential removal for the explicit uninstall cleanup. Enumerates generic entry names only;
//! vw-host supplies the exact app-owned scope. Credential blobs are never inspected or logged.
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CRED_TYPE_GENERIC, CredDeleteW, CredEnumerateW, CredFree,
};
use windows::core::{PCWSTR, Result};

/// Deletes generic credentials whose complete target name is selected by the caller.
/// All selected entries are attempted, returning the first error (missing entries already succeeded).
///
/// # Errors
/// Returns a Windows credential enumeration or deletion error.
pub fn delete_matching(select: impl Fn(&str) -> bool) -> Result<()> {
    let mut count = 0;
    let mut entries = std::ptr::null_mut();
    // SAFETY: initialized out parameters; Windows returns count valid credential pointers in one allocation.
    if let Err(error) =
        unsafe { CredEnumerateW(PCWSTR::null(), None, &raw mut count, &raw mut entries) }
    {
        return if error.code() == ERROR_NOT_FOUND.to_hresult() {
            Ok(())
        } else {
            Err(error)
        };
    }
    let mut failure = None;
    // SAFETY: the successful enumeration owns an array of count pointers, valid until CredFree below.
    for entry in unsafe { std::slice::from_raw_parts(entries, count as usize) } {
        // SAFETY: every entry and its NUL-terminated TargetName are part of that allocation.
        let credential = unsafe { &**entry };
        if credential.Type != CRED_TYPE_GENERIC {
            continue;
        }
        let target = unsafe { credential.TargetName.to_string() };
        match target {
            Ok(target) if select(&target) => {
                // SAFETY: the target string pointer is valid until the enumeration allocation is freed.
                if let Err(error) =
                    unsafe { CredDeleteW(credential.TargetName, CRED_TYPE_GENERIC, None) }
                    && error.code() != ERROR_NOT_FOUND.to_hresult()
                {
                    failure.get_or_insert(error);
                }
            }
            Err(error) => {
                failure.get_or_insert(error.into());
            }
            _ => {}
        }
    }
    // SAFETY: free the enumeration allocation exactly once, after its last access.
    unsafe {
        CredFree(entries.cast());
    }
    failure.map_or(Ok(()), Err)
}
