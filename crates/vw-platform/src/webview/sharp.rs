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
