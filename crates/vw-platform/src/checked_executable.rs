//! Retains the executable object between the Chrome helper's pin check and launch.
//! Uses OS file handles and sealed Linux snapshots; the caller retains the object while using its path.
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

/// An opened executable, retained through process creation (and through script reads on Linux).
pub struct CheckedExecutable {
    file: File,
    path: PathBuf,
}

impl CheckedExecutable {
    /// Opens once. Windows denies writers and deletion; Linux seals a snapshot before the caller hashes it.
    ///
    /// # Errors
    /// The executable cannot be opened or made stable.
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1); // FILE_SHARE_READ: no write or delete sharing.
        }
        let file = options.open(path)?;
        #[cfg(target_os = "linux")]
        let file = sealed_snapshot(file)?;
        #[cfg(target_os = "linux")]
        let path = {
            use std::os::{fd::AsRawFd, unix::fs::FileExt};
            let mut magic = [0; 4];
            let native = file.read_at(&mut magic, 0)? == 4 && magic == *b"\x7fELF";
            // Native helpers execute the child's own CLOEXEC descriptor, including when the app is
            // non-dumpable. Scripts reopen the parent's fd while it remains held, without inheritance.
            let process = if native {
                "self".to_owned()
            } else {
                std::process::id().to_string()
            };
            PathBuf::from(format!("/proc/{process}/fd/{}", file.as_raw_fd()))
        };
        #[cfg(not(target_os = "linux"))]
        let path = path.to_owned();
        Ok(Self { file, path })
    }

    /// The same opened object used to check the pins.
    pub fn file(&mut self) -> &mut File {
        &mut self.file
    }

    /// Path to execute while retaining this object.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(target_os = "linux")]
fn sealed_snapshot(mut source: File) -> io::Result<File> {
    use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
    use std::io::{Seek, SeekFrom};
    let flags = MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING;
    // EXEC makes the intent explicit on newer kernels. Older supported kernels predate that flag;
    // their default memfd is executable. A policy refusal is never retried with a weaker object.
    let fd = match memfd_create(c"chrome-helper", flags | MemfdFlags::EXEC) {
        Err(rustix::io::Errno::INVAL) => memfd_create(c"chrome-helper", flags)?,
        result => result?,
    };
    let mut snapshot = File::from(fd);
    io::copy(&mut source, &mut snapshot)?;
    snapshot.set_permissions(source.metadata()?.permissions())?;
    fcntl_add_seals(
        &snapshot,
        SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL,
    )?;
    snapshot.seek(SeekFrom::Start(0))?;
    Ok(snapshot)
}

/// Checks the kernel's code-directory hash of a running child, never a pathname lookup.
///
/// # Errors
/// The child is unsigned, gone, unreadable, or not the pinned code identity.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub fn verify_child_cdhash(pid: u32, expected: &str) -> io::Result<()> {
    use std::{
        ffi::{c_int, c_uint, c_void},
        fmt::Write,
    };
    unsafe extern "C" {
        fn csops(pid: c_int, ops: c_uint, address: *mut c_void, size: usize) -> c_int;
    }
    let pid = c_int::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut flags = 0u32;
    // SAFETY: CS_OPS_STATUS writes one u32. Require kill-on-invalid-page as well as current validity,
    // so a copied CodeDirectory cannot bless modified executable pages on Intel Macs either.
    if unsafe {
        csops(
            pid,
            0,
            std::ptr::from_mut(&mut flags).cast(),
            size_of::<u32>(),
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if flags & 0x201 != 0x201 {
        // CS_VALID | CS_KILL
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    let mut hash = [0u8; 20];
    // SAFETY: CS_OPS_CDHASH writes exactly the supplied 20-byte buffer. The kernel identifies the
    // already-executing image, including ad-hoc signatures; it does not reopen the replaced path.
    if unsafe { csops(pid, 5, hash.as_mut_ptr().cast(), hash.len()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let actual = hash
        .iter()
        .fold(String::with_capacity(40), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        });
    if expected.len() != 40 || actual != expected {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn snapshot_bytes_survive_overwrite_and_replacement_and_deny_writers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("helper");
        std::fs::write(&path, b"checked bytes").unwrap();
        let mut executable = CheckedExecutable::open(&path).unwrap();
        std::fs::write(&path, b"overwritten bytes").unwrap();
        std::fs::rename(&path, dir.path().join("old")).unwrap();
        std::fs::write(&path, b"replacement bytes").unwrap();
        let mut actual = Vec::new();
        executable.file().read_to_end(&mut actual).unwrap();
        assert_eq!(actual, b"checked bytes");
        assert!(
            executable
                .file()
                .write_all(b"write through the snapshot handle")
                .is_err()
        );
        let flags = rustix::io::fcntl_getfd(executable.file()).unwrap();
        assert!(flags.contains(rustix::io::FdFlags::CLOEXEC));
    }
}
