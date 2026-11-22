//! Whether an installed Windows copy of the app is complete: an installer stopped while it copied files leaves the
//! app able to start while a program it ships beside it is missing or cut short. Only an installed copy is judged (its
//! folder holds the uninstaller, or it is the installer's default folder), so a build run from elsewhere never is.
//! A program counts as cut short when its file is shorter than its own executable headers say (every section, and
//! the signature at the end of a signed file); nothing is run and only the headers are read.
//!
//! A program that is complete on disk can still fail to start: Windows refuses to load it when a DLL it imports is
//! absent ([`start_problem`]; a program built against the Visual C++ runtime on a Windows without that runtime). Such
//! a program is started once, without arguments and without a window, and its exit status read; every shipped helper
//! answers that with a usage error at once.
//!
//! Depends on: std, vw-platform (window-less spawn). Called by: `app` (startup, Windows user builds).

use std::io::Read as _;
use std::path::Path;

/// The programs the Windows installer places beside the app.
pub const HELPERS: [&str; 2] = ["vw-agent-tools.exe", "vw-video-helper.exe"];
/// The uninstaller the installer writes beside the app.
const UNINSTALLER: &str = "uninstall.exe";
/// How many bytes of a program's start hold the headers that are read.
const HEADERS: u64 = 4096;

/// The first program of [`HELPERS`] that is missing or cut short beside `exe` (then a missing uninstaller), when `exe`
/// is an installed copy; `None` when it is complete or not an installed copy. `default_folder`: the installer's default install folder.
#[must_use]
pub fn damaged_helper(exe: &Path, default_folder: Option<&Path>) -> Option<&'static str> {
    let folder = exe.parent()?;
    let has_uninstaller = folder.join(UNINSTALLER).is_file();
    if !installed_copy(exe, default_folder) {
        return None;
    }
    // A copy in the installer's own folder without its uninstaller is incomplete too: the person could not remove it.
    HELPERS
        .into_iter()
        .find(|name| !complete_file(&folder.join(name)))
        .or((!has_uninstaller).then_some(UNINSTALLER))
}

/// Whether `exe` is an installed copy: its folder holds the uninstaller, or it is the installer's default folder.
#[must_use]
pub fn installed_copy(exe: &Path, default_folder: Option<&Path>) -> bool {
    exe.parent().is_some_and(|folder| {
        folder.join(UNINSTALLER).is_file() || default_folder.is_some_and(|d| same_folder(d, folder))
    })
}

/// Whether Windows lists the product under its installed apps: the installer's uninstall entry exists in the person's
/// or the machine's registry (either registry view). An interrupted install can leave it out; Settings > Apps then
/// cannot remove the app.
#[cfg(windows)]
#[must_use]
pub fn uninstall_entry_present(product: &str) -> bool {
    use winreg::RegKey;
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };
    let path = format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{product}");
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .any(|hive| {
            [KEY_WOW64_64KEY, KEY_WOW64_32KEY].into_iter().any(|view| {
                RegKey::predef(hive)
                    .open_subkey_with_flags(&path, KEY_READ | view)
                    .is_ok()
            })
        })
}

