//! The installer copies a Windows update leaves in the temp folder are removed once that version (or a later one)
//! runs: `vw_host::update_leftovers::remove_installer_copies`. The updater writes the downloaded installer to
//! `<temp>/<app>-<version>-updater-<random>/<app>-<version>-installer.exe` and has to keep it (the installer runs
//! after the app exits), so the next start of the app removes it.
//!
//! PREDICT (written before the first run; skipped is not passed):
//! - a folder for the running version and one for an older version go; one for a newer version stays (an install of
//!   it may be about to start or may have failed and is retried by the updater itself);
//! - a folder of another app, a folder with an unexpected file or a sub-folder, a plain file with the folder name,
//!   and a folder name with an unreadable version stay untouched;
//! - a link (symlink) named like a leftover is never followed or removed, and the folder it points to keeps its file;
//! - a missing temp folder is no error (nothing removed).

use std::fs;
use std::path::{Path, PathBuf};

use vw_host::update_leftovers::remove_installer_copies;

const APP: &str = "Vibe Work";

fn temp(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("vw-update-leftovers-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A folder exactly as the updater plugin makes it, with its one installer file.
fn leftover(temp: &Path, version: &str, suffix: &str, ext: &str) -> PathBuf {
    let dir = temp.join(format!("{APP}-{version}-updater-{suffix}"));
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join(format!("{APP}-{version}-installer{ext}")),
        b"MZ installer",
    )
    .unwrap();
    dir
}

#[test]
fn copies_for_the_running_and_older_versions_go_newer_stay() {
    let t = temp("versions");
    let running = leftover(&t, "0.1.2", "Ab12Cd", ".exe");
    let older = leftover(&t, "0.1.1", "VcOXUh", ".exe");
    let older_msi = leftover(&t, "0.1.0", "mdc5PX", ".msi");
    let newer = leftover(&t, "0.1.3", "Zz99Yy", ".exe");
    let mut removed = remove_installer_copies(&t, APP, "0.1.2");
    removed.sort();
    let mut expected = vec![running.clone(), older.clone(), older_msi.clone()];
    expected.sort();
    assert_eq!(removed, expected);
    assert!(!running.exists() && !older.exists() && !older_msi.exists());
    assert!(newer.join(format!("{APP}-0.1.3-installer.exe")).is_file());
    fs::remove_dir_all(&t).unwrap();
}

#[test]
fn anything_unexpected_is_left_alone() {
    let t = temp("unexpected");
    // Another app's updater folder.
    let other = t.join("Other App-0.1.0-updater-Ab12Cd");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("Other App-0.1.0-installer.exe"), b"x").unwrap();
    // Our name, but an extra file next to the installer.
    let extra = leftover(&t, "0.1.1", "Ex7raF", ".exe");
    fs::write(extra.join("notes.txt"), b"keep").unwrap();
    // Our name, but a sub-folder inside.
    let nested = leftover(&t, "0.1.1", "Ne5ted", ".exe");
    fs::create_dir(nested.join("sub")).unwrap();
    // An installer named for another version than its folder.
    let mismatch = t.join(format!("{APP}-0.1.1-updater-Mi5mat"));
    fs::create_dir(&mismatch).unwrap();
    fs::write(mismatch.join(format!("{APP}-0.1.0-installer.exe")), b"x").unwrap();
    // A plain file with a leftover's name, and a folder whose version does not parse.
    let file = t.join(format!("{APP}-0.1.1-updater-F1le00"));
    fs::write(&file, b"x").unwrap();
    let bad = t.join(format!("{APP}-latest-updater-Bad000"));
    fs::create_dir(&bad).unwrap();
    // No random suffix at all.
    let bare = t.join(format!("{APP}-0.1.1-updater-"));
    fs::create_dir(&bare).unwrap();
    assert_eq!(
        remove_installer_copies(&t, APP, "0.1.2"),
        Vec::<PathBuf>::new()
    );
    for kept in [&other, &extra, &nested, &mismatch, &file, &bad, &bare] {
        assert!(kept.exists(), "{} was removed", kept.display());
    }
    assert!(extra.join(format!("{APP}-0.1.1-installer.exe")).is_file());
    fs::remove_dir_all(&t).unwrap();
}

#[cfg(unix)]
#[test]
fn a_link_named_like_a_leftover_is_not_followed() {
    let t = temp("link");
    let target = t.join("elsewhere");
    fs::create_dir(&target).unwrap();
    fs::write(target.join(format!("{APP}-0.1.1-installer.exe")), b"x").unwrap();
    let link = t.join(format!("{APP}-0.1.1-updater-L1nk00"));
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(
        remove_installer_copies(&t, APP, "0.1.2"),
        Vec::<PathBuf>::new()
    );
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(target.join(format!("{APP}-0.1.1-installer.exe")).is_file());
    fs::remove_dir_all(&t).unwrap();
}

#[test]
fn a_missing_temp_folder_removes_nothing() {
    let t = temp("missing");
    fs::remove_dir_all(&t).unwrap();
    assert_eq!(
        remove_installer_copies(&t, APP, "0.1.2"),
        Vec::<PathBuf>::new()
    );
}
