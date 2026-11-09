//! Private worker files stay owned until process reaping, including timeout and cancellation.
//! The supervisor holds this directory; Windows may briefly retain an executable's image mapping after exit.
use std::{
    io,
    time::{Duration, Instant},
};

pub(crate) struct Directory(pub(crate) tempfile::TempDir);

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = remove(Duration::from_secs(2), || {
            std::fs::remove_dir_all(self.0.path())
        });
    }
}

fn remove(bound: Duration, mut operation: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let started = Instant::now();
    loop {
        match operation() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                let remaining = bound.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(error);
                }
                std::thread::sleep(remaining.min(Duration::from_millis(25)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_waits_for_a_terminating_workers_executable_mapping() {
        let directory = tempfile::tempdir().unwrap();
        let helper = directory.path().join("vw-video-helper.exe");
        std::fs::write(&helper, b"copied helper").unwrap();
        let mut attempts = 0;
        let result = remove(Duration::from_secs(1), || {
            attempts += 1;
            if attempts < 3 {
                // Windows refuses removal until the terminated worker's image mapping is released.
                Err(io::ErrorKind::PermissionDenied.into())
            } else {
                std::fs::remove_dir_all(directory.path())
            }
        });
        assert!(result.is_ok(), "{result:?}");
        assert!(!helper.exists());
        assert_eq!(attempts, 3);
    }

    #[test]
    fn permanent_cleanup_failure_is_bounded() {
        let result = remove(Duration::ZERO, || {
            Err(io::ErrorKind::PermissionDenied.into())
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
}
