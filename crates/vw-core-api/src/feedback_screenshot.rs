//! Validates native, secret-safe app captures before they enter feedback state. Called only by the feedback controller.
use crate::{FeedbackScreenshot, feedback::Capture};
use base64::Engine;

pub(crate) async fn capture(provider: Option<&Capture>) -> Result<FeedbackScreenshot, String> {
    let provider = provider.ok_or("Screenshot unavailable. No screenshot attached.")?;
    let png = provider().await?;
    if png.len() > 1_048_576 {
        return Err("Screenshot is too large. No screenshot attached.".into());
    }
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Screenshot unavailable. No screenshot attached.".into());
    }
    let image = vw_preview::decode_limited(&png, 4096 * 4096 * 4)
        .map_err(|_| "Screenshot unavailable. No screenshot attached.")?;
    if image.width > 4096 || image.height > 4096 || image.width == 0 || image.height == 0 {
        return Err("Screenshot is too large. No screenshot attached.".into());
    }
    Ok(FeedbackScreenshot {
        mime_type: "image/png".into(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(png),
        width: image.width,
        height: image.height,
    })
}
