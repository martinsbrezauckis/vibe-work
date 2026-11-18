//! Local display dates and UTC capture timestamps.
//! Uses the process clock and the OS local offset for browser and secret receipts.

pub(crate) fn capture_timestamp() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:09}+00:00",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.nanosecond(),
    )
}

pub(crate) fn local_profile_date(at: Option<u64>, seconds: bool) -> Option<String> {
    let millis = i64::try_from(at?).ok()?;
    let instant =
        time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000).ok()?;
    let offset = time::UtcOffset::local_offset_at(instant).ok()?;
    let local = instant.to_offset(offset);
    let current =
        time::OffsetDateTime::now_utc().to_offset(time::UtcOffset::current_local_offset().ok()?);
    Some(profile_date_at(local, current, seconds))
}

fn profile_date_at(
    local: time::OffsetDateTime,
    current: time::OffsetDateTime,
    seconds: bool,
) -> String {
    let month = local.month().to_string();
    let month = &month[..3];
    let date = if local.year() == current.year() {
        format!("{} {month}", local.day())
    } else {
        format!("{} {month} {}", local.day(), local.year())
    };
    if seconds {
        format!(
            "{date} {:02}:{:02}:{:02}",
            local.hour(),
            local.minute(),
            local.second()
        )
    } else {
        format!("{date} {:02}:{:02}", local.hour(), local.minute())
    }
}

pub(crate) fn local_receipt_time() -> String {
    let local =
        time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    format!("{:02}:{:02}", local.hour(), local.minute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_timestamp_is_rfc3339_utc() {
        let value = capture_timestamp();
        assert!(value.ends_with("+00:00"), "{value}");
        assert!(
            time::OffsetDateTime::parse(&value, &time::format_description::well_known::Rfc3339)
                .is_ok(),
            "{value}"
        );
    }

    #[test]
    fn old_and_new_profile_dates_keep_year_and_seconds_rules() {
        let old = local_profile_date(Some(962_452_800_000), false).expect("older date");
        assert!(old.contains("2000"), "{old}");
        assert!(!old.contains(":00:"), "{old}");
        assert!(local_profile_date(None, false).is_none());
        assert!(local_profile_date(Some(u64::MAX), false).is_none());
    }

    #[test]
    fn local_receipt_time_has_hour_and_minute() {
        let value = local_receipt_time();
        assert_eq!(value.len(), 5, "{value}");
        assert_eq!(value.as_bytes()[2], b':');
    }

    #[test]
    fn profile_date_uses_local_calendar_year_and_optional_seconds() {
        let plus_two = time::UtcOffset::from_hms(2, 0, 0).unwrap();
        let utc = time::Date::from_calendar_date(2026, time::Month::December, 31)
            .unwrap()
            .with_hms(23, 30, 45)
            .unwrap()
            .assume_utc();
        let local = utc.to_offset(plus_two);
        let current = time::Date::from_calendar_date(2027, time::Month::June, 1)
            .unwrap()
            .midnight()
            .assume_offset(plus_two);
        assert_eq!(profile_date_at(local, current, false), "1 Jan 01:30");
        assert_eq!(profile_date_at(local, current, true), "1 Jan 01:30:45");
        let later = current.replace_year(2028).unwrap();
        assert_eq!(profile_date_at(local, later, true), "1 Jan 2027 01:30:45");
    }

    #[test]
    fn local_offset_is_available_with_another_live_thread() {
        let (send, receive) = std::sync::mpsc::channel::<()>();
        let other = std::thread::spawn(move || {
            let _ = receive.recv();
        });
        let now = time::OffsetDateTime::now_utc();
        let offset = time::UtcOffset::local_offset_at(now);
        send.send(()).unwrap();
        other.join().unwrap();
        assert!(
            offset.is_ok(),
            "the OS local offset must work in a threaded app: {offset:?}"
        );
    }
}
