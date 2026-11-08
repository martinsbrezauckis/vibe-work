//! Secret-safe app captures shared by feedback and the Debug API. Uses native window pixels and the UI secrets route.
//! Calls are serialized across both paths; unknown or moving secret fields prevent an image from leaving.
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

/// Why the native app screenshot could not safely be returned.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct WindowCaptureError {
    status: StatusCode,
    code: &'static str,
    message: String,
}
impl WindowCaptureError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    fn failed(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "windowFailed", message)
    }
    /// Whether missing or moving secret fields prevented a safe capture.
    #[must_use]
    pub fn secret_check_failed(&self) -> bool {
        matches!(self.code, "secretCheckFailed" | "secretMoved")
            || self.message == vw_preview::SHOTS_BLOCKED
    }
}
impl IntoResponse for WindowCaptureError {
    fn into_response(self) -> Response {
        crate::reply::guard_error(self.status, self.code, &self.message)
    }
}

/// What the UI's `secrets` route answered: the secret fields' rectangles (CSS pixels), how many show their value
/// in clear, the capture lock's epoch, the viewport width (and height for macOS viewport cropping).
#[derive(Debug, PartialEq)]
struct SecretState {
    rects: Vec<[f64; 4]>,
    clear: u64,
    epoch: u64,
    viewport: f64,
    #[cfg(target_os = "macos")]
    viewport_height: f64,
}

/// Asks the UI where secret fields are, turning the capture lock on (`lock = true`, before a capture) or off
/// (`false`, after it; the answer is read before the lock is released).
async fn secret_state(
    routes: &crate::UiRoutes,
    lock: bool,
) -> Result<SecretState, WindowCaptureError> {
    let refused = |why: &str| {
        WindowCaptureError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "secretCheckFailed",
            format!(
                "the screenshot is refused: the UI could not say where secret fields are ({why})"
            ),
        )
    };
    let handler = routes
        .get("secrets")
        .ok_or_else(|| refused("no secrets route"))?;
    let answer = handler(json!({ "lock": lock }))
        .await
        .map_err(|e| refused(&e.message))?;
    match answer.get("state").and_then(Value::as_str) {
        Some("busy") => return Err(refused("the window is busy with another request")),
        Some("pending") => return Err(refused("the UI has not finished checking secret fields")),
        _ => {}
    }
    let viewport = answer
        .get("viewportWidth")
        .and_then(Value::as_f64)
        .ok_or_else(|| refused("the UI returned no viewport width"))?;
    if !viewport.is_finite() || !(1.0..=10_000_000.0).contains(&viewport) {
        return Err(refused("the page has not loaded a viewport"));
    }
    #[cfg(target_os = "macos")]
    let viewport_height = answer
        .get("viewportHeight")
        .and_then(Value::as_f64)
        .filter(|height| *height >= 1.0)
        .ok_or_else(|| refused("the UI returned no viewport height"))?;
    let rects = answer
        .get("rects")
        .and_then(Value::as_array)
        .ok_or_else(|| refused("no rectangles"))?
        .iter()
        .map(|r| {
            let num = |key: &str| {
                r.get(key)
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && value.abs() <= 10_000_000.0)
                    .ok_or_else(|| refused("invalid secret rectangle"))
            };
            let rect = [num("x")?, num("y")?, num("width")?, num("height")?];
            if rect[2] < 0.0 || rect[3] < 0.0 {
                return Err(refused("invalid secret rectangle size"));
            }
            Ok(rect)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SecretState {
        rects,
        clear: answer
            .get("clear")
            .and_then(Value::as_u64)
            .ok_or_else(|| refused("no secret field count"))?,
        epoch: answer
            .get("epoch")
            .and_then(Value::as_u64)
            .ok_or_else(|| refused("no capture epoch"))?,
        viewport,
        #[cfg(target_os = "macos")]
        viewport_height,
    })
}

/// The page viewport in a native snapshot's pixels. Keep the width and origin so CSS rectangles use the same
/// scale for both secret blanking and element crops; only the extra rows below the viewport are removed.
#[cfg(any(target_os = "macos", test))]
fn viewport_rect(
    image_width: u32,
    image_height: u32,
    viewport_width: f64,
    viewport_height: f64,
) -> Result<vw_preview::PixelRect, String> {
    if image_width == 0
        || image_height == 0
        || !viewport_width.is_finite()
        || !viewport_height.is_finite()
        || viewport_width < 1.0
        || viewport_height < 1.0
    {
        return Err("invalid capture viewport".to_owned());
    }
    let height = (viewport_height * f64::from(image_width) / viewport_width).round();
    if !height.is_finite() || height < 1.0 || height > f64::from(image_height) {
        return Err("the snapshot does not cover the page viewport".to_owned());
    }
    // Checked above: a positive integer no larger than the existing image height.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let height = height as u32;
    Ok(vw_preview::PixelRect {
        x: 0,
        y: 0,
        width: image_width,
        height,
    })
}

/// `rects` (CSS pixels) in the capture's pixels for an image `image_width` wide, one pixel larger on every side.
fn to_pixels(found: &SecretState, image_width: u32) -> Vec<vw_preview::PixelRect> {
    let scale = f64::from(image_width) / found.viewport;
    // CSS pixels times the capture scale, rounded: window sizes fit an i64 by far.
    #[allow(clippy::cast_possible_truncation)]
    let px = |v: f64| (v * scale).round() as i64;
    found
        .rects
        .iter()
        .map(|[x, y, w, h]| vw_preview::PixelRect {
            x: px(*x) - 1,
            y: px(*y) - 1,
            width: u32::try_from(px(*w).max(0) + 2).unwrap_or(u32::MAX),
            height: u32::try_from(px(*h).max(0) + 2).unwrap_or(u32::MAX),
        })
        .collect()
}

