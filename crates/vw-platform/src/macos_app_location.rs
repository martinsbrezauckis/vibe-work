//! macOS running-application location facts for the in-app updater:
//! - Primary App Translocation check via `Security.framework`'s `SecTranslocateIsTranslocatedURL`,
//!   loaded lazily with `dlopen`/`dlsym` so binaries that never inspect updater modes avoid loading it.
//!   When the symbol is missing or returns an error, returns `None` so callers fall back to the exact
//!   `AppTranslocation` path-component check.
//! - Read-only filesystem check via `rustix::fs::statvfs` (`StatVfsMountFlags::RDONLY`), distinguishing
//!   unwritable read-only volumes (e.g., an unquarantined `.dmg` under `/Volumes/...`, which always fails
//!   with OS error 30 `EROFS`) from normal `/Applications` (where `PermissionDenied` triggers macOS admin
//!   authorization).
//!
//! Depends on: `core-foundation`, `libc`, `rustix`. Called by: `app` (`updater.rs`).

use std::ffi::c_void;
use std::path::Path;
use std::sync::OnceLock;

use core_foundation::base::{Boolean, TCFType};
use core_foundation::error::{CFError, CFErrorRef};
use core_foundation::url::{CFURL, CFURLRef};
use rustix::fs::{StatVfsMountFlags, statvfs};

/// Location facts for a running macOS application bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppLocationFacts {
    /// Result of `SecTranslocateIsTranslocatedURL`, or `None` if unavailable or the call errored.
    pub translocated: Option<bool>,
    /// Whether the enclosing `.app` bundle or its containing folder sits on a read-only filesystem (`ST_RDONLY`).
    pub read_only_fs: bool,
}

type SecTranslocateIsTranslocatedUrlFn = unsafe extern "C" fn(
    path: CFURLRef,
    is_translocated: *mut bool,
    error: *mut CFErrorRef,
) -> Boolean;

fn load_sec_translocate() -> Option<SecTranslocateIsTranslocatedUrlFn> {
    static SYMBOL: OnceLock<Option<SecTranslocateIsTranslocatedUrlFn>> = OnceLock::new();
    *SYMBOL.get_or_init(|| {
        // SAFETY: constant NUL-terminated system framework path; `RTLD_LAZY` keeps it open for the process lifetime.
        let handle = unsafe {
            libc::dlopen(
                c"/System/Library/Frameworks/Security.framework/Security".as_ptr(),
                libc::RTLD_LAZY,
            )
        };
        if handle.is_null() {
            return None;
        }
        // SAFETY: `handle` came from `dlopen` above and the symbol name is a constant NUL-terminated C string.
        let ptr: *mut c_void =
            unsafe { libc::dlsym(handle, c"SecTranslocateIsTranslocatedURL".as_ptr()) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: `SecTranslocateIsTranslocatedURL` in `Security.framework` has C ABI
        // `Boolean (*)(CFURLRef, bool *, CFErrorRef *)`.
        Some(unsafe { std::mem::transmute::<*mut c_void, SecTranslocateIsTranslocatedUrlFn>(ptr) })
    })
}

/// Returns the enclosing `.app` bundle directory for an executable inside `Contents/MacOS`,
/// or the nearest `.app` ancestor, falling back to `executable` itself.
#[must_use]
pub fn app_bundle_or_path(executable: &Path) -> &Path {
    if let Some(macos_dir) = executable.parent()
        && macos_dir.file_name().is_some_and(|n| n == "MacOS")
        && let Some(contents_dir) = macos_dir.parent()
        && contents_dir.file_name().is_some_and(|n| n == "Contents")
        && let Some(bundle_dir) = contents_dir.parent()
    {
        return bundle_dir;
    }
    executable
        .ancestors()
        .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))
        .unwrap_or(executable)
}

/// Queries `SecTranslocateIsTranslocatedURL` for `path`.
/// Returns `None` if `Security.framework` cannot be queried or reports an error.
#[must_use]
pub fn is_translocated(path: &Path) -> Option<bool> {
    let func = load_sec_translocate()?;
    let url = CFURL::from_path(path, path.is_dir())?;
    let mut translocated = false;
    let mut error: CFErrorRef = std::ptr::null_mut();
    // SAFETY: `url.as_concrete_TypeRef()` is a valid `CFURLRef`, `translocated` is a writable `bool`,
    // and `error` receives either null or an owned `CFErrorRef` released below.
    let ok = unsafe {
        func(
            url.as_concrete_TypeRef(),
            &raw mut translocated,
            &raw mut error,
        )
    };
    if !error.is_null() {
        // SAFETY: `SecTranslocateIsTranslocatedURL` follows the Create rule for out-errors.
        let _owned = unsafe { CFError::wrap_under_create_rule(error) };
    }
    (ok != 0).then_some(translocated)
}

/// Returns `true` when the enclosing `.app` bundle (or nearest existing ancestor) sits on a
/// read-only filesystem (`ST_RDONLY`, e.g., a mounted `.dmg` or read-only APFS volume).
/// Stops at the innermost existing ancestor so firmlinked folders like `/Applications` do not
/// inherit the sealed read-only `/` root volume flag.
#[must_use]
pub fn is_read_only_fs(executable: &Path) -> bool {
    app_bundle_or_path(executable)
        .ancestors()
        .find_map(|candidate| statvfs(candidate).ok())
        .is_some_and(|vfs| vfs.f_flag.contains(StatVfsMountFlags::RDONLY))
}

/// Probes macOS App Translocation and read-only mount status for `executable`.
#[must_use]
pub fn facts(executable: Option<&Path>) -> Option<AppLocationFacts> {
    let exe = executable?;
    let bundle = app_bundle_or_path(exe);
    Some(AppLocationFacts {
        translocated: is_translocated(bundle),
        read_only_fs: is_read_only_fs(exe),
    })
}
