//! Atomic replacement regression cases: real collisions, failed writes, metadata and link handling.
//! Uses isolated sibling files; Windows cases require a native Windows runner, without elevation.

#[cfg(unix)]
use super::replace_file;
use super::{MetadataPolicy, replace_with};
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
