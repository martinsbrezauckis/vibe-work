//! An installed copy with a missing or cut-short program is reported; a complete one, or a copy that is not
//! installed, is not.
//!
//! PREDICT: before the check an installer stopped while it copied files left a copy that started without a word
//! (`damaged_helper` answered nothing), so the person never learned to run the installer again.

#[cfg(unix)]
use super::start_problem_within;
use super::{
    HELPERS, damaged_helper, helper_start_problem, installed_copy, loader_failure, required_length,
    start_problem, start_problems,
};
use std::path::Path;

/// A 64-bit executable's headers with `sections` (file offset, size) and an optional signature (offset, size), padded
/// to the length the headers promise.
fn executable(sections: &[(u32, u32)], signature: Option<(u32, u32)>) -> Vec<u8> {
    let pe = 0x80usize;
    let optional = pe + 4 + 20;
    let optional_size = 112 + 16 * 8;
    let mut bytes = vec![0u8; 0x400];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3C..0x40].copy_from_slice(&u32::try_from(pe).unwrap().to_le_bytes());
    bytes[pe..pe + 4].copy_from_slice(b"PE\0\0");
    let count = u16::try_from(sections.len()).unwrap();
    bytes[pe + 6..pe + 8].copy_from_slice(&count.to_le_bytes());
    bytes[pe + 20..pe + 22].copy_from_slice(&u16::try_from(optional_size).unwrap().to_le_bytes());
    bytes[optional..optional + 2].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[optional + 108..optional + 112].copy_from_slice(&16u32.to_le_bytes());
    if let Some((offset, size)) = signature {
        let security = optional + 112 + 4 * 8;
        bytes[security..security + 4].copy_from_slice(&offset.to_le_bytes());
        bytes[security + 4..security + 8].copy_from_slice(&size.to_le_bytes());
    }
    let table = optional + optional_size;
    for (index, (pointer, size)) in sections.iter().enumerate() {
        let section = table + index * 40;
        bytes[section + 16..section + 20].copy_from_slice(&size.to_le_bytes());
        bytes[section + 20..section + 24].copy_from_slice(&pointer.to_le_bytes());
    }
    let needed = usize::try_from(required_length(&bytes).unwrap()).unwrap();
    bytes.resize(needed.max(bytes.len()), 0);
    bytes
}

fn install(folder: &Path, uninstaller: bool) {
    std::fs::write(folder.join("vibe-work.exe"), b"app").unwrap();
    if uninstaller {
        std::fs::write(folder.join("uninstall.exe"), b"uninstaller").unwrap();
    }
    for name in HELPERS {
        std::fs::write(
            folder.join(name),
            executable(&[(0x400, 0x2000), (0x2400, 0x800)], Some((0x2C00, 0x300))),
        )
        .unwrap();
    }
}

#[test]
fn the_headers_promise_the_end_of_the_last_section_and_of_the_signature() {
    let signed = executable(&[(0x400, 0x200), (0x600, 0x1000)], Some((0x1600, 0x80)));
    assert_eq!(required_length(&signed), Some(0x1680));
    let unsigned = executable(&[(0x400, 0x200), (0x600, 0x1000)], None);
    assert_eq!(required_length(&unsigned), Some(0x1600));
    assert_eq!(required_length(b"\x7fELF"), None, "not an executable");
    assert_eq!(required_length(&signed[..0x90]), None, "headers cut short");
}

#[test]
fn a_complete_installed_copy_is_not_reported() {
    let folder = tempfile::tempdir().unwrap();
    install(folder.path(), true);
    assert_eq!(
        damaged_helper(&folder.path().join("vibe-work.exe"), None),
        None
    );
}

