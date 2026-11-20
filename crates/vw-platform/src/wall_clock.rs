//! Wall-clock milliseconds since the Unix epoch in the two forms the workspace stores: unsigned (`u64`: records,
//! leases, comments) and signed (`i64`: vault, key and rate-limit timestamps). One reading of the clock instead of a
//! copy per crate; the conversion bounds are those every copy had.
//!
//! Depends on: std only. Called by: crates that stamp records with the current time.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch: 0 when the clock is before it, `u64::MAX` past that range.
#[must_use]
pub fn unix_ms() -> u64 {
    unix_ms_at(SystemTime::now())
}

/// Milliseconds since the Unix epoch as a signed timestamp: 0 when the clock is before it or past `i64::MAX`.
#[must_use]
pub fn unix_ms_signed() -> i64 {
    unix_ms_signed_at(SystemTime::now())
}

fn unix_ms_at(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn unix_ms_signed_at(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{unix_ms, unix_ms_at, unix_ms_signed, unix_ms_signed_at};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn reads_whole_milliseconds_since_the_epoch() {
        let at = UNIX_EPOCH + Duration::from_micros(1_500_999);
        assert_eq!(unix_ms_at(at), 1_500);
        assert_eq!(unix_ms_signed_at(at), 1_500);
        assert_eq!(unix_ms_at(UNIX_EPOCH), 0);
    }

    #[test]
    fn a_clock_before_the_epoch_reads_zero() {
        let Some(before) = UNIX_EPOCH.checked_sub(Duration::from_secs(1)) else {
            return; // A platform whose clock cannot go below the epoch has nothing to check.
        };
        assert_eq!(unix_ms_at(before), 0);
        assert_eq!(unix_ms_signed_at(before), 0);
    }

    #[test]
    fn a_signed_reading_past_its_range_is_zero_and_the_unsigned_one_keeps_counting() {
        let past =
            Duration::from_millis(u64::try_from(i64::MAX).unwrap()) + Duration::from_millis(1);
        let Some(at) = UNIX_EPOCH.checked_add(past) else {
            return; // This platform's clock cannot represent that time.
        };
        assert_eq!(unix_ms_signed_at(at), 0);
        assert_eq!(unix_ms_at(at), u64::try_from(i64::MAX).unwrap() + 1);
    }

    #[test]
    fn now_lies_between_two_readings_of_the_system_clock() {
        let before = unix_ms_at(SystemTime::now());
        let (unsigned, signed) = (unix_ms(), unix_ms_signed());
        let after = unix_ms_at(SystemTime::now());
        assert!((before..=after).contains(&unsigned));
        assert!((before..=after).contains(&u64::try_from(signed).unwrap()));
    }
}
