//! Real filesystem regressions for Vibe-mode cleanup and its ownership boundary.
//! Depends on tempfile and vw-platform; runs without a window, as the current user.

#![allow(clippy::unwrap_used)]

use std::fs;
use vw_platform::remove_owned_tree;

#[path = "../../../tests/public/fixtures/readonly_package.rs"]
mod fixture;
use fixture::readonly_package;

#[test]
fn removes_vibe_modes() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("vibe-home");
    readonly_package(&home.join("logs/session/plugins/packages/ab/hash"));
    let result = remove_owned_tree(temp.path(), &home);
    assert!(result.is_ok(), "Vibe-mode tree was not removed: {result:?}");
    assert!(!home.exists());
}

#[test]
fn refuses_root_itself_and_parent_traversal() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("child")).unwrap();
    assert!(remove_owned_tree(temp.path(), temp.path()).is_err());
    assert!(temp.path().join("child").exists());
    let escape = temp.path().join("child/../child");
    assert!(remove_owned_tree(temp.path(), &escape).is_err());
    assert!(temp.path().join("child").exists());
}

#[cfg(unix)]
#[test]
fn refuses_linked_ancestor_and_never_chmods_a_link_target() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let kept = outside.path().join("kept");
    readonly_package(&kept);
    symlink(outside.path(), temp.path().join("escape")).unwrap();
    assert!(remove_owned_tree(temp.path(), &temp.path().join("escape/kept")).is_err());
    assert!(
        remove_owned_tree(
            &temp.path().join("escape"),
            &temp.path().join("escape/kept")
        )
        .is_err()
    );
    assert_eq!(
        fs::metadata(&kept).unwrap().permissions().mode() & 0o777,
        0o500
    );
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    symlink(&kept, home.join("linked-package")).unwrap();
    remove_owned_tree(temp.path(), &home).unwrap();
    assert_eq!(
        fs::metadata(&kept).unwrap().permissions().mode() & 0o777,
        0o500
    );
    assert!(kept.join("skills/example/SKILL.md").exists());
    remove_owned_tree(outside.path(), &kept).unwrap();
}

#[cfg(windows)]
#[test]
fn refuses_junction_ancestors_and_unlinks_internal_junctions() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let kept = outside.path().join("kept");
    readonly_package(&kept);
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let link = home.join("link");
    let status = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .status()
        .unwrap();
    assert!(status.success(), "junction creation failed");
    assert!(remove_owned_tree(temp.path(), &link.join("kept")).is_err());
    remove_owned_tree(temp.path(), &home).unwrap();
    assert!(kept.join("plugin.json").exists());
    assert!(
        fs::metadata(kept.join("plugin.json"))
            .unwrap()
            .permissions()
            .readonly()
    );
    remove_owned_tree(outside.path(), &kept).unwrap();
}

#[cfg(windows)]
#[test]
fn removes_readonly_hardlink_without_widening_outside_file() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("kept");
    let home = temp.path().join("home");
    fs::write(&outside, b"outside nonce").unwrap();
    fs::create_dir(&home).unwrap();
    fs::hard_link(&outside, home.join("link")).unwrap();
    let original = fs::metadata(&outside).unwrap().permissions();
    let mut perms = original.clone();
    perms.set_readonly(true);
    fs::set_permissions(&outside, perms).unwrap();

    let result = remove_owned_tree(temp.path(), &home);
    // Opening the outside name refreshes Windows' per-link attribute cache.
    let readonly = fs::File::open(&outside)
        .unwrap()
        .metadata()
        .unwrap()
        .permissions()
        .readonly();
    // Restore the fixture only after observing cleanup's effect on the shared file.
    fs::set_permissions(&outside, original).unwrap();
    assert!(result.is_ok(), "hard-link cleanup failed: {result:?}");
    assert!(!home.exists());
    assert_eq!(fs::read(&outside).unwrap(), b"outside nonce");
    assert!(readonly, "cleanup made the outside hard link writable");
}