#[test]
fn a_missing_or_cut_short_program_of_an_installed_copy_is_reported() {
    let folder = tempfile::tempdir().unwrap();
    install(folder.path(), true);
    let exe = folder.path().join("vibe-work.exe");
    // Cut short as an installer stopped while it copied the file: the headers arrived, the rest did not.
    let agent_tools = folder.path().join(HELPERS[0]);
    let bytes = std::fs::read(&agent_tools).unwrap();
    std::fs::write(&agent_tools, &bytes[..0x1000]).unwrap();
    assert_eq!(damaged_helper(&exe, None), Some(HELPERS[0]));
    std::fs::write(&agent_tools, &bytes).unwrap();
    std::fs::remove_file(folder.path().join(HELPERS[1])).unwrap();
    assert_eq!(damaged_helper(&exe, None), Some(HELPERS[1]));
}

#[test]
fn only_an_installed_copy_is_judged() {
    let folder = tempfile::tempdir().unwrap();
    install(folder.path(), false);
    std::fs::remove_file(folder.path().join(HELPERS[1])).unwrap();
    let exe = folder.path().join("vibe-work.exe");
    assert_eq!(
        damaged_helper(&exe, None),
        None,
        "a build run from elsewhere"
    );
    assert_eq!(
        damaged_helper(&exe, Some(folder.path())),
        Some(HELPERS[1]),
        "the installer's default folder, before its uninstaller was written"
    );
}

/// PREDICT: fails while only the helpers are checked: a copy in the installer's default folder whose uninstaller is
/// missing (the installer stopped before writing it) started with no notice, and the person could not uninstall it.
#[test]
fn a_default_folder_copy_without_its_uninstaller_is_reported() {
    let folder = tempfile::tempdir().unwrap();
    install(folder.path(), false);
    let exe = folder.path().join("vibe-work.exe");
    assert_eq!(
        damaged_helper(&exe, Some(folder.path())),
        Some("uninstall.exe")
    );
    assert_eq!(
        damaged_helper(&exe, None),
        None,
        "a build run from elsewhere"
    );
}

#[test]
fn installed_copy_is_the_uninstaller_folder_or_the_default_folder() {
    let folder = tempfile::tempdir().unwrap();
    install(folder.path(), false);
    let exe = folder.path().join("vibe-work.exe");
    assert!(!installed_copy(&exe, None), "a build run from elsewhere");
    assert!(installed_copy(&exe, Some(folder.path())));
    std::fs::write(folder.path().join("uninstall.exe"), b"u").unwrap();
    assert!(installed_copy(&exe, None));
}

/// A real signed or unsigned Windows program (this test binary) is complete; a copy cut short is not.
#[cfg(windows)]
#[test]
fn a_real_windows_program_cut_short_is_reported() {
    let folder = tempfile::tempdir().unwrap();
    install(folder.path(), true);
    let real = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    let helper = folder.path().join(HELPERS[0]);
    std::fs::write(&helper, &real).unwrap();
    let exe = folder.path().join("vibe-work.exe");
    assert_eq!(damaged_helper(&exe, None), None);
    std::fs::write(&helper, &real[..real.len() / 2]).unwrap();
    assert_eq!(damaged_helper(&exe, None), Some(HELPERS[0]));
}

// PREDICT (start probe): a helper complete on disk that Windows cannot load (it imports VCRUNTIME140.dll on a Windows
// without the Visual C++ runtime) passed `damaged_helper`, so nothing told the person or the agent; every tool call only
// said "exited with code 3221225781". The probe names the loader's refusal and ignores a program's own error exit.

#[test]
fn only_the_loader_refusals_count_as_cannot_start() {
    let missing_dll = i32::from_ne_bytes(0xC000_0135_u32.to_ne_bytes());
    assert_eq!(
        missing_dll, -1_073_741_515,
        "the code Windows reports as exit code 3221225781"
    );
    let text = loader_failure(missing_dll).unwrap();
    assert_eq!(
        text,
        "Windows could not load it: error 0xC0000135, a DLL it needs is missing"
    );
    for status in [0xC000_0139_u32, 0xC000_007B, 0xC000_0142] {
        assert!(loader_failure(i32::from_ne_bytes(status.to_ne_bytes())).is_some());
    }
    // A program's own exits (success, usage error, a crash after it started) are not start failures.
    for code in [
        0,
        1,
        2,
        101,
        i32::from_ne_bytes(0xC000_0005_u32.to_ne_bytes()),
    ] {
        assert_eq!(loader_failure(code), None, "{code}");
    }
}

