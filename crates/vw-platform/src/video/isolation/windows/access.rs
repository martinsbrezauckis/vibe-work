//! `AppContainer` identities, private-file grants and lifetime-bound launch attributes.
//! Only the bootstrap calls these APIs, before any input is decoded.
#![allow(
    clippy::wildcard_imports,
    reason = "Win32 security types and constants form this module's API surface"
)]
use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, Isolation::*, *},
        Storage::FileSystem::*,
        System::{Console::*, Registry::*, Threading::*},
    },
    core::{PCWSTR, PWSTR},
};

pub(super) struct Owned(pub HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: each handle is obtained once and is not shared with another owner.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
pub(super) struct Sid(pub PSID);
impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: CreateAppContainerProfile returns a SID freed with FreeSid.
        unsafe { FreeSid(self.0) };
    }
}
pub(super) fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}
/// Prefix of the per-run `AppContainer` profiles: `<PREFIX><pid>.<creation time>.<nonce>` (hex), one per worker.
/// Windows refuses to start a process in an `AppContainer` whose profile is not registered (`CreateProcessW` fails
/// with "file not found"), so each run registers its own and deletes it afterwards; no identity or container
/// storage is shared between runs. A run that is killed cannot clean up, so `sweep` removes profiles of bootstrap
/// processes that no longer exist. The uninstaller deletes every profile whose name starts with this prefix.
pub const PREFIX: &str = "app.vibework.desktop.video.";
const MAPPINGS: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings";