/// The main window captured under the capture lock with every secret field blanked, and how many were blanked.
/// The lock holds across the capture; the answers before and after must match (same rectangles, same number
/// shown in clear, same lock epoch), else the image is discarded and the capture tried once more, then refused
/// (`409 secretMoved`). One screenshot at a time (the shared UI route registry), so two cannot release each other's lock.
///
/// # Errors
/// Refuses when native pixels or stable secret-field checks are unavailable.
pub async fn capture_main(
    app: &crate::SharedAppWindow,
    routes: &crate::UiRoutes,
) -> Result<(Vec<u8>, usize), WindowCaptureError> {
    let _one_at_a_time = routes.shots.lock().await;
    for _ in 0..2 {
        let before = secret_state(routes, true).await?;
        let captured = app.capture("main".to_owned()).await;
        let after = secret_state(routes, false).await?;
        let png = captured.map_err(WindowCaptureError::failed)?;
        if before != after {
            tracing::warn!("window screenshot discarded: secret fields changed during the capture");
            continue;
        }
        let mut image = vw_preview::decode(&png).map_err(WindowCaptureError::failed)?;
        #[cfg(target_os = "macos")]
        {
            // WKWebView's default snapshot uses its bounds, while its automatic title-bar inset reduces the
            // page viewport. Crop the composed image before blanking, using the same width-derived CSS scale.
            let rect = viewport_rect(
                image.width,
                image.height,
                after.viewport,
                after.viewport_height,
            )
            .map_err(WindowCaptureError::failed)?;
            if rect.height != image.height {
                image = vw_preview::crop(&image, rect).ok_or_else(|| {
                    WindowCaptureError::failed("the page viewport is outside the snapshot")
                })?;
            }
        }
        let rects = to_pixels(&after, image.width);
        for rect in &rects {
            // Clip first: offscreen fields cannot allocate a patch larger than the capture itself.
            let left = usize::try_from(rect.x.max(0))
                .unwrap_or(usize::MAX)
                .min(image.width as usize);
            let top = usize::try_from(rect.y.max(0))
                .unwrap_or(usize::MAX)
                .min(image.height as usize);
            let right = usize::try_from((rect.x + i64::from(rect.width)).max(0))
                .unwrap_or(usize::MAX)
                .min(image.width as usize);
            let bottom = usize::try_from((rect.y + i64::from(rect.height)).max(0))
                .unwrap_or(usize::MAX)
                .min(image.height as usize);
            for row in top..bottom {
                let start = (row * image.width as usize + left) * 4;
                let end = (row * image.width as usize + right) * 4;
                for pixel in image.pixels[start..end].as_chunks_mut::<4>().0 {
                    *pixel = [128, 128, 128, 255];
                }
            }
        }
        let png = vw_preview::encode(&image).map_err(WindowCaptureError::failed)?;
        return Ok((png, rects.len()));
    }
    Err(WindowCaptureError::new(
        StatusCode::CONFLICT,
        "secretMoved",
        "the screenshot is refused: secret fields changed while it was taken (twice)",
    ))
}

#[cfg(test)]
mod tests {
    use super::{SecretState, to_pixels, viewport_rect};

    #[test]
    fn viewport_excludes_native_insets_at_the_capture_scale() {
        let retina = viewport_rect(2560, 1600, 1280.0, 772.0).unwrap();
        assert_eq!(
            (retina.x, retina.y, retina.width, retina.height),
            (0, 0, 2560, 1544)
        );
        let ordinary = viewport_rect(1280, 800, 1280.0, 772.0).unwrap();
        assert_eq!((ordinary.width, ordinary.height), (1280, 772));
        let fractional = viewport_rect(1920, 1200, 1280.0, 771.0).unwrap();
        assert_eq!(fractional.height, 1157);
        assert_eq!(
            viewport_rect(2560, 1600, 1280.0, 800.0).unwrap().height,
            1600
        );
    }

    #[test]
    fn viewport_crop_preserves_pixels_and_secret_coordinates() {
        let mut image = vw_preview::canvas(20, 16, [255, 0, 0, 255]);
        let fields = SecretState {
            rects: vec![[3.0, 4.0, 2.0, 2.0]],
            clear: 0,
            epoch: 1,
            viewport: 10.0,
            #[cfg(target_os = "macos")]
            viewport_height: 6.0,
        };
        let before = to_pixels(&fields, image.width);
        let crop = viewport_rect(image.width, image.height, 10.0, 6.0).unwrap();
        image = vw_preview::crop(&image, crop).unwrap();
        let after = to_pixels(&fields, image.width);
        assert_eq!(before, after);
        for rect in after {
            let secret = vw_preview::canvas(rect.width, rect.height, [128, 128, 128, 255]);
            vw_preview::paste(&mut image, &secret, rect);
        }
        let cropped = image;
        assert_eq!((cropped.width, cropped.height), (20, 12));
        assert_eq!(cropped.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(cropped.pixel(6, 8), Some([128, 128, 128, 255]));
        assert_eq!(cropped.pixel(9, 11), Some([128, 128, 128, 255]));
    }

    #[test]
    fn viewport_refuses_invalid_or_incomplete_captures() {
        for (width, height) in [
            (0.0, 772.0),
            (-1.0, 772.0),
            (f64::NAN, 772.0),
            (1280.0, 0.0),
            (1280.0, f64::INFINITY),
            (1280.0, 801.0),
        ] {
            assert!(viewport_rect(2560, 1600, width, height).is_err());
        }
        assert!(viewport_rect(0, 1600, 1280.0, 772.0).is_err());
        assert!(viewport_rect(2560, 0, 1280.0, 772.0).is_err());
    }
}
