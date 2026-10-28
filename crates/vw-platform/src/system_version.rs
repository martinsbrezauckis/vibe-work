//! This computer's system version, for checks that compare it with what a program needs:
//! - macOS through `sw_vers -productVersion` (`11.7.10`, `14.0`, `27.0.1`);
//! - Windows from the registry (`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`: the major and minor version
//!   numbers, `CurrentBuildNumber` and the update revision `UBR`, as `10.0.26300.9711`, the numbers `ver` prints).
//!   Not `GetVersionEx`: without a compatibility manifest it reports Windows 8 (6.2) to the program, whatever runs;
//! - Linux: no system version (a distribution's version says nothing about whether a program starts); the running C
//!   library instead ([`glibc_version`], `gnu_get_libc_version`), which is what an `AppImage`'s programs need.
//!
//! A value that cannot be read is `None`, so nothing that depends on it restricts anything.
//!
//! Depends on: std, `windows-registry` (Windows), libc (Linux). Called by: app (the update check), `webview`
//! (macOS data stores).

/// The system's name and version (`("macOS", "14.0")`, `("Windows", "10.0.26300.9711")`), `None` when it is not
/// read (Linux) or cannot be.
#[must_use]
pub fn system_version() -> Option<(&'static str, String)> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()?;
        let text = String::from_utf8(output.stdout).ok()?;
        let version = text.trim();
        (output.status.success() && !version.is_empty()).then(|| ("macOS", version.to_owned()))
    }
    #[cfg(windows)]
    {
        windows_version().map(|version| ("Windows", version))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

/// `major.minor.build[.revision]` from the registry; the revision is left out when the value is missing.
#[cfg(windows)]
fn windows_version() -> Option<String> {
    let key = windows_registry::LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()?;
    let major = key.get_u32("CurrentMajorVersionNumber").ok()?;
    let minor = key.get_u32("CurrentMinorVersionNumber").ok()?;
    let build: u32 = key
        .get_string("CurrentBuildNumber")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(match key.get_u32("UBR") {
        Ok(revision) => format!("{major}.{minor}.{build}.{revision}"),
        Err(_) => format!("{major}.{minor}.{build}"),
    })
}

/// The running C library's version on Linux with glibc (`2.39`), `None` elsewhere or when it cannot be read.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[allow(unsafe_code)]
#[must_use]
pub fn glibc_version() -> Option<String> {
    // SAFETY: `gnu_get_libc_version` takes no arguments and returns a pointer to a NUL-terminated string that glibc
    // owns for the life of the process (it is never freed or changed); a null pointer is checked before reading.
    let text = unsafe {
        let pointer = libc::gnu_get_libc_version();
        if pointer.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(pointer)
    };
    let version = text.to_str().ok()?.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

/// The running C library's version on Linux with glibc; `None` on this system.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
#[must_use]
pub fn glibc_version() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    /// Windows: the real build with its update revision (`10.0.26300.9711`), the same numbers `ver` prints
    /// ("Microsoft Windows [Version 10.0.26300.9711]"); never the 6.2 that `GetVersionEx` reports to programs
    /// without a compatibility manifest.
    #[cfg(windows)]
    #[test]
    fn windows_reads_the_real_build() {
        let Some((os, version)) = super::system_version() else {
            panic!("the Windows version was not read");
        };
        assert_eq!(os, "Windows");
        let parts: Vec<u64> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        assert_eq!(parts.len(), 4, "{version}");
        assert_eq!(parts[..2], [10, 0], "{version}");
        assert!(parts[2] >= 10240, "{version}");
        let ver = std::process::Command::new("cmd")
            .args(["/d", "/c", "ver"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        assert!(
            ver.contains(&format!("Version {version}]")),
            "{ver:?} vs {version}"
        );
    }

    /// Linux: the running C library's version, as `getconf GNU_LIBC_VERSION` prints it ("glibc 2.43").
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn linux_reads_the_running_glibc() {
        let Some(version) = super::glibc_version() else {
            panic!("the glibc version was not read");
        };
        let getconf = std::process::Command::new("getconf")
            .arg("GNU_LIBC_VERSION")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        assert_eq!(getconf, format!("glibc {version}"));
        // The system version is not read on Linux (a distribution's version says nothing about what runs).
        assert_eq!(super::system_version(), None);
    }
}
