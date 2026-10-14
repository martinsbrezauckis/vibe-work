//! The `pub_date` of the manifest: an RFC 3339 UTC time in the one form this tool writes and accepts,
//! `YYYY-MM-DDTHH:MM:SSZ` (the updater parses any RFC 3339; one strict form keeps the check simple).
//!
//! Depends on: std only. Called by: the crate root (`check`), `files.rs` (default date).

/// `value` is `YYYY-MM-DDTHH:MM:SSZ` with plausible ranges (an RFC 3339 UTC time, the form [`now_rfc3339`] writes).
#[must_use]
pub fn valid_pub_date(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return false;
    }
    let num = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = &value[range];
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    matches!(
        (
            num(0..4),
            num(5..7),
            num(8..10),
            num(11..13),
            num(14..16),
            num(17..19)
        ),
        (
            Some(1970..=9999),
            Some(1..=12),
            Some(1..=31),
            Some(0..=23),
            Some(0..=59),
            Some(0..=59)
        )
    )
}

/// The current time as `YYYY-MM-DDTHH:MM:SSZ`.
#[must_use]
pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rest = secs % 86_400;
    // Civil date from days since 1970-01-01 (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}
