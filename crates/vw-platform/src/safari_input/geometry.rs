//! Portable native-window/viewport proof and CSS-to-screen mapping. macOS supplies only measured rectangles.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub(super) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub(super) struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(super) struct Viewport {
    pub width: f64,
    pub height: f64,
}
pub(super) fn map(
    window: Rect,
    viewport: Rect,
    css: Viewport,
    point: Point,
) -> Result<Point, String> {
    let invalid = || {
        "Safari cannot prove the page-to-screen coordinates. Keep the window visible and unzoomed, then read the page again.".to_owned()
    };
    if ![
        window.x,
        window.y,
        window.width,
        window.height,
        viewport.x,
        viewport.y,
        viewport.width,
        viewport.height,
        css.width,
        css.height,
        point.x,
        point.y,
    ]
    .into_iter()
    .all(f64::is_finite)
        || [
            window.width,
            window.height,
            viewport.width,
            viewport.height,
            css.width,
            css.height,
        ]
        .into_iter()
        .any(|n| n <= 0. || n > 65536.)
        || point.x < 0.
        || point.y < 0.
        || point.x >= css.width
        || point.y >= css.height
        || viewport.x < window.x - 1.
        || viewport.y < window.y - 1.
        || viewport.x + viewport.width > window.x + window.width + 1.
        || viewport.y + viewport.height > window.y + window.height + 1.
    {
        return Err(invalid());
    }
    let sx = viewport.width / css.width;
    let sy = viewport.height / css.height;
    if !(0.2..=5.).contains(&sx) || !(0.2..=5.).contains(&sy) || (sx - sy).abs() > 0.01 * sx.max(sy)
    {
        return Err(invalid());
    }
    Ok(Point {
        x: viewport.x + point.x * sx,
        y: viewport.y + point.y * sy,
    })
}
pub(super) fn same_window(a: Rect, b: Rect) -> bool {
    [a.x, a.y, a.width, a.height, b.x, b.y, b.width, b.height]
        .into_iter()
        .all(f64::is_finite)
        && a.width > 0.
        && a.height > 0.
        && b.width > 0.
        && b.height > 0.
        && [a.x - b.x, a.y - b.y, a.width - b.width, a.height - b.height]
            .into_iter()
            .all(|n| n.abs() <= 2.)
}
pub(super) fn window_index(windows: &[Rect], expected: Rect) -> Result<usize, String> {
    let mut matching = windows
        .iter()
        .enumerate()
        .filter(|(_, w)| same_window(**w, expected));
    let index=matching.next().map(|(i,_)|i).ok_or("Safari has no native window matching the checked bounds. Restore its window and read the page again.")?;
    if matching.next().is_some() {
        return Err("More than one Safari window matches the checked bounds. Move the intended window, then read it again.".into());
    }
    Ok(index)
}
/// Waits up to `wait` until `count()` reports at least one web area of the checked page. `WebKit` builds a page's
/// accessibility tree when it is first asked for, so the first look after Safari came forward can find none (the
/// first native input then refused although the page was in front). It never refuses by itself: the dispatch stage's
/// own check decides. A failed look counts as none; `stop` (cancellation) ends the wait with its error.
pub(super) fn await_exposed(
    wait: std::time::Duration,
    mut count: impl FnMut() -> Result<usize, String>,
    mut stop: impl FnMut() -> Result<(), String>,
    pause: impl Fn(),
) -> Result<(), String> {
    let until = std::time::Instant::now() + wait;
    loop {
        stop()?;
        if count().is_ok_and(|n| n > 0) || std::time::Instant::now() >= until {
            return Ok(());
        }
        pause();
    }
}
pub(super) fn recent(checked: u64, now: u128) -> Result<(), String> {
    if now < u128::from(checked) || now - u128::from(checked) > 1000 {
        return Err("The checked Safari target is no longer recent. Read the page and prepare a new action.".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn window() -> Rect {
        Rect {
            x: -1400.,
            y: 25.,
            width: 1200.,
            height: 900.,
        }
    }
    fn viewport() -> Rect {
        Rect {
            x: -1400.,
            y: 125.,
            width: 1200.,
            height: 800.,
        }
    }
    #[test]
    fn css_point_maps_to_screen_points_without_retina_multiplication() {
        let p = map(
            window(),
            viewport(),
            Viewport {
                width: 600.,
                height: 400.,
            },
            Point { x: 300., y: 100. },
        )
        .unwrap();
        assert_eq!(p, Point { x: -800., y: 325. });
    }
    #[test]
    fn unknown_scale_outside_points_and_uncontained_viewport_refuse() {
        let css = Viewport {
            width: 1200.,
            height: 800.,
        };
        assert!(map(window(), viewport(), css, Point { x: 1200., y: 20. }).is_err());
        assert!(
            map(
                window(),
                viewport(),
                css,
                Point {
                    x: f64::NAN,
                    y: 20.
                }
            )
            .is_err()
        );
        assert!(
            map(
                window(),
                Rect {
                    width: 1000.,
                    ..viewport()
                },
                css,
                Point { x: 20., y: 20. }
            )
            .is_err()
        );
        assert!(
            map(
                window(),
                Rect {
                    x: 0.,
                    ..viewport()
                },
                css,
                Point { x: 20., y: 20. }
            )
            .is_err()
        );
        assert!(
            map(
                window(),
                viewport(),
                Viewport {
                    width: 0.,
                    height: 800.
                },
                Point { x: 20., y: 20. }
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    /// PREDICT (before the change): the prepare stage did not wait for the page's accessibility tree, so the first
    /// native drag of a new Safari session window was refused ("did not expose one matching visible web area") and
    /// the same drag right after it completed (macOS 27). The wait stops at the first exposed web area, is
    /// bounded, treats a failed look as none, and a cancellation ends it.
    #[test]
    fn the_prepare_stage_waits_for_the_pages_web_area_but_never_past_its_bound() {
        use std::time::Duration;
        let mut seen = vec![Ok(0), Err("no tree yet".to_owned()), Ok(0), Ok(1), Ok(1)].into_iter();
        let mut looks = 0;
        let ok = await_exposed(
            Duration::from_secs(5),
            || {
                looks += 1;
                seen.next().unwrap()
            },
            || Ok(()),
            || {},
        );
        assert_eq!(
            (ok, looks),
            (Ok(()), 4),
            "stops at the first exposed web area"
        );
        let started = std::time::Instant::now();
        let bounded = await_exposed(
            Duration::from_millis(30),
            || Ok(0),
            || Ok(()),
            || std::thread::sleep(Duration::from_millis(5)),
        );
        assert_eq!(bounded, Ok(()), "the dispatch stage decides");
        assert!(
            started.elapsed() >= Duration::from_millis(30)
                && started.elapsed() < Duration::from_secs(2)
        );
        let cancelled = await_exposed(
            Duration::from_secs(5),
            || Ok(0),
            || Err("cancelled".to_owned()),
            || {},
        );
        assert_eq!(cancelled, Err("cancelled".to_owned()));
    }
    #[test]
    fn native_window_must_be_unique_finite_and_still_at_checked_bounds() {
        let r = Rect {
            x: 10.,
            y: 30.,
            width: 1200.,
            height: 800.,
        };
        assert_eq!(window_index(&[Rect { x: 500., ..r }, r], r).unwrap(), 1);
        assert!(window_index(&[r, r], r).is_err());
        assert!(window_index(&[], r).is_err());
        assert!(!same_window(r, Rect { x: 13., ..r }));
        assert!(!same_window(r, Rect { x: f64::NAN, ..r }));
        assert!(recent(1000, 2000).is_ok());
        assert!(recent(1000, 2001).is_err());
        assert!(recent(1001, 1000).is_err());
    }
}
