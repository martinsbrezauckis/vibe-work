//! Independent Linux capability probe for tests that execute sealed helpers.
//! Uses a harmless OS executable or script, never the product launcher; only a policy denial skips a test.

use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;

pub fn available(test: &str, script: bool) -> bool {
    use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
    let flags = MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING;
    let fd = match memfd_create(c"test-execution-policy", flags | MemfdFlags::EXEC) {
        Err(rustix::io::Errno::INVAL) => memfd_create(c"test-execution-policy", flags),
        result => result,
    }
    .expect("create independent execution probe");
    let mut file = std::fs::File::from(fd);
    if script {
        file.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
    } else {
        assert!(
            std::process::Command::new("/bin/true")
                .status()
                .unwrap()
                .success()
        );
        std::io::copy(&mut std::fs::File::open("/bin/true").unwrap(), &mut file).unwrap();
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o700))
        .unwrap();
    fcntl_add_seals(
        &file,
        SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL,
    )
    .unwrap();
    let process = if script {
        std::process::id().to_string()
    } else {
        "self".into()
    };
    match std::process::Command::new(format!("/proc/{process}/fd/{}", file.as_raw_fd())).status() {
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            // Bypass harness capture so a normal cargo test run cannot hide this missing coverage.
            writeln!(std::io::stderr().lock(),
                "SKIPPED: {test}: sandbox denies independent sealed-memfd execution ({error}); NOT RUN (not a pass)").unwrap();
            // A gate host (VW_REQUIRE_ALL=1, set by check-all.sh and merge_check.sh) must be able to run it: fail there.
            assert!(
                !std::env::var("VW_REQUIRE_ALL").is_ok_and(|value| value == "1"),
                "required test prerequisite is unavailable: {test}: sealed-memfd execution is denied ({error})"
            );
            false
        }
        result => {
            assert!(result.expect("execute independent policy probe").success());
            true
        }
    }
}
