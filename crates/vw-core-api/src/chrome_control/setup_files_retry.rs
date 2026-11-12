//! Bounded replacement retries for setup files held by Windows scanners.
//! Called by `setup_files` after writing a temporary file; every attempt rechecks the destination.
use std::{io, time::Duration};

pub(super) fn replace(
    windows: bool,
    mut unchanged: impl FnMut() -> io::Result<()>,
    mut rename: impl FnMut() -> io::Result<()>,
    mut sleep: impl FnMut(Duration),
) -> io::Result<()> {
    let mut pauses = [25, 50, 100, 200, 400, 800, 1600].into_iter();
    loop {
        unchanged()?;
        let error = match rename() {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        // Windows may report either access denied or a sharing violation for a scanner's handle.
        let busy = windows
            && (error.kind() == io::ErrorKind::PermissionDenied
                || matches!(error.raw_os_error(), Some(5 | 32)));
        if busy && let Some(ms) = pauses.next() {
            sleep(Duration::from_millis(ms));
            continue;
        }
        return Err(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn retries_busy_replace_until_the_scanner_releases_it() {
        for code in [5, 32] {
            let mut attempts = 0;
            let mut checked = 0;
            let mut pauses = Vec::new();
            replace(
                true,
                || {
                    checked += 1;
                    Ok(())
                },
                || {
                    attempts += 1;
                    if attempts <= 7 {
                        Err(io::Error::from_raw_os_error(code))
                    } else {
                        Ok(())
                    }
                },
                |pause| pauses.push(pause.as_millis()),
            )
            .unwrap();
            assert_eq!(attempts, 8);
            assert_eq!(checked, attempts);
            assert_eq!(pauses, [25, 50, 100, 200, 400, 800, 1600]);
        }
    }

    #[test]
    fn busy_retry_is_bounded_and_other_errors_are_not_retried() {
        for (windows, kind, expected) in [
            (true, io::ErrorKind::PermissionDenied, 8),
            (false, io::ErrorKind::PermissionDenied, 1),
            (true, io::ErrorKind::InvalidInput, 1),
        ] {
            let mut attempts = 0;
            let error = replace(
                windows,
                || Ok(()),
                || {
                    attempts += 1;
                    Err(io::Error::from(kind))
                },
                |_| {},
            )
            .unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(attempts, expected);
        }
    }

    #[test]
    fn a_concurrent_edit_stops_the_retry_before_another_rename() {
        let attempts = Cell::new(0);
        let error = replace(
            true,
            || {
                if attempts.get() == 0 {
                    Ok(())
                } else {
                    Err(io::Error::from(io::ErrorKind::InvalidInput))
                }
            },
            || {
                attempts.set(attempts.get() + 1);
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            },
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(attempts.get(), 1);
    }
}
