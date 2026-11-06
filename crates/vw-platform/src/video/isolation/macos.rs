//! Seatbelt and resource limits for the signed media helper, installed before reading the request.
//! The supervisor separately bounds resident memory: macOS does not enforce `RLIMIT_RSS`.
use std::{ffi::CString, path::Path};

#[link(name = "sandbox")]
unsafe extern "C" {
    fn sandbox_init(
        profile: *const libc::c_char,
        flags: u64,
        error: *mut *mut libc::c_char,
    ) -> libc::c_int;
    fn sandbox_free_error(error: *mut libc::c_char);
}

/// Installs a default-deny filesystem/network/process policy for exactly one immutable snapshot.
/// # Errors
/// Invalid snapshot path, failed hard limit or unavailable Seatbelt. Never decode on failure.
pub fn restrict(input: &Path) -> Result<(), String> {
    use rustix::process::{Resource, Rlimit, setrlimit};
    let input = input
        .canonicalize()
        .map_err(|_| "backend_unavailable: sandbox snapshot")?;
    let profile = CString::new(super::profile::profile(&input)?)
        .map_err(|_| "backend_unavailable: sandbox profile")?;
    for (resource, cap) in [
        (Resource::Cpu, 35),
        (Resource::Fsize, 0),
        (Resource::Core, 0),
        (Resource::Nofile, 64),
    ] {
        setrlimit(
            resource,
            Rlimit {
                current: Some(cap),
                maximum: Some(cap),
            },
        )
        .map_err(|_| "backend_unavailable: helper resource limits")?;
    }
    let mut error = std::ptr::null_mut();
    // SAFETY: profile is NUL-terminated, valid through the call; the error string is freed by its allocator.
    let status = unsafe { sandbox_init(profile.as_ptr(), 0, &raw mut error) };
    if !error.is_null() {
        unsafe { sandbox_free_error(error) };
    }
    if status != 0 {
        return Err("backend_unavailable: macOS media sandbox denied".into());
    }
    Ok(())
}

/// Reads the supervised helper's resident bytes; no task port or elevated permission is requested.
/// # Errors
/// Process exited or the OS refused accounting. The supervisor must fail closed while it is running.
pub fn resident_bytes(pid: u32) -> Result<u64, String> {
    let pid = i32::try_from(pid).map_err(|_| "limit_exceeded: helper process identity")?;
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::uninit();
    let size = i32::try_from(size_of::<libc::proc_taskinfo>())
        .map_err(|_| "limit_exceeded: task info size")?;
    // SAFETY: proc_pidinfo writes at most size bytes into correctly aligned taskinfo storage.
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read != size {
        return Err("backend_unavailable: helper memory accounting".into());
    }
    // SAFETY: the successful call initialized the entire structure.
    Ok(unsafe { info.assume_init() }.pti_resident_size)
}