fn same_folder(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn complete_file(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(length) = file.metadata().map(|m| m.len()) else {
        return false;
    };
    let mut head = Vec::new();
    if file.take(HEADERS).read_to_end(&mut head).is_err() {
        return false;
    }
    required_length(&head).is_some_and(|needed| needed <= length)
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The file length a Windows executable's headers (`head`, its first bytes) promise: the end of its last section's
/// data and of its signature. `None` when the headers are not those of an executable or are cut short themselves.
#[must_use]
pub fn required_length(head: &[u8]) -> Option<u64> {
    if head.get(..2)? != b"MZ" {
        return None;
    }
    let pe = usize::try_from(u32_at(head, 0x3C)?).ok()?;
    if head.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let coff = pe + 4;
    let sections = usize::from(u16_at(head, coff + 2)?);
    let optional = coff + 20;
    let optional_size = usize::from(u16_at(head, coff + 16)?);
    // The directory count and the data directories sit at other places in 32- and 64-bit headers; the signature's
    // place is data directory 4 (security), as a file offset.
    let (count, directories) = match u16_at(head, optional)? {
        0x10b => (optional + 92, optional + 96),
        0x20b => (optional + 108, optional + 112),
        _ => return None,
    };
    let mut needed = u64::try_from(optional + optional_size + sections * 40).ok()?;
    if u32_at(head, count)? > 4 {
        let security = directories + 4 * 8;
        let (offset, size) = (u32_at(head, security)?, u32_at(head, security + 4)?);
        needed = needed.max(u64::from(offset) + u64::from(size));
    }
    let table = optional + optional_size;
    for index in 0..sections {
        let section = table + index * 40;
        let (size, pointer) = (u32_at(head, section + 16)?, u32_at(head, section + 20)?);
        needed = needed.max(u64::from(pointer) + u64::from(size));
    }
    Some(needed)
}

/// The Chrome and Edge helper the Windows installer places in the app's resources (absent from builds without it).
pub const CHROME_HELPER: [&str; 3] = ["chrome-control", "bin", "chrome-control.exe"];

/// The first shipped program of an installed copy that is present but cannot start (file name, why), checked in the
/// order [`HELPERS`]; `None` when each starts or `exe` is not an installed copy. The Chrome helper needs a release
/// fingerprint and is checked only by [`start_problems_with_pin`]. A missing program is [`damaged_helper`]'s to report.
#[must_use]
pub fn helper_start_problem(
    exe: &Path,
    default_folder: Option<&Path>,
) -> Option<(&'static str, String)> {
    start_problems(exe, default_folder).into_iter().next()
}

/// Every shipped program of an installed copy that is present but cannot start (file name, why), in the order of
/// [`helper_start_problem`]; empty when each starts or `exe` is not an installed copy. Chrome is skipped without a
/// fingerprint. Each included program is started once.
#[must_use]
pub fn start_problems(exe: &Path, default_folder: Option<&Path>) -> Vec<(&'static str, String)> {
    start_problems_with_pin(exe, default_folder, None)
}

/// Startup probes with the release's browser-helper fingerprint. An absent or differing pin
/// leaves that helper for the existing integrity notice; it must never be executed by the probe.
#[must_use]
pub fn start_problems_with_pin(
    exe: &Path,
    default_folder: Option<&Path>,
    chrome_pin: Option<&str>,
) -> Vec<(&'static str, String)> {
    let Some(folder) = exe.parent() else {
        return Vec::new();
    };
    if !installed_copy(exe, default_folder) {
        return Vec::new();
    }
    let chrome: std::path::PathBuf = CHROME_HELPER.iter().collect();
    HELPERS
        .iter()
        .map(|name| (*name, folder.join(name)))
        .chain(
            std::iter::once((CHROME_HELPER[2], folder.join(chrome)))
                .filter(|(_, path)| chrome_pin.is_some_and(|pin| fingerprint_matches(path, pin))),
        )
        .filter(|(_, path)| path.is_file())
        .filter_map(|(name, path)| start_problem(&path).map(|problem| (name, problem)))
        .collect()
}

fn fingerprint_matches(path: &Path, expected: &str) -> bool {
    use sha2::{Digest, Sha256};
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => digest.update(&buffer[..size]),
            Err(_) => return false,
        }
    }
    format!("{:x}", digest.finalize()).eq_ignore_ascii_case(expected)
}

/// How long a start probe waits for the program to exit. A program still running then is reported as starting (a
/// slow first start, for example while a virus scanner reads it, is not a failure).
pub const START_PROBE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Why the program at `path` cannot start, in words for a person or an agent (`None`: it starts, or the probe could not
/// tell within [`START_PROBE_WAIT`]). The program runs once without arguments, input or window.
#[must_use]
pub fn start_problem(path: &Path) -> Option<String> {
    start_problem_within(path, START_PROBE_WAIT)
}

/// [`start_problem`] with its own wait.
#[must_use]
pub fn start_problem_within(path: &Path, wait: std::time::Duration) -> Option<String> {
    use std::process::Stdio;
    let mut command = vw_platform::process_command::hidden(path);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Some(format!("it could not be started ({error})")),
    };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code().and_then(loader_failure),
            Ok(None) if started.elapsed() < wait => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// The words for an exit status with which Windows refuses to load a program before any of its code runs (the
/// status as `ExitStatus::code` gives it); `None` for every other status, including a program's own error exit.
#[must_use]
pub fn loader_failure(code: i32) -> Option<String> {
    // The NTSTATUS values are 32-bit patterns; `code` carries them as a signed number.
    let status = u32::from_ne_bytes(code.to_ne_bytes());
    let why = match status {
        0xC000_0135 => "a DLL it needs is missing",
        0xC000_0139 => "a DLL it needs is too old (entry point not found)",
        0xC000_007B => "it or a DLL it needs is not a valid program for this Windows",
        0xC000_0142 => "a DLL it needs failed to start",
        _ => return None,
    };
    Some(format!(
        "Windows could not load it: error {status:#010X}, {why}"
    ))
}

#[cfg(test)]
#[path = "install_check_tests.rs"]
mod tests;
