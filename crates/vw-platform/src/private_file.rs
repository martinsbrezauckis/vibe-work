//! User-only directories and files, for secrets that must sit on disk for a while (the Debug API's
//! per-run token descriptor).
//!
//! Depends on: std; on Windows the Win32 security API (`windows` crate) to set DACLs and read owners,
//! and the OS tools `whoami.exe` and `icacls.exe` from `%SystemRoot%\System32` (absolute paths, never a
//! PATH lookup) only to read permissions back ([`is_user_only`]). Called by: vw-debug-api, the stores
//! (projects, comments, settings, changes, agent tools, previews).
//! Invariants:
//! - Unix: the directory is mode 0700 and the file mode 0600, created new (`O_CREAT|O_EXCL`, so an
//!   existing file or a planted symlink at the path makes the write fail instead of being
//!   followed). A directory owned by someone else cannot be chmod-ed and is refused.
//! - Windows: the directory's DACL is replaced by one entry, the current user (by SID) with full
//!   control, inherited by new children; inheritance from the parent is removed. A file is created
//!   (`CreateFileW`, `CREATE_NEW`) with the same explicit, protected user-only DACL in its security
//!   descriptor, so it is user-only from its first moment (it never carries the folder's
//!   inherited entries). Both use the Win32 security
//!   API (`os::acl`, with `unsafe` confined to vw-platform). Previously PowerShell handled it,
//!   adding about 0.6 s per store save on Windows 11. Reading back (`is_user_only`) stays on
//!   `icacls`, a second, independent mechanism. Paths past 248 characters are handed to these calls in the
//!   `\\?\` form (`os::long_path`), as `std::fs` does for its own calls, so a deep folder works the same.
//! - A directory owned by another user is refused (Unix: `chmod` fails with EPERM; Windows: its owner is
//!   read first, because replacing the DACL leaves the owner its implicit right to change it back).
//! - Every function fails closed: an error means the caller must not use the path for a secret.
//!
//! Remaining window (documented, not closed): on Windows a new directory briefly carries the
//! inherited ACL of its parent before `icacls` replaces it. Under the user's profile folder that
//! ACL already excludes other non-admin users, and the secret is written only after the directory
//! is user-only.

use std::fs;
use std::io::Write;
use std::path::Path;

/// Why a private directory or file could not be made or checked.
#[derive(Debug, thiserror::Error)]
pub enum PrivateFileError {
    /// A filesystem call failed.
    #[error("{action} {path}: {source}")]
    Io {
        /// What was being done.
        action: &'static str,
        /// The path involved.
        path: String,
        /// The OS error.
        source: std::io::Error,
    },
    /// The path exists but is not what the caller needs (a symlink, a file, wrong owner).
    #[error("{path} cannot be used privately: {reason}")]
    Unusable {
        /// The path involved.
        path: String,
        /// Why.
        reason: String,
    },
    /// An OS permission tool failed or printed something we could not read (Windows).
    #[error("{tool} failed for {path}: {detail}")]
    Tool {
        /// The tool (`whoami`, `icacls`).
        tool: &'static str,
        /// The path involved.
        path: String,
        /// Exit status or parse problem (never file contents).
        detail: String,
    },
}

fn io(action: &'static str, path: &Path, source: std::io::Error) -> PrivateFileError {
    PrivateFileError::Io {
        action,
        path: path.display().to_string(),
        source,
    }
}

/// Creates `path` (and missing parents) as a directory only the current user can use, or makes
/// an existing directory so.
///
/// # Errors
/// [`PrivateFileError`] when the path is a symlink or not a directory, belongs to another user
/// (Unix: `chmod` fails; Windows: the owner is checked, the Administrators group and SYSTEM
/// accepted), or permissions cannot be set.
pub fn ensure_private_dir(path: &Path) -> Result<(), PrivateFileError> {
    fs::create_dir_all(path).map_err(|e| io("create directory", path, e))?;
    let meta = fs::symlink_metadata(path).map_err(|e| io("inspect directory", path, e))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(PrivateFileError::Unusable {
            path: path.display().to_string(),
            reason: "not a plain directory".to_owned(),
        });
    }
    // Unix: `chmod` of a directory owned by someone else fails (EPERM), so `restrict_dir` refuses it.
    #[cfg(windows)]
    os::check_owner(path)?;
    os::restrict_dir(path)
}

