//! Encodes screenshots into a bounded GIF without an external executable; recording owns the inputs.
//! Frames scale to the first frame's canvas, at most 800 by 600, with timing clipped to 0.1–5 seconds.
use super::recording::Frame;
pub(super) fn encode(frames: Vec<Frame>) -> Result<Vec<u8>, String> {
    let first = frames
        .first()
        .ok_or("The recording has no captured frames; start a new recording in a visible tab.")?;
    let image = vw_preview::decode(&first.png)?;
    let mut width = image.width.min(800);
    let mut height = (image.height * width / image.width).max(1);
    if height > 600 {
        width = (width * 600 / height).max(1);
        height = 600;
    }
    let width = u16::try_from(width).map_err(|_| "Invalid frame width; start a new recording.")?;
    let height =
        u16::try_from(height).map_err(|_| "Invalid frame height; start a new recording.")?;
    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, width, height, &[])
            .map_err(|e| format!("GIF could not start: {e}; try a shorter recording."))?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        for frame in frames {
            let image = vw_preview::decode(&frame.png)?;
            let mut pixels = vec![255; usize::from(width) * usize::from(height) * 4];
            for y in 0..height {
                for x in 0..width {
                    let sx = u32::from(x) * image.width / u32::from(width);
                    let sy = u32::from(y) * image.height / u32::from(height);
                    if let Some(pixel) = image.pixel(sx, sy) {
                        let at = (usize::from(y) * usize::from(width) + usize::from(x)) * 4;
                        pixels[at..at + 4].copy_from_slice(&pixel);
                    }
                }
            }
            let mut picture = gif::Frame::from_rgba_speed(width, height, &mut pixels, 10);
            picture.delay = frame.delay;
            encoder.write_frame(&picture).map_err(|e| {
                format!("A GIF frame could not be encoded: {e}; start a shorter recording.")
            })?;
        }
    }
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("The finished GIF exceeds 32 MiB; record fewer actions.".into());
    }
    Ok(bytes)
}
