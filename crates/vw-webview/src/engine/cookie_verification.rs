//! Cookie mutation verification for the native adapter; runs on its blocking worker, never the UI thread.
//! A receipt describes the observed store, including a bounded wait for asynchronous native mutations.
use std::time::{Duration, Instant};

pub(super) const CHANGE_WAIT: Duration = Duration::from_secs(1);

pub(super) fn changed(
    bound: Duration,
    mut observe: impl FnMut() -> Result<bool, String>,
) -> Result<bool, String> {
    let started = Instant::now();
    loop {
        if observe()? {
            return Ok(true);
        }
        let remaining = bound.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Ok(false);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(25)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_delete_is_observed_before_reporting_the_receipt() {
        let mut reads = 0;
        let removed = changed(CHANGE_WAIT, || {
            reads += 1;
            // DeleteCookie accepted the request, but the store still returns the old cookie twice.
            Ok(reads >= 3)
        });
        assert_eq!(removed, Ok(true));
        assert_eq!(reads, 3);
    }

    #[test]
    fn retained_cookie_is_not_a_success_at_the_bound() {
        assert_eq!(changed(Duration::from_millis(2), || Ok(false)), Ok(false));
    }

    #[test]
    fn immediate_completion_needs_one_read() {
        let mut reads = 0;
        assert_eq!(
            changed(CHANGE_WAIT, || {
                reads += 1;
                Ok(true)
            }),
            Ok(true)
        );
        assert_eq!(reads, 1);
    }

    #[test]
    fn failed_read_is_not_a_success() {
        assert_eq!(
            changed(CHANGE_WAIT, || Err("store unavailable".into())),
            Err("store unavailable".into())
        );
    }
}
