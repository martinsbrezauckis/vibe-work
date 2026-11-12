//! Portable admission and coordinate plan for view-local macOS wheel delivery.
//! Called by the `AppKit` adapter; Linux tests cover bounds, axis signs and window-local coordinates.

pub(super) fn deltas(
    options: (u8, u8, u8, Option<(i32, i32)>),
) -> Result<Option<(i32, i32)>, String> {
    let Some((x, y)) = options.3 else {
        return Ok(None);
    };
    if (options.0, options.1, options.2) != (0, 0, 0)
        || !(-20_000..=20_000).contains(&x)
        || !(-20_000..=20_000).contains(&y)
        || (x == 0 && y == 0)
    {
        return Err(refusal("The native wheel options are invalid"));
    }
    // Quartz orders vertical before horizontal; positive Quartz deltas move content down/right.
    Ok(Some((-y, -x)))
}

/// A windowless `NSEvent` flips Quartz coordinates about the first screen. Compensate so its
/// `locationInWindow` is the already hit-tested window point; it is never posted to the screen.
pub(super) fn location(point: (f64, f64), first_screen_height: f64) -> Result<(f64, f64), String> {
    if ![point.0, point.1, first_screen_height]
        .into_iter()
        .all(f64::is_finite)
        || first_screen_height <= 0.0
    {
        return Err(refusal("The native wheel has invalid screen geometry"));
    }
    Ok((point.0, first_screen_height - point.1))
}

pub(super) fn refusal(reason: &str) -> String {
    format!("{reason}. Read the page again and use scroll with the intended pane ref.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_refusals_keep_a_reason_and_next_step() {
        for reason in [
            deltas((0, 0, 0, Some((0, 0)))).unwrap_err(),
            location((0.0, 0.0), 0.0).unwrap_err(),
        ] {
            assert!(
                reason.contains("wheel")
                    && reason.contains("Read the page")
                    && reason.contains("scroll"),
                "{reason}"
            );
        }
    }

    #[test]
    fn wheel_plan_maps_both_axes_without_a_button_sequence() {
        assert_eq!(deltas((0, 0, 0, Some((70, 120)))), Ok(Some((-120, -70))));
        assert_eq!(deltas((0, 0, 0, Some((-70, -120)))), Ok(Some((120, 70))));
        assert_eq!(deltas((0, 1, 0, None)), Ok(None));
    }

    #[test]
    fn wheel_plan_refuses_unbounded_or_mixed_input() {
        for options in [
            (0, 0, 0, Some((0, 0))),
            (0, 0, 0, Some((20_001, 1))),
            (0, 0, 0, Some((1, i32::MIN))),
            (1, 0, 0, Some((0, 1))),
            (0, 1, 0, Some((0, 1))),
            (0, 0, 8, Some((0, 1))),
        ] {
            assert!(deltas(options).is_err());
        }
    }

    #[test]
    fn wheel_location_stays_in_the_checked_window_coordinates() {
        assert_eq!(location((120.0, 90.0), 1080.0), Ok((120.0, 990.0)));
        assert_eq!(location((120.0, 900.0), 800.0), Ok((120.0, -100.0)));
        assert!(location((f64::NAN, 1.0), 1080.0).is_err());
        assert!(location((1.0, 1.0), 0.0).is_err());
    }
}