/// Creates a new file at `path` that only the current user can use from its creation (Unix: mode 0600; Windows: a
/// protected user-only DACL in the creating call), open for writing. The parent should already be private
/// ([`ensure_private_dir`]). The file must not exist yet.
///
/// # Errors
/// [`PrivateFileError`] when the file exists (also a dangling symlink) or cannot be created.
pub fn create_private_new(path: &Path) -> Result<fs::File, PrivateFileError> {
    os::create_new(path).map_err(|e| io("create file", path, e))
}

/// Writes `bytes` to a new file at `path` that only the current user can read. The parent must
/// already be private ([`ensure_private_dir`]). The file must not exist yet.
///
/// # Errors
/// [`PrivateFileError`] when the file exists (also a dangling symlink), cannot be written or
/// synced, or its permissions cannot be set; a partly written file is removed.
pub fn write_private_new(path: &Path, bytes: &[u8]) -> Result<(), PrivateFileError> {
    let mut file = create_private_new(path)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| io("write file", path, e));
    // Windows: user-only since `create_private_new`; Unix: 0600 asserted again (independent of the umask).
    #[cfg(unix)]
    let written = written.and_then(|()| os::restrict_file(path));
    if written.is_err() {
        let _ = fs::remove_file(path);
    }
    written
}

/// Replaces the file at `path` with `bytes`, atomically: the bytes go to a new user-only file next to it
/// ([`write_private_new`]), which is then renamed over `path`. A crash leaves the old file or the new one, never
/// half of one. The parent must already be private ([`ensure_private_dir`]). Shared by the comment stores
/// (Project Pages, diff line comments).
///
/// # Errors
/// [`PrivateFileError`] when the new file cannot be written or renamed; the temporary file is removed.
pub fn replace_private(path: &Path, bytes: &[u8]) -> Result<(), PrivateFileError> {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".new-{}-{nonce}", std::process::id()));
    let temp = path.with_file_name(name);
    write_private_new(&temp, bytes)?;
    fs::rename(&temp, path).map_err(|e| {
        let _ = fs::remove_file(&temp);
        io("replace file", path, e)
    })
}

/// Moves a file that could not be read aside (to `<name>.bad-<stamp>`) so a store can start empty without ever
/// overwriting it. Returns the new path, or `None` when the move failed.
#[must_use]
pub fn set_aside(path: &Path, stamp: u64) -> Option<std::path::PathBuf> {
    let mut name = path.file_name()?.to_os_string();
    name.push(format!(".bad-{stamp}"));
    let aside = path.with_file_name(name);
    fs::rename(path, &aside).ok().map(|()| aside)
}

/// True when only the current user (and, on Windows, nobody else by an explicit entry) can read
/// `path`. Used by callers to verify what they wrote, and by tests.
///
/// # Errors
/// [`PrivateFileError`] when the permissions cannot be read.
pub fn is_user_only(path: &Path) -> Result<bool, PrivateFileError> {
    os::is_user_only(path)
}

#[cfg(unix)]
mod os {
    //! Unix modes: 0700 directories, 0600 files.
    use std::fs::{self, File, OpenOptions, Permissions};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::Path;

    use super::{PrivateFileError, io};

    pub(super) fn restrict_dir(path: &Path) -> Result<(), PrivateFileError> {
        // Fails with EPERM on a directory owned by another user: refused, as documented.
        fs::set_permissions(path, Permissions::from_mode(0o700))
            .map_err(|e| io("restrict directory", path, e))
    }

    pub(super) fn create_new(path: &Path) -> std::io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }

    pub(super) fn restrict_file(path: &Path) -> Result<(), PrivateFileError> {
        // The umask can only clear bits, so 0600 from `create_new` already holds; set it anyway so
        // the invariant does not depend on that.
        fs::set_permissions(path, Permissions::from_mode(0o600))
            .map_err(|e| io("restrict file", path, e))
    }

    #[allow(
        clippy::verbose_bit_mask,
        reason = "`mode & 0o077 == 0` reads as 'no group or other bits'"
    )]
    pub(super) fn is_user_only(path: &Path) -> Result<bool, PrivateFileError> {
        let meta = fs::symlink_metadata(path).map_err(|e| io("inspect", path, e))?;
        Ok(!meta.file_type().is_symlink() && meta.permissions().mode() & 0o077 == 0)
    }
}

