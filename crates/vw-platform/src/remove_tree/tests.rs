//! Deterministic ownership-boundary regressions for cleanup's root acquisition.
//! Uses a synchronous check seam and real links; called by vw-platform's unit tests.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) {
    let status = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .status()
        .unwrap();
    assert!(status.success(), "junction creation failed");
}

#[test]
fn root_swap_after_check_keeps_outside_probe() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let moved = temp.path().join("moved");
    let outside = temp.path().join("outside");
    fs::create_dir_all(root.join("probe")).unwrap();
    fs::create_dir_all(outside.join("probe")).unwrap();
    fs::write(outside.join("probe/nonce"), b"kept").unwrap();

    let result = super::remove_after_root_check(&root, &root.join("probe"), || {
        fs::rename(&root, &moved).unwrap();
        link_dir(&outside, &root);
    });

    assert!(
        outside.join("probe/nonce").exists(),
        "cleanup followed the replacement root and removed the outside nonce"
    );
    assert!(result.is_ok(), "cleanup lost its acquired root: {result:?}");
    assert!(!moved.join("probe").exists());
}

#[test]
fn refuses_linked_root_with_trailing_separator_or_dot() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let outside = temp.path().join("outside");
    fs::create_dir_all(outside.join("probe")).unwrap();
    fs::write(outside.join("probe/nonce"), b"kept").unwrap();
    link_dir(&outside, &root);
    for spelling in [
        root.clone(),
        root.join("."),
        std::path::PathBuf::from(format!("{}/", root.display())),
    ] {
        assert!(super::remove_owned_tree(&spelling, &spelling.join("probe")).is_err());
        assert_eq!(fs::read(outside.join("probe/nonce")).unwrap(), b"kept");
    }
}

#[test]
fn missing_root_or_descendant_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    super::remove_owned_tree(&missing, &missing.join("probe")).unwrap();
    super::remove_owned_tree(temp.path(), &missing.join("probe")).unwrap();
    fs::write(&missing, b"regular file").unwrap();
    assert!(super::remove_owned_tree(&missing, &missing.join("probe")).is_err());
    assert_eq!(fs::read(missing).unwrap(), b"regular file");
}
