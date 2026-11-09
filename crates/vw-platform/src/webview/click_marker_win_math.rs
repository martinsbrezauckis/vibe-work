//! Physical-pixel placement and diagnostic text for the Windows marker. No native calls.
//! Used by the popup renderer and its window-free regression tests; preserves the child renderer's rounding.

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Position {
    pub left: i32,
    pub top: i32,
    pub side: i32,
}

pub(super) fn position(
    screen_origin: (i32, i32),
    view_offset: (i32, i32),
    point: (f64, f64),
    dpi: u32,
) -> Position {
    let side = rounded(28.0 * f64::from(dpi) / 96.0);
    Position {
        left: screen_origin.0 + view_offset.0 + rounded(point.0) - side / 2,
        top: screen_origin.1 + view_offset.1 + rounded(point.1) - side / 2,
        side,
    }
}

// Viewport bounds and DPI are bounded by the native window; values here fit an i32.
#[allow(clippy::cast_possible_truncation)]
fn rounded(value: f64) -> i32 {
    value.round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_marker_screen_position_at_100_percent() {
        assert_eq!(
            position((800, 200), (41, 63), (36.0, 87.0), 96),
            Position {
                left: 863,
                top: 336,
                side: 28
            }
        );
    }

    #[test]
    fn click_marker_screen_position_at_150_percent_preserves_physical_rounding() {
        // The viewport point has already been mapped to physical WebView2 bounds: no second DPI multiply.
        assert_eq!(
            position((1200, 300), (41, 53), (34.4, 122.8), 144),
            Position {
                left: 1254,
                top: 455,
                side: 42
            }
        );
    }

    #[test]
    fn click_marker_after_scroll_uses_the_new_viewport_point() {
        assert_eq!(
            position((800, 200), (41, 63), (36.0, 211.0), 96),
            Position {
                left: 863,
                top: 460,
                side: 28
            }
        );
        assert_eq!(
            position((1200, 300), (41, 53), (34.4, 304.0), 144),
            Position {
                left: 1254,
                top: 636,
                side: 42
            }
        );
    }

    #[test]
    fn click_marker_screen_position_on_a_monitor_above_and_left() {
        assert_eq!(
            position((-1920, -1080), (41, 63), (36.0, 87.0), 96),
            Position {
                left: -1857,
                top: -944,
                side: 28
            }
        );
    }
}