/// One run's `AppContainer`: a fresh profile and SID. Dropping it revokes every file grant it made and deletes the
/// profile, with its container folders, on every return path of the bootstrap.
pub(super) struct Container {
    name: String,
    sid: Sid,
    granted: Vec<std::path::PathBuf>,
}
impl Container {
    /// Registers a new profile for this run; no capabilities are stored in it and the launch grants none.
    /// Windows requires LOCALAPPDATA in the environment to create (and delete) the profile.
    pub(super) fn create() -> io::Result<Self> {
        use std::hash::BuildHasher;
        let created = process_created(std::process::id())
            .ok_or_else(|| io::Error::other("bootstrap creation time"))?;
        // RandomState is randomly keyed per instance: a nonce without another dependency.
        let nonce = std::collections::hash_map::RandomState::new().hash_one(created) & 0xffff_ffff;
        let name = format!(
            "{PREFIX}{:08x}.{created:016x}.{nonce:08x}",
            std::process::id()
        );
        let wide_name = wide(OsStr::new(&name));
        let display = wide(OsStr::new("Vibe Work video review"));
        // SAFETY: the strings are NUL terminated and live for the call; the SID is freed with FreeSid.
        let sid = unsafe {
            CreateAppContainerProfile(
                PCWSTR(wide_name.as_ptr()),
                PCWSTR(display.as_ptr()),
                PCWSTR(display.as_ptr()),
                None,
            )
        }
        .map_err(io::Error::other)?;
        Ok(Self {
            name,
            sid: Sid(sid),
            granted: Vec::new(),
        })
    }
    pub(super) fn sid(&self) -> PSID {
        self.sid.0
    }
    /// Read (and optionally execute) access for this run's SID only; revoked again when the container drops.
    pub(super) fn grant(&mut self, path: &Path, execute: bool) -> io::Result<()> {
        self.granted.push(path.to_path_buf());
        let permissions = FILE_GENERIC_READ.0 | if execute { FILE_GENERIC_EXECUTE.0 } else { 0 };
        set_access(path, self.sid.0, GRANT_ACCESS, permissions)
    }
    #[cfg(test)]
    pub(super) fn name(&self) -> &str {
        &self.name
    }
}
impl Drop for Container {
    fn drop(&mut self) {
        for path in &self.granted {
            let _ = set_access(path, self.sid.0, REVOKE_ACCESS, 0);
        }
        delete(&self.name);
    }
}
fn delete(name: &str) {
    let name = wide(OsStr::new(name));
    // SAFETY: the name is NUL terminated and live for the call.
    let _ = unsafe { DeleteAppContainerProfile(PCWSTR(name.as_ptr())) };
}
/// The creation time of a running process, which together with its PID names exactly one bootstrap.
fn process_created(pid: u32) -> Option<u64> {
    // SAFETY: the handle is owned and closed; GetProcessTimes writes four stack FILETIMEs.
    unsafe {
        let process = Owned(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?);
        let mut times = [FILETIME::default(); 4];
        let [created, exited, kernel, user] = &mut times;
        GetProcessTimes(process.0, created, exited, kernel, user).ok()?;
        Some(u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime))
    }
}
/// Registered profile names with `PREFIX`, read from the per-user `AppContainer` mappings.
pub(super) fn profiles() -> Vec<String> {
    let path = wide(OsStr::new(MAPPINGS));
    let value = wide(OsStr::new("Moniker"));
    let mut names = Vec::new();
    let mut key = HKEY::default();
    // SAFETY: buffers are stack arrays with their lengths passed in; the key is closed once.
    unsafe {
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            KEY_READ,
            &raw mut key,
        )
        .is_err()
        {
            return names;
        }
        for index in 0.. {
            let mut sid = [0u16; 256];
            let mut length = u32::try_from(sid.len()).unwrap_or(0);
            if RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(sid.as_mut_ptr())),
                &raw mut length,
                None,
                None,
                None,
                None,
            )
            .is_err()
            {
                break;
            }
            let mut moniker = [0u16; 128];
            let mut bytes = u32::try_from(size_of_val(&moniker)).unwrap_or(0);
            if RegGetValueW(
                key,
                PCWSTR(sid.as_ptr()),
                PCWSTR(value.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(moniker.as_mut_ptr().cast()),
                Some(&raw mut bytes),
            )
            .is_ok()
            {
                let units = (bytes as usize / 2).min(moniker.len());
                let text = String::from_utf16_lossy(&moniker[..units]);
                let text = text.trim_end_matches('\0');
                if text.starts_with(PREFIX) {
                    names.push(text.to_owned());
                }
            }
        }
        let _ = RegCloseKey(key);
    }
    names
}
/// Deletes profiles left by bootstrap processes that no longer run (a killed or crashed run). A live run's profile
/// is named by its PID and creation time, so concurrent runs are never touched; only `PREFIX` names are considered.
pub fn sweep() {
    for name in profiles() {
        let mut parts = name[PREFIX.len()..].split('.');
        let owner = parts
            .next()
            .and_then(|pid| u32::from_str_radix(pid, 16).ok());
        let created = parts
            .next()
            .and_then(|time| u64::from_str_radix(time, 16).ok());
        let live = matches!((owner, created), (Some(pid), Some(time)) if process_created(pid) == Some(time));
        if !live {
            delete(&name);
        }
    }
}
fn set_access(path: &Path, sid: PSID, mode: ACCESS_MODE, permissions: u32) -> io::Result<()> {
    let name = wide(path.as_os_str());
    let (mut descriptor, mut acl, mut next) = (
        PSECURITY_DESCRIPTOR::default(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    );
    // SAFETY: all output pointers are correctly typed; the descriptors and ACLs are freed by LocalFree exactly once.
    unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(name.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&raw mut acl),
            None,
            &raw mut descriptor,
        )
        .ok()
        .map_err(io::Error::other)?;
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: permissions,
            grfAccessMode: mode,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: PWSTR(sid.0.cast()),
                ..Default::default()
            },
        };
        let result = SetEntriesInAclW(Some(&[entry]), Some(acl), &raw mut next).ok();
        let result = result.and_then(|()| {
            SetNamedSecurityInfoW(
                PCWSTR(name.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(next),
                None,
            )
            .ok()
        });
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        if !next.is_null() {
            let _ = LocalFree(Some(HLOCAL(next.cast())));
        }
        result.map_err(io::Error::other)
    }
}
pub(super) fn pipe(which: STD_HANDLE) -> io::Result<Owned> {
    // SAFETY: duplicate only a standard pipe with the same rights; attributes restrict inheritance to these copies.
    unsafe {
        let source = GetStdHandle(which).map_err(io::Error::other)?;
        let mut target = HANDLE::default();
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            GetCurrentProcess(),
            &raw mut target,
            0,
            true,
            DUPLICATE_SAME_ACCESS,
        )
        .map_err(io::Error::other)?;
        Ok(Owned(target))
    }
}
pub(super) struct Attributes<'a> {
    storage: Vec<usize>,
    values: std::marker::PhantomData<&'a ()>,
}
impl<'a> Attributes<'a> {
    pub(super) fn new(count: u32) -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: first call only returns required storage; Vec<usize> supplies pointer-aligned backing memory.
        unsafe {
            let _ = InitializeProcThreadAttributeList(None, count, None, &raw mut size);
        }
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
        unsafe {
            InitializeProcThreadAttributeList(
                Some(LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast())),
                count,
                None,
                &raw mut size,
            )
        }
        .map_err(io::Error::other)?;
        Ok(Self {
            storage,
            values: std::marker::PhantomData,
        })
    }
    pub(super) fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.storage.as_mut_ptr().cast())
    }
    pub(super) fn add<T>(&mut self, key: usize, value: &'a T) -> io::Result<()> {
        // SAFETY: callers keep every referenced value live through spawn; no pointer escapes this module.
        unsafe {
            UpdateProcThreadAttribute(
                self.pointer(),
                0,
                key,
                Some(std::ptr::from_ref(value).cast()),
                size_of::<T>(),
                None,
                None,
            )
        }
        .map_err(io::Error::other)
    }
}
impl Drop for Attributes<'_> {
    fn drop(&mut self) {
        // SAFETY: the list was initialized in the owned backing storage.
        unsafe { DeleteProcThreadAttributeList(self.pointer()) };
    }
}