#[test]
fn a_missing_or_unloadable_program_cannot_start() {
    let folder = tempfile::tempdir().unwrap();
    let missing = folder.path().join(HELPERS[0]);
    assert!(
        start_problem(&missing)
            .unwrap()
            .starts_with("it could not be started (")
    );
    // Not a program the system will start (see NOT_A_PROGRAM and not_starting).
    not_starting(&missing);
    assert!(start_problem(&missing).is_some());
}

#[test]
fn a_program_that_starts_and_exits_with_its_own_error_starts() {
    let program = if cfg!(windows) {
        // where.exe without arguments prints its usage error and exits 2.
        std::path::PathBuf::from(r"C:\Windows\System32\where.exe")
    } else {
        // `/usr/bin/false` exists on Linux and macOS; recent macOS has no `/bin/false`.
        std::path::PathBuf::from("/usr/bin/false")
    };
    assert_eq!(start_problem(&program), None);
}

#[cfg(unix)]
#[test]
fn a_program_still_running_after_the_wait_is_not_reported_and_is_stopped() {
    let folder = tempfile::tempdir().unwrap();
    let script = folder.path().join("slow");
    let marker = folder.path().join("finished");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nsleep 3\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        settled(|| start_problem_within(&script, std::time::Duration::from_millis(300))),
        None
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    std::thread::sleep(std::time::Duration::from_secs(4));
    assert!(!marker.exists(), "the probe stopped the program");
}

/// `probe()` again while a script this test just wrote is still "busy" for exec (Linux ETXTBSY, os error 26: another
/// test thread forked while the file was open for writing). Only the test's own fresh scripts can hit it.
fn settled<T: std::fmt::Debug>(probe: impl Fn() -> T) -> T {
    for _ in 0..50 {
        let result = probe();
        if !format!("{result:?}").contains("os error 26") {
            return result;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    probe()
}

/// A program that starts and exits with its own error at once (this system's), written to `path`.
fn starting(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    #[cfg(windows)]
    std::fs::copy(r"C:\Windows\System32\where.exe", path).unwrap();
    #[cfg(unix)]
    {
        std::fs::write(path, b"#!/bin/sh\nexit 2\n").unwrap();
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
    }
}

/// A file the system refuses to start (empty), written to `path`.
/// Bytes no system can load as a program (no executable header, no `#!` line).
const NOT_A_PROGRAM: &[u8] = b"\0\0\0\0 not a program\n";

/// A program file the system refuses to start. Unix: not executable (mode 0644), because macOS hands an executable
/// file it cannot load to the shell, which "starts" and exits 126 (and an empty one exits 0); Windows: bytes that are
/// not a program.
fn not_starting(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, NOT_A_PROGRAM).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
}

#[test]
fn the_first_shipped_program_that_cannot_start_is_named() {
    let folder = tempfile::tempdir().unwrap();
    install(folder.path(), true);
    let exe = folder.path().join("vibe-work.exe");
    for name in HELPERS {
        starting(&folder.path().join(name));
    }
    assert_eq!(
        settled(|| helper_start_problem(&exe, None)),
        None,
        "every program starts"
    );
    let chrome = folder.path().join("chrome-control/bin/chrome-control.exe");
    not_starting(&chrome);
    let (name, problem) = settled(|| helper_start_problem(&exe, None)).unwrap();
    assert_eq!(name, "chrome-control.exe");
    assert!(problem.starts_with("it could not be started"), "{problem}");
    not_starting(&folder.path().join(HELPERS[1]));
    assert_eq!(helper_start_problem(&exe, None).unwrap().0, HELPERS[1]);
    let all: Vec<_> = start_problems(&exe, None)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        all,
        [HELPERS[1], "chrome-control.exe"],
        "each program is reported, in order"
    );
    std::fs::remove_file(folder.path().join("uninstall.exe")).unwrap();
    assert_eq!(
        helper_start_problem(&exe, None),
        None,
        "not an installed copy"
    );
}
