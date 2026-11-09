//! Config replacement under a reader's Windows sharing lock, with bounded retry and stale-write checks.
//! Injects filesystem errors for portable checks; Windows also exercises a real open handle.
use super::replace_with;
use std::path::Path;
use vw_platform::PrivateFileError;

const BEFORE: &str = "active_model = 'before'\n";
const AFTER: &str = "active_model = 'after'\n";

fn busy(path: &Path, code: i32) -> PrivateFileError {
    PrivateFileError::Io {
        action: "replace file",
        path: path.display().to_string(),
        source: std::io::Error::from_raw_os_error(code),
    }
}

#[tokio::test(start_paused = true)]
async fn a_transient_windows_reader_does_not_fail_the_model_check() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, BEFORE).unwrap();
    let mut attempts = 0;
    let changed = replace_with(&path, BEFORE, AFTER, true, |file, bytes| {
        attempts += 1;
        if attempts == 1 {
            Err(busy(file, 32))
        } else {
            vw_platform::replace_private(file, bytes)
        }
    })
    .await
    .unwrap();
    assert!(changed);
    assert_eq!(attempts, 2);
    assert_eq!(std::fs::read_to_string(path).unwrap(), AFTER);
}

#[tokio::test(start_paused = true)]
async fn a_changed_config_is_preserved_when_a_reader_releases_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, BEFORE).unwrap();
    let mut attempts = 0;
    let changed = replace_with(&path, BEFORE, AFTER, true, |file, _| {
        attempts += 1;
        std::fs::write(file, "active_model = 'theirs'\n").unwrap();
        Err(busy(file, 32))
    })
    .await
    .unwrap();
    assert!(!changed);
    assert_eq!(attempts, 1);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "active_model = 'theirs'\n"
    );
}

#[tokio::test(start_paused = true)]
async fn persistent_locks_are_bounded_and_other_errors_are_not_retried() {
    for (windows, code, expected) in [(true, 5, 5), (true, 32, 5), (true, 2, 1), (false, 32, 1)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, BEFORE).unwrap();
        let mut attempts = 0;
        let start = tokio::time::Instant::now();
        assert!(
            replace_with(&path, BEFORE, AFTER, windows, |file, _| {
                attempts += 1;
                Err(busy(file, code))
            })
            .await
            .is_err()
        );
        assert_eq!(attempts, expected);
        assert!(start.elapsed() <= std::time::Duration::from_millis(375));
        assert_eq!(std::fs::read_to_string(path).unwrap(), BEFORE);
    }
}

#[cfg(windows)]
#[tokio::test]
async fn a_real_windows_reader_can_finish_before_the_model_config_is_replaced() {
    use std::os::windows::fs::OpenOptionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, BEFORE).unwrap();
    let reader = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(&path)
        .unwrap();
    let error = vw_platform::replace_private(&path, AFTER.as_bytes()).unwrap_err();
    assert!(
        matches!(error, PrivateFileError::Io { source, .. } if matches!(source.raw_os_error(), Some(5 | 32)))
    );
    let release = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        drop(reader);
    });
    assert!(super::replace(&path, BEFORE, AFTER).await.unwrap());
    release.await.unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), AFTER);
}
