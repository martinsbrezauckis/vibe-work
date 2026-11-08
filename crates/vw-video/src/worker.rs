//! Helper-only extraction and composition. Never loaded by the default core; output encoding stays constrained.
use crate::protocol::{Frame, Job, Reply};
use vw_agent_tools::video::Output;
use vw_platform::video::Decoder;
use vw_preview::{PixelRect, Rgba};
/// Executes a validated request in a process whose sandbox and resource limits are already active.
/// # Errors
/// Unavailable media support, an incomplete decode or any exceeded limit.
#[allow(
    clippy::cast_possible_truncation,
    reason = "request count is at most 24 and each output axis is capped before arithmetic"
)]
pub fn extract(job: &Job) -> Result<Reply, String> {
    let mut times = job.request.times()?;
    let edge = job
        .request
        .max_edge
        .unwrap_or(vw_preview::MAX_WIDTH)
        .min(vw_preview::MAX_WIDTH);
    if edge == 0 {
        return Err("limit_exceeded: image edge".into());
    }
    let mut decoder = Decoder::open(std::path::Path::new(&job.request.path), edge)?;
    let meta = decoder.metadata()?;
    job.request.validate_duration(meta.duration_ns)?;
    let (selected, scan) = if job.request.scene_limit().is_some() {
        let (selected, scan) = crate::scenes::select(&mut decoder, &job.request)?;
        times = selected.iter().map(|f| f.requested_ms).collect();
        (selected, Some(scan))
    } else {
        (Vec::new(), None)
    };
    let mut reply = Reply {
        id: job.id.clone(),
        backend: vw_platform::video::backend(),
        duration_ns: meta.duration_ns,
        source_size: [meta.width, meta.height],
        rotation: meta.rotation,
        pixel_aspect_ratio: [meta.par.0, meta.par.1],
        codec: meta.codec,
        frames: Vec::new(),
        planned_frames: job.request.frame_limit()?,
        scan,
        image_count: 0,
        image_size: [0, 0],
        columns: 0,
        png: String::new(),
    };
    if job.request.output == Output::Metadata || times.is_empty() {
        reply.frames = selected;
        return Ok(reply);
    }
    compose(&mut decoder, job, times, edge, reply)
}
#[allow(
    clippy::cast_possible_truncation,
    reason = "at most 24 selected frames with bounded image axes"
)]
fn compose(
    decoder: &mut Decoder,
    job: &Job,
    times: Vec<u64>,
    edge: u32,
    mut reply: Reply,
) -> Result<Reply, String> {
    let cols = if job.request.output == Output::Frame {
        1
    } else {
        4.min(times.len() as u32)
    };
    let rows = (times.len() as u32).div_ceil(cols);
    let cell_w = edge / cols;
    let cell_h = if job.request.output == Output::Frame {
        edge
    } else {
        (edge / rows).min(cell_w * 3 / 4).max(32)
    };
    let mut sheet = vw_preview::canvas(edge, cell_h * rows, [24, 24, 24, 255]);
    let mut unique = Vec::new();
    // A contact sheet never displays a source frame wider than one cell. Let native decoders
    // scale at extraction time where supported, and avoid large intermediate RGBA copies.
    let frame_edge = decode_edge(edge, cols, job.request.output);
    for (index, ms) in times.into_iter().enumerate() {
        let frame = decoder.frame(ms * 1_000_000, frame_edge)?;
        let frame_id = if let Some(i) = unique.iter().position(|p| *p == frame.pts_ns) {
            i
        } else {
            let id = unique.len();
            unique.push(frame.pts_ns);
            let image = Rgba {
                width: frame.width,
                height: frame.height,
                pixels: frame.pixels,
            };
            if job.request.output == Output::Frame {
                sheet = image;
            } else {
                let w = cell_w.saturating_sub(8).max(1);
                let h = cell_h.saturating_sub(22).max(1);
                let scale = (u64::from(image.width) * u64::from(h))
                    .max(u64::from(image.height) * u64::from(w));
                let dw =
                    (u64::from(image.width) * u64::from(w) * u64::from(h) / scale).max(1) as u32;
                let dh =
                    (u64::from(image.height) * u64::from(w) * u64::from(h) / scale).max(1) as u32;
                let x = (id as u32 % cols) * cell_w;
                let y = (id as u32 / cols) * cell_h;
                vw_preview::paste(
                    &mut sheet,
                    &image,
                    PixelRect {
                        x: i64::from(x + (cell_w - dw) / 2),
                        y: i64::from(y + 2),
                        width: dw,
                        height: dh,
                    },
                );
                stamp(&mut sheet, x + 4, y + cell_h - 16, frame.pts_ns / 1_000_000);
            }
            id
        };
        reply.frames.push(Frame {
            index,
            requested_ms: ms,
            actual_ns: frame.pts_ns,
            frame_id,
        });
    }
    // Reuse the screenshot encoder and byte ceiling, including its bounded downscaling policy.
    let shot = vw_preview::shape(&vw_preview::encode(&sheet)?)?;
    reply.image_count = 1;
    reply.image_size = [shot.width, shot.height];
    reply.columns = cols;
    reply.png = shot.base64();
    Ok(reply)
}

fn decode_edge(edge: u32, cols: u32, output: Output) -> u32 {
    if output == Output::ContactSheet {
        edge / cols
    } else {
        edge
    }
}
// Small built-in decimal glyphs avoid font, network and filesystem access in the helper.
#[allow(
    clippy::cast_possible_truncation,
    reason = "glyph coordinates are bounded by the small decimal timestamp and canvas"
)]
fn stamp(image: &mut Rgba, x: u32, y: u32, ms: u64) {
    const DIGITS: [[u8; 5]; 11] = [
        [7, 5, 5, 5, 7],
        [2, 6, 2, 2, 7],
        [7, 1, 7, 4, 7],
        [7, 1, 7, 1, 7],
        [5, 5, 7, 1, 1],
        [7, 4, 7, 1, 7],
        [7, 4, 7, 5, 7],
        [7, 1, 1, 1, 1],
        [7, 5, 7, 5, 7],
        [7, 5, 7, 1, 7],
        [0, 0, 0, 0, 2],
    ];
    let text = format!("{}.{:03}", ms / 1000, ms % 1000);
    for (i, c) in text.bytes().enumerate() {
        let glyph = if c == b'.' { 10 } else { usize::from(c - b'0') };
        for (row, bits) in DIGITS[glyph].iter().enumerate() {
            for col in 0..3 {
                if bits & (1 << (2 - col)) != 0 {
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let px = x + i as u32 * 8 + col * 2 + dx;
                            let py = y + row as u32 * 2 + dy;
                            if px < image.width && py < image.height {
                                let at = (py as usize * image.width as usize + px as usize) * 4;
                                image.pixels[at..at + 4].copy_from_slice(&[240, 240, 240, 255]);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn contact_sheet_decodes_at_cell_size() {
        assert_eq!(
            super::decode_edge(1280, 4, vw_agent_tools::video::Output::ContactSheet),
            320
        );
        assert_eq!(
            super::decode_edge(1280, 1, vw_agent_tools::video::Output::Frame),
            1280
        );
    }
}
