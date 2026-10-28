//! The size of a sharp viewport snapshot of a zoomed preview (block macOS): a preview laid out at a device
//! width wider than its panel is shown at page zoom = panel / device width, so an ordinary snapshot has the panel's
//! size and the page drawn smaller (a 1280 px Desktop page in a 640 pt panel: 640 pt wide). Asking `WebKit` for a
//! snapshot `css width` points wide (`WKSnapshotConfiguration.snapshotWidth`) renders the same viewport again at one
//! point per CSS pixel, so the picture is `css width x backing scale` pixels whatever the zoom.
//!
//! The limits are the capture's own (`vw-browser` `MAX_PIXEL_WIDTH` 8192, `MAX_PIXELS` 32,000,000; the Linux
//! capture checks the same in `vw-webview` `linux_capture::capture_size`), checked BEFORE `WebKit` allocates the bitmap.
//!
//! Depends on: nothing. Called by: `webview::sharp_mac` (macOS), tests.
//! Invariants: total (NaN, zero or negative inputs give an error, never a size).

/// Widest picture in device pixels.
pub const MAX_PIXEL_WIDTH: f64 = 8192.0;
/// Largest picture in device pixels.
pub const MAX_PIXELS: f64 = 32_000_000.0;

/// The snapshot width in points (= CSS pixels of the viewport) for a webview of `bounds` points (width, height) at
/// page zoom `zoom` on a screen of `scale` device pixels per point; `None` when the zoom is 1 or more (an ordinary
/// snapshot is already one point per CSS pixel or more).
///
/// # Errors
/// Invalid geometry, or a picture over the pixel limits.
pub fn snapshot_width(bounds: (f64, f64), zoom: f64, scale: f64) -> Result<Option<f64>, String> {
    let valid = |v: f64| v.is_finite() && v > 0.0;
    if !(valid(bounds.0) && valid(bounds.1) && valid(zoom) && valid(scale)) {
        return Err("invalid capture viewport".to_owned());
    }
    if zoom >= 1.0 {
        return Ok(None);
    }
    let width = (bounds.0 / zoom).ceil();
    let height = (bounds.1 / zoom).ceil();
    if width * scale > MAX_PIXEL_WIDTH || width * height * scale * scale > MAX_PIXELS {
        return Err("capture viewport exceeds the pixel limits".to_owned());
    }
    Ok(Some(width))
}

#[cfg(test)]
mod tests {
    use super::snapshot_width;

    #[test]
    fn desktop_page_in_a_half_panel() {
        // 640 x 400 pt panel showing a 1280 px Desktop page (zoom 0.5): 1280 points, at any backing scale.
        assert_eq!(snapshot_width((640.0, 400.0), 0.5, 2.0), Ok(Some(1280.0)));
        assert_eq!(snapshot_width((640.0, 400.0), 0.5, 1.0), Ok(Some(1280.0)));
        // Rounded up: a CSS pixel is never lost.
        assert_eq!(snapshot_width((640.0, 400.0), 0.6, 2.0), Ok(Some(1067.0)));
    }

    #[test]
    fn no_gain_at_zoom_one_or_more() {
        assert_eq!(snapshot_width((1280.0, 800.0), 1.0, 2.0), Ok(None));
        assert_eq!(snapshot_width((1280.0, 800.0), 2.0, 2.0), Ok(None));
    }

    #[test]
    fn limits_and_invalid_geometry() {
        // 4096 css px wide at scale 2 = 8192 px: the limit itself is allowed; one more css px is not.
        assert_eq!(snapshot_width((2048.0, 100.0), 0.5, 2.0), Ok(Some(4096.0)));
        assert!(snapshot_width((2048.5, 100.0), 0.5, 2.0).is_err());
        // Area: 1280 x 8000 css px at scale 2 = 40,960,000 px (the oversized capture case).
        assert!(snapshot_width((640.0, 4000.0), 0.5, 2.0).is_err());
        for bad in [
            ((0.0, 400.0), 0.5, 2.0),
            ((640.0, -1.0), 0.5, 2.0),
            ((640.0, 400.0), f64::NAN, 2.0),
            ((640.0, 400.0), 0.5, 0.0),
            ((f64::INFINITY, 400.0), 0.5, 2.0),
        ] {
            assert!(snapshot_width(bad.0, bad.1, bad.2).is_err(), "{bad:?}");
        }
    }
}
