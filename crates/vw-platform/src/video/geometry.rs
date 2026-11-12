//! Display normalization shared by native decoders. Checked geometry precedes allocation.
#[cfg(not(target_os = "macos"))]
use super::{Frame, Metadata};
use vw_video_types::{Code, Error};
pub(super) fn geometry(width: u32, height: u32) -> Result<(), Error> {
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || u64::from(width) * u64::from(height) > 8_847_360
    {
        return Err(Error::new(Code::Dimensions, "source dimensions"));
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub(super) fn display(frame: &Frame, meta: &Metadata, edge: u32) -> Result<Frame, Error> {
    geometry(frame.width, frame.height)?;
    if edge == 0 || edge > 4096 || meta.par.0 == 0 || meta.par.1 == 0 {
        return Err(Error::new(Code::ImageEdge, "output geometry"));
    }
    let w = u64::from(frame.width) * u64::from(meta.par.0) / u64::from(meta.par.1);
    let h = u64::from(frame.height);
    if w == 0 || w > 16384 {
        return Err(Error::new(Code::PixelAspect, "pixel aspect ratio"));
    }
    let (dw, dh) = if meta.rotation == 90 || meta.rotation == 270 {
        (h, w)
    } else {
        (w, h)
    };
    let scale = dw.max(dh).max(u64::from(edge));
    let width = u32::try_from((dw * u64::from(edge) / scale).max(1))
        .map_err(|e| Error::new(Code::Dimensions, e))?;
    let height = u32::try_from((dh * u64::from(edge) / scale).max(1))
        .map_err(|e| Error::new(Code::Dimensions, e))?;
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
            let src = (usize::try_from(ny)
                .map_err(|error| Error::at(Code::DecodeLimit, "row", error))?
                * frame.width as usize
                + usize::try_from(nx)
                    .map_err(|error| Error::at(Code::DecodeLimit, "column", error))?)
                * 4;
            let dst = (y as usize * width as usize + x as usize) * 4;
            pixels[dst..dst + 4].copy_from_slice(
                frame
                    .pixels
                    .get(src..src + 4)
                    .ok_or(Error::new(Code::DecodeFailed, "short pixels"))?,
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
