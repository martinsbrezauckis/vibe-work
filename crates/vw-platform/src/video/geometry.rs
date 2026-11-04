//! Display normalization shared by native decoders. Checked geometry precedes allocation.
#[cfg(not(target_os = "macos"))]
use super::{Frame, Metadata};
pub(super) fn geometry(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || u64::from(width) * u64::from(height) > 8_847_360
    {
        return Err("limit_exceeded: source dimensions".into());
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub(super) fn display(frame: &Frame, meta: &Metadata, edge: u32) -> Result<Frame, String> {
    geometry(frame.width, frame.height)?;
    if edge == 0 || edge > 4096 || meta.par.0 == 0 || meta.par.1 == 0 {
        return Err("limit_exceeded: output geometry".into());
    }
    let w = u64::from(frame.width) * u64::from(meta.par.0) / u64::from(meta.par.1);
    let h = u64::from(frame.height);
    if w == 0 || w > 16384 {
        return Err("limit_exceeded: pixel aspect ratio".into());
    }
    let (dw, dh) = if meta.rotation == 90 || meta.rotation == 270 {
        (h, w)
    } else {
        (w, h)
    };
    let scale = dw.max(dh).max(u64::from(edge));
    let width =
        u32::try_from((dw * u64::from(edge) / scale).max(1)).map_err(|_| "limit_exceeded")?;
    let height =
        u32::try_from((dh * u64::from(edge) / scale).max(1)).map_err(|_| "limit_exceeded")?;
    let mut pixels = vec![0; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let (nx, ny) = match meta.rotation {
                90 => (
                    u64::from(y) * u64::from(frame.width) / u64::from(height),
                    u64::from(width - 1 - x) * u64::from(frame.height) / u64::from(width),
                ),
                180 => (
                    u64::from(width - 1 - x) * u64::from(frame.width) / u64::from(width),
                    u64::from(height - 1 - y) * u64::from(frame.height) / u64::from(height),
                ),
                270 => (
                    u64::from(height - 1 - y) * u64::from(frame.width) / u64::from(height),
                    u64::from(x) * u64::from(frame.height) / u64::from(width),
                ),
                _ => (
                    u64::from(x) * u64::from(frame.width) / u64::from(width),
                    u64::from(y) * u64::from(frame.height) / u64::from(height),
                ),
            };
            let src = (usize::try_from(ny).map_err(|_| "limit_exceeded: row")?
                * frame.width as usize
                + usize::try_from(nx).map_err(|_| "limit_exceeded: column")?)
                * 4;
            let dst = (y as usize * width as usize + x as usize) * 4;
            pixels[dst..dst + 4].copy_from_slice(
                frame
                    .pixels
                    .get(src..src + 4)
                    .ok_or("decode_failed: short pixels")?,
            );
        }
    }
    Ok(Frame {
        pts_ns: frame.pts_ns,
        width,
        height,
        pixels,
    })
}
