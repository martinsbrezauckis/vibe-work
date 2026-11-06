//! Atomic replacement regression cases: real collisions, failed writes, metadata and link handling.
//! Uses isolated sibling files; Windows cases require a native Windows runner, without elevation.

use super::{MetadataPolicy, replace_file, replace_with};
use std::fs;
use std::io::{self, Write};

#[test]
fn collision_retry_never_changes_the_existing_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("document.txt");
    let collision = dir.path().join(".document.txt.vw-0000000000000001.tmp");
    fs::write(&collision, "unrelated stale file").unwrap();
    let mut suffix = 0;
    replace_with(
        &path,
        MetadataPolicy::NewFile,
        || {
            suffix += 1;
            Ok(suffix)
        },
        |file| file.write_all(b"whole"),
    )
    .unwrap();
    assert_eq!(suffix, 2);
    assert_eq!(fs::read(&path).unwrap(), b"whole");
    assert_eq!(
        fs::read_to_string(collision).unwrap(),
        "unrelated stale file"
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn exhausted_collisions_leave_the_destination_and_collision_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("document.txt");
    fs::write(&path, "before").unwrap();
    let collision = dir.path().join(".document.txt.vw-0000000000000001.tmp");
    fs::write(&collision, "not ours").unwrap();
    let error = replace_with(
        &path,
        MetadataPolicy::NewFile,
        || Ok(1),
        |_| panic!("must not write"),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&path).unwrap(), b"before");
    assert_eq!(fs::read(collision).unwrap(), b"not ours");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn partial_write_failure_keeps_original_and_removes_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("document.txt");
    fs::write(&path, "before").unwrap();
    let error = replace_with(
        &path,
        MetadataPolicy::PreservePermissions,
        || Ok(7),
        |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected write failure"))
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "injected write failure");
    assert_eq!(fs::read(&path).unwrap(), b"before");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn rename_failure_removes_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing-folder");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("untouched"), "before").unwrap();
    assert!(replace_file(&path, b"after", MetadataPolicy::NewFile).is_err());
    assert_eq!(fs::read(path.join("untouched")).unwrap(), b"before");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn either_policy_can_create_and_replace_whole_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file with spaces.txt");
    for policy in [MetadataPolicy::NewFile, MetadataPolicy::PreservePermissions] {
        replace_file(&path, b"new", policy).unwrap();
        replace_file(&path, b"replacement", policy).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        fs::remove_file(&path).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn permission_policy_retains_modes_including_read_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("document.txt");
    for mode in [0o640, 0o400] {
        fs::write(&path, "before").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        replace_file(&path, b"after", MetadataPolicy::PreservePermissions).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"after");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            mode
        );
        fs::remove_file(&path).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn colliding_temp_symlink_and_final_symlink_never_receive_writes() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    fs::write(&outside, "untouched").unwrap();
    let target = dir.path().join("export.md");
    let collision = dir.path().join(".export.md.vw-0000000000000001.tmp");
    std::os::unix::fs::symlink(&outside, &collision).unwrap();
    std::os::unix::fs::symlink(&outside, &target).unwrap();
    assert!(replace_file(&target, b"refused", MetadataPolicy::PreservePermissions).is_err());
    let mut suffix = 0;
    replace_with(
        &target,
        MetadataPolicy::NewFile,
        || {
            suffix += 1;
            Ok(suffix)
        },
        |file| file.write_all(b"export"),
    )
    .unwrap();
    assert_eq!(suffix, 2);
    assert_eq!(fs::read(&outside).unwrap(), b"untouched");
    assert!(
        fs::symlink_metadata(collision)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        !fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(target).unwrap(), b"export");
}

#[cfg(windows)]
fn powershell(path: &std::path::Path, script: &str) -> String {
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("VW_ATOMIC_TEST_PATH", path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[cfg(windows)]
#[test]
fn windows_replacement_preserves_shared_dacl_and_inheritance_protection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared document.txt");
    fs::write(&path, "before").unwrap();
    // Use fixed SIDs so this works on localized Windows without resolving account names.
    powershell(
        &path,
        r"
$ErrorActionPreference = 'Stop'
$sid = '*' + [Security.Principal.WindowsIdentity]::GetCurrent().User.Value + ':(F)'
& icacls.exe $env:VW_ATOMIC_TEST_PATH '/inheritance:r' '/grant:r' $sid '*S-1-5-32-545:(RX)' | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'could not set the test DACL' }
",
    );
    let snapshot = r"(Get-Acl -LiteralPath $env:VW_ATOMIC_TEST_PATH).GetSecurityDescriptorSddlForm([Security.AccessControl.AccessControlSections]::Access)";
    for protected in [true, false] {
        if !protected {
            powershell(
                &path,
                r"$ErrorActionPreference = 'Stop'; & icacls.exe $env:VW_ATOMIC_TEST_PATH '/inheritance:e' | Out-Null; if ($LASTEXITCODE -ne 0) { throw 'could not enable test inheritance' }",
            );
        }
        let before = powershell(&path, snapshot);
        replace_file(&path, b"after", MetadataPolicy::PreservePermissions).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"after");
        assert_eq!(
            powershell(&path, snapshot),
            before,
            "DACL or protection changed"
        );
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn windows_read_only_and_sharing_failures_keep_original_and_clean_temporary_file() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("document.txt");
    fs::write(&path, "before").unwrap();
    let original_permissions = fs::metadata(&path).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    assert!(replace_file(&path, b"after", MetadataPolicy::PreservePermissions).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"before");
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    fs::set_permissions(&path, original_permissions).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    assert!(replace_file(&path, b"after", MetadataPolicy::PreservePermissions).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"before");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    drop(held);
    replace_file(&path, b"after", MetadataPolicy::PreservePermissions).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"after");
}
