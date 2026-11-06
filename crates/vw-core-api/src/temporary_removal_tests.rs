//! Temporary scratch cleanup under short and persistent reader holds; called by Rust's test harness.
//! Uses real Windows directory handles and injected EBUSY elsewhere, without changing process current folders.
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use super::{TemporarySessions, remove_scratch_in_with};

fn fixture() -> (tempfile::TempDir, TemporarySessions) {
    let root = std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, Into::into);
    std::fs::create_dir_all(&root).unwrap();
    let data = tempfile::Builder::new()
        .prefix("temporary-removal-")
        .tempdir_in(root)
        .unwrap();
    let record = TemporarySessions::load(Some(data.path().join("sessions")));
    (data, record)
}

fn hold_folder(folder: &Path) -> Arc<File> {
    #[cfg(windows)]
    let handle = {
        use std::os::windows::fs::OpenOptionsExt as _;
        // A reader that shares reads/writes, but not deletion, reproduces a Windows sharing violation.
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x3)
            .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS: open the directory itself.
            .open(folder)
            .unwrap()
    };
    #[cfg(not(windows))]
    let handle = File::open(folder).unwrap();
    Arc::new(handle)
}

fn remove_held(folder: &Path, holder: &Weak<File>) -> io::Result<()> {
    #[cfg(not(windows))]
    if holder.upgrade().is_some() {
        return Err(io::ErrorKind::ResourceBusy.into());
    }
    let _ = holder;
    std::fs::remove_dir_all(folder)
}

fn assert_busy(error: &io::Error) {
    #[cfg(windows)]
    assert_eq!(error.raw_os_error(), Some(32), "{error}");
    #[cfg(not(windows))]
    assert_eq!(error.kind(), io::ErrorKind::ResourceBusy, "{error}");
}

#[test]
fn a_short_reader_hold_is_retried_until_the_folder_is_removed() {
    let (_data, record) = fixture();
    let folder = record.new_scratch().unwrap();
    let holder = hold_folder(&folder);
    let weak = Arc::downgrade(&holder);
    assert_busy(&remove_held(&folder, &weak).unwrap_err());
    let (first_attempt, release_after_attempt) = std::sync::mpsc::channel();
    let release = std::thread::spawn(move || {
        release_after_attempt.recv().unwrap();
        std::thread::sleep(Duration::from_millis(250));
        drop(holder);
    });
    let started = Instant::now();
    let mut attempts = 0;
    remove_scratch_in_with(folder.parent().unwrap(), &folder, |path| {
        attempts += 1;
        let result = remove_held(path, &weak);
        if attempts == 1 {
            first_attempt.send(()).unwrap();
        }
        result
    });
    let elapsed = started.elapsed();
    // Join before asserting so even a regression releases its real Windows handle normally.
    release.join().unwrap();
    assert!(!folder.exists(), "short reader hold left the folder behind");
    assert!(attempts > 1, "the initial sharing violation was retried");
    assert!(elapsed < Duration::from_secs(3), "elapsed: {elapsed:?}");
}

#[test]
fn a_persistent_reader_hold_stops_at_the_budget_and_a_later_sweep_removes_it() {
    let (_data, record) = fixture();
    let folder = record.new_scratch().unwrap();
    let holder = hold_folder(&folder);
    let weak = Arc::downgrade(&holder);
    assert_busy(&remove_held(&folder, &weak).unwrap_err());
    let started = Instant::now();
    let mut attempts = 0;
    remove_scratch_in_with(folder.parent().unwrap(), &folder, |path| {
        attempts += 1;
        remove_held(path, &weak)
    });
    let elapsed = started.elapsed();
    let left_for_sweep = folder.exists();
    // Keep the holder alive through the entire first call, independently of scheduler delays.
    drop(holder);
    record.sweep_scratch();
    assert!(
        left_for_sweep,
        "an in-use folder must remain for the fallback"
    );
    assert!(!folder.exists(), "the existing later sweep removes it");
    assert!(
        elapsed >= Duration::from_millis(1900) && elapsed < Duration::from_secs(3),
        "expected about 2 seconds of bounded retry, got {elapsed:?} ({attempts} attempts)"
    );
    assert!(attempts > 1 && attempts < 100, "attempts: {attempts}");
}

#[test]
fn unrelated_removal_errors_return_without_backoff() {
    let (_data, record) = fixture();
    let folder = record.new_scratch().unwrap();
    let mut attempts = 0;
    let started = Instant::now();
    remove_scratch_in_with(folder.parent().unwrap(), &folder, |_| {
        attempts += 1;
        Err(io::ErrorKind::PermissionDenied.into())
    });
    assert_eq!(attempts, 1);
    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(folder.exists());
}

#[cfg(unix)]
#[test]
fn a_scratch_root_replaced_by_a_link_during_backoff_is_left_alone() {
    let (data, record) = fixture();
    let folder = record.new_scratch().unwrap();
    let root = folder.parent().unwrap();
    let outside = data.path().join("outside");
    let foreign_folder = outside.join(folder.file_name().unwrap());
    std::fs::create_dir_all(&foreign_folder).unwrap();
    std::fs::write(foreign_folder.join("keep.txt"), "keep").unwrap();
    let mut attempts = 0;
    remove_scratch_in_with(root, &folder, |_| {
        attempts += 1;
        if attempts == 1 {
            std::fs::rename(root, data.path().join("original-root")).unwrap();
            std::os::unix::fs::symlink(&outside, root).unwrap();
        }
        Err(io::ErrorKind::ResourceBusy.into())
    });
    assert_eq!(attempts, 1, "each retry must recheck the scratch root");
    assert!(foreign_folder.join("keep.txt").is_file());
}