#[cfg(windows)]
mod os {
    //! Windows DACLs: set with `SetNamedSecurityInfoW` (a protected one-entry DACL for the SID in the
    //! process token), read back with `icacls`, keyed by the current user's name from `whoami`.
    use std::fs::File;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::PrivateFileError;

    /// `CREATE_NO_WINDOW`: no console window flashes when a GUI app runs the tool.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Paths of this many UTF-16 units or more get the long form ([`long_path`]): the shortest limit of the Win32
    /// calls, a directory name (260 minus room for an 8.3 file name), the same threshold std uses.
    const LONG_FROM: usize = 248;

    /// `path` as Win32 calls accept it past 260 characters. `std::fs` converts long paths itself; the calls made
    /// here directly (`CreateFileW`, the named security calls, `icacls`) failed instead ("The system cannot find
    /// the path specified."), so a data folder deeper than about 120 characters could not hold the session lease
    /// records. A long path is made absolute the way Windows does it (`std::path::absolute`: `/` becomes `\`,
    /// `.` and `..` are applied) and gets the `\\?\` prefix (`\\?\UNC\` for a network share). Shorter paths, and
    /// paths that already start with `\\?\` or `\\.\`, are passed exactly as before.
    pub(super) fn long_path(path: &Path) -> std::borrow::Cow<'_, Path> {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        if path.as_os_str().encode_wide().count() < LONG_FROM {
            return std::borrow::Cow::Borrowed(path);
        }
        let Ok(full) = std::path::absolute(path) else {
            return std::borrow::Cow::Borrowed(path);
        };
        let units: Vec<u16> = full.as_os_str().encode_wide().collect();
        let starts = |prefix: &str| {
            let prefix: Vec<u16> = prefix.encode_utf16().collect();
            units.starts_with(&prefix)
        };
        if starts(r"\\?\") || starts(r"\\.\") {
            return std::borrow::Cow::Owned(full);
        }
        let (prefix, rest) = if starts(r"\\") {
            (r"\\?\UNC\", &units[2..])
        } else {
            (r"\\?\", &units[..])
        };
        let mut long: Vec<u16> = prefix.encode_utf16().collect();
        long.extend_from_slice(rest);
        std::borrow::Cow::Owned(PathBuf::from(std::ffi::OsString::from_wide(&long)))
    }

    fn system_tool(name: &str) -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join(name)
    }

    fn run(
        tool: &'static str,
        args: &[&std::ffi::OsStr],
        path: &Path,
    ) -> Result<String, PrivateFileError> {
        let fail = |detail: String| PrivateFileError::Tool {
            tool,
            path: path.display().to_string(),
            detail,
        };
        let output = Command::new(system_tool(tool))
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| fail(e.to_string()))?;
        if !output.status.success() {
            return Err(fail(failure_detail(&output)));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Exit status plus the tool's own error text, whitespace-collapsed and capped at 400 chars,
    /// so a failure on another machine can be diagnosed from the log. Uses stderr, or stdout when
    /// stderr is empty (`icacls` and `whoami` print errors there). These tools only see paths and
    /// SIDs, never secrets.
    fn failure_detail(output: &std::process::Output) -> String {
        let text = if output.stderr.iter().all(u8::is_ascii_whitespace) {
            String::from_utf8_lossy(&output.stdout)
        } else {
            String::from_utf8_lossy(&output.stderr)
        };
        let reason: String = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(400)
            .collect();
        format!("exit status {}: {reason}", output.status)
    }

    /// The current user's SID and `DOMAIN\name`, from `whoami /user /fo csv /nh`
    /// (`"desktop-1\user","S-1-5-21-..."`).
    fn current_user(path: &Path) -> Result<(String, String), PrivateFileError> {
        let text = run(
            "whoami.exe",
            &[
                "/user".as_ref(),
                "/fo".as_ref(),
                "csv".as_ref(),
                "/nh".as_ref(),
            ],
            path,
        )?;
        let fields: Vec<&str> = text
            .trim()
            .split(',')
            .map(|f| f.trim_matches('"'))
            .collect();
        match fields.as_slice() {
            [name, sid] if sid.starts_with("S-1-") => Ok(((*sid).to_owned(), (*name).to_owned())),
            _ => Err(PrivateFileError::Tool {
                tool: "whoami.exe",
                path: path.display().to_string(),
                detail: "unexpected output shape".to_owned(),
            }),
        }
    }

    /// Replaces the whole DACL of `path` with one protected entry: full control for the current
    /// user (`D:P(A;OICI;FA;;;<SID>)` for folders, `D:P(A;;FA;;;<SID>)` for files), through the
    /// Win32 security API ([`acl`]).
    ///
    /// WORKAROUND(Windows): a fresh protected descriptor, not an edit of the existing one: `icacls
    /// /inheritance:r /grant:r` keeps explicit entries of other principals, and `icacls /remove`
    /// cannot delete an entry whose SID does not resolve (error 1332). Both seen on Windows 11, where another
    /// developer tool's sandbox adds its own users group and an unresolvable account SID with Modify
    /// rights to workspace folders. Only the DACL section is written, which needs no privilege (writing
    /// a whole descriptor, as the `Set-Acl` cmdlet does, needs `SeSecurityPrivilege`, held only
    /// elevated: Windows 11).
    ///
    /// PERF(Windows): this used to run `whoami.exe` and a PowerShell script per call, about
    /// 0.4-0.7 s each on Windows 11, so every store save (projects, comments, settings) took about 0.6 s and a
    /// hard-kill test never saw a write finish. The API call takes well under a millisecond.
    fn grant_only_user(path: &Path, inherit: bool) -> Result<(), PrivateFileError> {
        let fail = |e: std::io::Error| PrivateFileError::Io {
            action: "restrict",
            path: path.display().to_string(),
            source: e,
        };
        let sid = acl::current_user_sid().map_err(fail)?;
        acl::set_only_user(path, &sid, inherit).map_err(fail)
    }

    /// Refuses a directory owned by anyone but the current user, the Administrators group (the owner of
    /// what an elevated administrator creates) or SYSTEM: replacing the DACL leaves another owner its
    /// implicit right to change the DACL back (a pre-created folder can otherwise be accepted).
    pub(super) fn check_owner(path: &Path) -> Result<(), PrivateFileError> {
        const ADMINISTRATORS: &str = "S-1-5-32-544";
        const SYSTEM: &str = "S-1-5-18";
        let fail = |e: std::io::Error| PrivateFileError::Io {
            action: "read the owner of",
            path: path.display().to_string(),
            source: e,
        };
        let sid = acl::current_user_sid().map_err(fail)?;
        let owner = acl::owner_sid(path).map_err(fail)?;
        if owner == sid || owner == ADMINISTRATORS || owner == SYSTEM {
            Ok(())
        } else {
            Err(PrivateFileError::Unusable {
                path: path.display().to_string(),
                reason: format!("owned by {owner}, not by the current user"),
            })
        }
    }

    #[allow(
        unsafe_code,
        reason = "Win32 security calls without a safe wrapper (unsafe code is kept inside vw-platform)"
    )]
    mod acl {
        //! The Win32 security calls behind `grant_only_user`, `create_new` and `check_owner`: the current
        //! user's SID from the process token, a DACL from SDDL written with `SetNamedSecurityInfoW` or given
        //! to `CreateFileW` at creation, a file's owner with `GetNamedSecurityInfoW`. Every buffer the OS allocates is freed with `LocalFree`
        //! exactly once, on every path (the [`Local`] guard).
        use std::io;
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::{FromRawHandle as _, OwnedHandle};
        use std::path::Path;
        use std::sync::OnceLock;

        use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree, WIN32_ERROR};
        use windows::Win32::Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
        };
        use windows::Win32::Security::{
            ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation,
            OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
        };
        use windows::Win32::Storage::FileSystem::{
            CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        use windows::core::{BOOL, PCWSTR, PWSTR};

        /// Memory the OS allocated with `LocalAlloc`, freed once on drop.
        struct Local(*mut core::ffi::c_void);

        impl Drop for Local {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    // SAFETY: the pointer came from an API documented to allocate with LocalAlloc
                    // (ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                    // GetNamedSecurityInfoW) and is freed only here, once.
                    let _ = unsafe { LocalFree(Some(HLOCAL(self.0))) };
                }
            }
        }

        /// The NUL-terminated name the calls take, in the long form past 248 characters ([`super::long_path`]).
        fn wide(path: &Path) -> Vec<u16> {
            super::long_path(path)
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect()
        }

        fn status(code: WIN32_ERROR) -> io::Result<()> {
            if code.0 == 0 {
                Ok(())
            } else {
                #[allow(
                    clippy::cast_possible_wrap,
                    reason = "Win32 error codes are small positive numbers"
                )]
                Err(io::Error::from_raw_os_error(code.0 as i32))
            }
        }

        /// `S-1-5-...` text of `sid`.
        ///
        /// # Safety
        /// `sid` must point to a valid SID for the duration of the call.
        unsafe fn sid_text(sid: PSID) -> io::Result<String> {
            let mut text = PWSTR::null();
            // SAFETY: the caller guarantees `sid`; `text` is a valid out-pointer.
            unsafe { ConvertSidToStringSidW(sid, &raw mut text) }.map_err(io::Error::other)?;
            let _free = Local(text.0.cast());
            // SAFETY: on success `text` is a NUL-terminated wide string the call allocated.
            String::from_utf16(unsafe { text.as_wide() }).map_err(io::Error::other)
        }

        /// The SID of the user this process runs as. Cached: a process's user never changes.
        pub(super) fn current_user_sid() -> io::Result<String> {
            static SID: OnceLock<String> = OnceLock::new();
            if let Some(sid) = SID.get() {
                return Ok(sid.clone());
            }
            let mut token = HANDLE::default();
            // SAFETY: the pseudo handle of the current process needs no closing; `token` is a valid
            // out-pointer and is closed below.
            unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) }
                .map_err(io::Error::other)?;
            let sid = token_user_sid(token);
            // SAFETY: `token` is the open handle from above, closed exactly once.
            let _ = unsafe { CloseHandle(token) };
            let sid = sid?;
            Ok(SID.get_or_init(|| sid).clone())
        }

        fn token_user_sid(token: HANDLE) -> io::Result<String> {
            let mut needed = 0_u32;
            // SAFETY: a null buffer of length 0 only asks for the size (the call fails with
            // ERROR_INSUFFICIENT_BUFFER and sets `needed`).
            let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &raw mut needed) };
            if needed == 0 {
                return Err(io::Error::other("the token's user could not be sized"));
            }
            // u64 elements: the buffer is suitably aligned for TOKEN_USER (pointers inside).
            let mut buffer = vec![0_u64; (needed as usize).div_ceil(8)];
            // SAFETY: `buffer` is writable and at least `needed` bytes long.
            unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    Some(buffer.as_mut_ptr().cast()),
                    needed,
                    &raw mut needed,
                )
            }
            .map_err(io::Error::other)?;
            // SAFETY: on success the buffer starts with a TOKEN_USER whose SID points into the buffer,
            // which lives until the end of this function.
            unsafe {
                let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
                sid_text(user.User.Sid)
            }
        }

        /// A security descriptor holding a protected DACL with one full-control entry for `sid` (inherited by new
        /// children when `inherit`); freed by the returned guard.
        fn only_user_descriptor(
            sid: &str,
            inherit: bool,
        ) -> io::Result<(PSECURITY_DESCRIPTOR, Local)> {
            // The SID comes from the process token (`S-1-` digits and dashes), never from a caller.
            if !sid.starts_with("S-1-")
                || !sid
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b == b'-' || b == b'S')
            {
                return Err(io::Error::other("unexpected SID text"));
            }
            let flags = if inherit { "OICI" } else { "" };
            let sddl: Vec<u16> = format!("D:P(A;{flags};FA;;;{sid})")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            // SAFETY: `sddl` is NUL-terminated and lives across the call; `descriptor` is a valid
            // out-pointer, freed by the guard.
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(sddl.as_ptr()),
                    SDDL_REVISION_1,
                    &raw mut descriptor,
                    None,
                )
            }
            .map_err(io::Error::other)?;
            Ok((descriptor, Local(descriptor.0)))
        }

        /// Writes a protected DACL with one full-control entry for `sid` (inherited by new children
        /// when `inherit`) to `path`.
        pub(super) fn set_only_user(path: &Path, sid: &str, inherit: bool) -> io::Result<()> {
            let (descriptor, _free) = only_user_descriptor(sid, inherit)?;
            let mut present = BOOL::default();
            let mut defaulted = BOOL::default();
            let mut dacl: *mut ACL = std::ptr::null_mut();
            // SAFETY: `descriptor` is the valid descriptor from above; the out-pointers are valid; the
            // DACL pointer points into the descriptor, which outlives its use below.
            unsafe {
                GetSecurityDescriptorDacl(
                    descriptor,
                    &raw mut present,
                    &raw mut dacl,
                    &raw mut defaulted,
                )
            }
            .map_err(io::Error::other)?;
            if !present.as_bool() || dacl.is_null() {
                return Err(io::Error::other("the descriptor has no DACL"));
            }
            let name = wide(path);
            // SAFETY: `name` is NUL-terminated and lives across the call; `dacl` is valid (above). Only
            // the DACL (and its protection flag) is written.
            status(unsafe {
                SetNamedSecurityInfoW(
                    PCWSTR(name.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(dacl.cast_const()),
                    None,
                )
            })
        }

        /// Creates the new file `path` (`CREATE_NEW`: an existing name, also a link, is refused) for writing, with a
        /// protected one-entry full-control DACL for `sid` given in the creating call, so the file never carries
        /// its folder's inherited entries. Sharing as `std::fs` opens files (read, write, delete).
        pub(super) fn create_only_user(path: &Path, sid: &str) -> io::Result<std::fs::File> {
            let (descriptor, _free) = only_user_descriptor(sid, false)?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                    .map_err(io::Error::other)?,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: BOOL::from(false),
            };
            let name = wide(path);
            // SAFETY: `name` is NUL-terminated and lives across the call; `attributes` and the descriptor it points
            // to (freed by the guard after the call) are valid for the call. On success the handle is new and owned
            // by the returned File only.
            let handle = unsafe {
                CreateFileW(
                    PCWSTR(name.as_ptr()),
                    FILE_GENERIC_WRITE.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    Some(&raw const attributes),
                    CREATE_NEW,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
            }
            .map_err(|e| io::Error::from_raw_os_error(e.code().0 & 0xFFFF))?;
            // SAFETY: `handle` is a valid, newly opened file handle that nothing else owns.
            let owned = unsafe { OwnedHandle::from_raw_handle(handle.0) };
            Ok(std::fs::File::from(owned))
        }

        /// The owner SID of `path`.
        pub(super) fn owner_sid(path: &Path) -> io::Result<String> {
            let name = wide(path);
            let mut owner = PSID::default();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            // SAFETY: `name` is NUL-terminated and lives across the call; the out-pointers are valid;
            // the descriptor is freed by the guard and `owner` points into it.
            status(unsafe {
                GetNamedSecurityInfoW(
                    PCWSTR(name.as_ptr()),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION,
                    Some(&raw mut owner),
                    None,
                    None,
                    None,
                    &raw mut descriptor,
                )
            })?;
            let _free = Local(descriptor.0);
            if owner.0.is_null() {
                return Err(io::Error::other("no owner"));
            }
            // SAFETY: `owner` points into `descriptor`, alive until the guard drops after this call.
            unsafe { sid_text(owner) }
        }
    }

    /// The principals named by the access entries of `icacls <path>` (names, or bare SIDs when a
    /// SID does not resolve). Lines without `:(` (summary line, blanks) are not entries.
    fn principals(path: &Path) -> Result<Vec<String>, PrivateFileError> {
        // icacls prints the path as it was given, so the prefix stripped below is the same long form.
        let long = long_path(path);
        let listing = run("icacls.exe", &[long.as_os_str()], path)?;
        let shown = long.display().to_string();
        Ok(listing
            .lines()
            .filter(|line| line.contains(":("))
            .map(|line| {
                let line = line.strip_prefix(shown.as_str()).unwrap_or(line).trim();
                line.split(":(")
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            })
            .collect())
    }

    pub(super) fn restrict_dir(path: &Path) -> Result<(), PrivateFileError> {
        grant_only_user(path, true)
    }

    /// A new file with the current user's protected one-entry DACL from its creation ([`acl::create_only_user`]).
    pub(super) fn create_new(path: &Path) -> std::io::Result<File> {
        acl::create_only_user(path, &acl::current_user_sid()?)
    }

    /// Reads `icacls <path>`: every access entry line is `<principal>:<flags>`; the path is
    /// user-only when there is exactly one entry and it names the current user. Lines without
    /// `:(` (the localized summary line, blank lines) are not entries.
    pub(super) fn is_user_only(path: &Path) -> Result<bool, PrivateFileError> {
        let (_, name) = current_user(path)?;
        let entries = principals(path)?;
        Ok(entries.len() == 1 && entries[0].eq_ignore_ascii_case(&name))
    }
}
