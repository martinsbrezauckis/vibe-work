//! Whole-window screenshots from parts: the app window is the main webview plus child webviews
//! (previews, the Project tab's viewer) at their places; on Linux and macOS the window image is made by pasting
//! each part's own snapshot onto a canvas, and a client may ask for one element's rectangle (crop).
//!
//! Depends on: `shot` (`Rgba`). Called by: vw-webview (`window_shot`), vw-debug-api (`/screenshot?crop=`).
//! Invariants:
//! - Every operation stays inside the canvas: parts and crops are clipped, never written or read out of bounds.
//! - A part whose snapshot size differs from its place (a scale rounding of one pixel, a webview that has not
//!   repainted after a resize) is resized to the place with nearest-neighbour sampling, so the composite always
//!   has the window's exact size.

use crate::shot::Rgba;

/// A rectangle in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    /// Left edge.
    pub x: i64,
    /// Top edge.
    pub y: i64,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// A blank opaque canvas of `width` x `height` filled with `fill` (RGBA).
#[must_use]
pub fn canvas(width: u32, height: u32, fill: [u8; 4]) -> Rgba {
    let count = (width as usize) * (height as usize);
    Rgba {
        width,
        height,
        pixels: fill.repeat(count),
    }
}

/// `image` resized to `width` x `height` (nearest neighbour; the same image when the size already matches).
#[must_use]
pub fn fit(image: &Rgba, width: u32, height: u32) -> Rgba {
    if image.width == width && image.height == height {
        return image.clone();
    }
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in 0..height {
        let sy = u32::try_from(u64::from(y) * u64::from(image.height) / u64::from(height.max(1)))
            .unwrap_or(0);
        for x in 0..width {
            let sx = u32::try_from(u64::from(x) * u64::from(image.width) / u64::from(width.max(1)))
                .unwrap_or(0);
            pixels.extend(image.pixel(sx, sy).unwrap_or([0, 0, 0, 255]));
        }
    }
    Rgba {
        width,
        height,
        pixels,
    }
}

/// Pastes `part` onto `canvas` at `place` (resized to it first, clipped to the canvas; alpha is ignored: parts
/// are opaque webview surfaces).
pub fn paste(canvas: &mut Rgba, part: &Rgba, place: PixelRect) {
    let part = fit(part, place.width, place.height);
    let row = canvas.width as usize * 4;
    for py in 0..part.height {
        let cy = place.y + i64::from(py);
        if cy < 0 || cy >= i64::from(canvas.height) {
            continue;
        }
        for px in 0..part.width {
            let cx = place.x + i64::from(px);
            if cx < 0 || cx >= i64::from(canvas.width) {
                continue;
            }
            let (Ok(cxu), Ok(cyu)) = (usize::try_from(cx), usize::try_from(cy)) else {
                continue;
            };
            if let Some(p) = part.pixel(px, py) {
                let at = cyu * row + cxu * 4;
                if let Some(dst) = canvas.pixels.get_mut(at..at + 4) {
                    dst.copy_from_slice(&[p[0], p[1], p[2], 255]);
                }
            }
        }
    }
}

/// The part of `image` inside `rect` (clipped to the image). `None` when nothing of `rect` is inside.
#[must_use]
pub fn crop(image: &Rgba, rect: PixelRect) -> Option<Rgba> {
    let x0 = rect.x.clamp(0, i64::from(image.width));
    let y0 = rect.y.clamp(0, i64::from(image.height));
    let x1 = (rect.x + i64::from(rect.width)).clamp(0, i64::from(image.width));
    let y1 = (rect.y + i64::from(rect.height)).clamp(0, i64::from(image.height));
    let (width, height) = (u32::try_from(x1 - x0).ok()?, u32::try_from(y1 - y0).ok()?);
    if width == 0 || height == 0 {
        return None;
    }
    let (x0, y0) = (u32::try_from(x0).ok()?, u32::try_from(y0).ok()?);
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in y0..y0 + height {
        for x in x0..x0 + width {
            pixels.extend(image.pixel(x, y)?);
        }
    }
    Some(Rgba {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_clips_and_crop_reads_back() {
        let mut base = canvas(10, 10, [0, 0, 0, 255]);
        let red = canvas(4, 4, [255, 0, 0, 255]);
        paste(
            &mut base,
            &red,
            PixelRect {
                x: 8,
                y: -2,
                width: 4,
                height: 4,
            },
        );
        assert_eq!(base.pixel(9, 0), Some([255, 0, 0, 255]));
        assert_eq!(base.pixel(9, 2), Some([0, 0, 0, 255]));
        let part = crop(
            &base,
            PixelRect {
                x: 8,
                y: 0,
                width: 5,
                height: 1,
            },
        )
        .unwrap();
        assert_eq!((part.width, part.height), (2, 1));
        assert!(
            crop(
                &base,
                PixelRect {
                    x: 20,
                    y: 0,
                    width: 5,
                    height: 5
                }
            )
            .is_none()
        );
        let resized = fit(&red, 8, 2);
        assert_eq!((resized.width, resized.height), (8, 2));
    }
}
