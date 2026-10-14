//! Screenshot shaping: every engine hands over a PNG; this module decodes it (bounded), converts it to sRGB when
//! it carries a colour profile, scales it down to at most [`MAX_WIDTH`] pixels wide and re-encodes it until it
//! fits [`MAX_BYTES`], so an agent's image never costs more than about a megabyte of context and transport
//! (brief: "size-capped") and shows the page's own colours.
//!
//! Depends on: png, base64, moxcms (ICC conversion). Called by: `service` (screenshot), tests (pixel probe).
//! Invariants:
//! - Decoding is limited to [`MAX_DECODE_BYTES`] of pixel memory; a larger or broken PNG is an error, never a
//!   partial image.
//! - The output is always a valid untagged 8-bit RGBA (or RGB when fully opaque) PNG in sRGB. `WebView2`
//!   captures in the display's colour space and tags the PNG with that ICC profile (on wide-gamut Windows 11 displays
//!   display turned #13a37f into (65,160,129)); re-encoding drops the tag, so the pixels are converted first. A
//!   profile that cannot be read or converted leaves the pixels as they are (logged).

use std::io::Cursor;

use base64::Engine as _;
use png::{BitDepth, ColorType, Compression, Decoder, Encoder, Limits, Transformations};

/// Widest screenshot returned.
pub const MAX_WIDTH: u32 = 1280;
/// Largest encoded screenshot returned.
pub const MAX_BYTES: usize = 1024 * 1024;
/// Most pixel memory a decode may use (a 4K x 4K RGBA frame).
pub const MAX_DECODE_BYTES: usize = 4096 * 4096 * 4;

/// A decoded RGBA8 image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes (`width * height * 4`).
    pub pixels: Vec<u8>,
}

impl Rgba {
    /// The pixel at (`x`, `y`) as `[r, g, b, a]`, `None` outside the image.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = (usize::try_from(y).ok()? * usize::try_from(self.width).ok()?
            + usize::try_from(x).ok()?)
            * 4;
        let p = self.pixels.get(at..at + 4)?;
        Some([p[0], p[1], p[2], p[3]])
    }
}

/// A shaped screenshot, ready to return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    /// PNG bytes (at most [`MAX_BYTES`]).
    pub png: Vec<u8>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Shot {
    /// The PNG as standard base64 (MCP image content, command results).
    #[must_use]
    pub fn base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.png)
    }
}

/// Decodes a PNG into RGBA8.
///
/// # Errors
/// A text reason for a broken, unsupported or too large image.
pub fn decode(png_bytes: &[u8]) -> Result<Rgba, String> {
    decode_limited(png_bytes, MAX_DECODE_BYTES)
}

/// [`decode`] with another pixel-memory limit (full-page captures).
///
/// # Errors
/// A text reason for a broken, unsupported or too large image.
pub fn decode_limited(png_bytes: &[u8], limit: usize) -> Result<Rgba, String> {
    let mut decoder = Decoder::new_with_limits(Cursor::new(png_bytes), Limits { bytes: limit });
    decoder.set_transformations(Transformations::normalize_to_color8() | Transformations::ALPHA);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("the screenshot is not a readable PNG: {e}"))?;
    let size = reader
        .output_buffer_size()
        .ok_or("the screenshot is too large")?;
    if size > limit {
        return Err("the screenshot is too large".to_owned());
    }
    let icc = reader.info().icc_profile.as_ref().map(|p| p.to_vec());
    let mut buf = vec![0u8; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("the screenshot could not be decoded: {e}"))?;
    buf.truncate(info.buffer_size());
    if info.bit_depth != BitDepth::Eight {
        return Err("the screenshot has an unsupported bit depth".to_owned());
    }
    let pixels = match info.color_type {
        ColorType::Rgba => buf,
        ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[r, g, b]| [*r, *g, *b, 255])
            .collect(),
        ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[g, a]| [*g, *g, *g, *a])
            .collect(),
        ColorType::Grayscale => buf.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        ColorType::Indexed => return Err("the screenshot has an unsupported palette".to_owned()),
    };
    let pixels = match icc {
        Some(profile) => to_srgb(&profile, pixels),
        None => pixels,
    };
    Ok(Rgba {
        width: info.width,
        height: info.height,
        pixels,
    })
}

/// Converts RGBA8 `pixels` from the RGB colour space described by the ICC `profile` to sRGB (relative
/// colorimetric, the default intent). Returns the pixels unchanged when the profile is not an RGB profile or
/// cannot be read (the caller then shows the engine's values, as before this conversion existed).
#[must_use]
pub fn to_srgb(profile: &[u8], pixels: Vec<u8>) -> Vec<u8> {
    let source = match moxcms::ColorProfile::new_from_slice(profile) {
        Ok(p) if p.color_space == moxcms::DataColorSpace::Rgb => p,
        Ok(_) => return pixels,
        Err(error) => {
            tracing::info!(%error, "screenshot colour profile unreadable: colours kept as captured");
            return pixels;
        }
    };
    let srgb = moxcms::ColorProfile::new_srgb();
    let transform = match source.create_transform_8bit(
        moxcms::Layout::Rgba,
        &srgb,
        moxcms::Layout::Rgba,
        moxcms::TransformOptions::default(),
    ) {
        Ok(t) => t,
        Err(error) => {
            tracing::info!(%error, "screenshot colour profile not convertible: colours kept as captured");
            return pixels;
        }
    };
    let mut out = vec![0u8; pixels.len()];
    match transform.transform(&pixels, &mut out) {
        Ok(()) => out,
        Err(error) => {
            tracing::info!(%error, "screenshot colour conversion failed: colours kept as captured");
            pixels
        }
    }
}

/// Encodes RGBA8 pixels (RGB when every pixel is opaque, which is smaller).
///
/// # Errors
/// A text reason when the buffer does not match the size or encoding fails.
pub fn encode(image: &Rgba) -> Result<Vec<u8>, String> {
    let expected = usize::try_from(u64::from(image.width) * u64::from(image.height) * 4)
        .map_err(|_| "image too large".to_owned())?;
    if image.pixels.len() != expected || image.width == 0 || image.height == 0 {
        return Err("image buffer does not match its size".to_owned());
    }
    let (rgba, _) = image.pixels.as_chunks::<4>();
    let opaque = rgba.iter().all(|p| p[3] == 255);
    let mut out = Vec::new();
    {
        let mut encoder = Encoder::new(&mut out, image.width, image.height);
        encoder.set_depth(BitDepth::Eight);
        encoder.set_compression(Compression::Balanced);
        let data: Vec<u8> = if opaque {
            encoder.set_color(ColorType::Rgb);
            rgba.iter().flat_map(|[r, g, b, _]| [*r, *g, *b]).collect()
        } else {
            encoder.set_color(ColorType::Rgba);
            image.pixels.clone()
        };
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&data).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Box-filter downscale to `width` (height keeps the aspect ratio, at least 1).
#[must_use]
pub fn downscale(image: &Rgba, width: u32) -> Rgba {
    if width >= image.width || width == 0 {
        return image.clone();
    }
    let height = ((u64::from(image.height) * u64::from(width)) / u64::from(image.width)).max(1);
    let height = u32::try_from(height).unwrap_or(1);
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 4);
    // Source span of output index `i` of `out` along a side of `src` pixels (never empty, inside the image).
    let span = |i: u32, out: u32, src: u32| -> (u32, u32) {
        let at =
            |k: u32| u32::try_from(u64::from(k) * u64::from(src) / u64::from(out)).unwrap_or(src);
        let start = at(i).min(src.saturating_sub(1));
        (start, at(i + 1).max(start + 1).min(src))
    };
    for y in 0..height {
        let (y0, y1) = span(y, height, image.height);
        for x in 0..width {
            let (x0, x1) = span(x, width, image.width);
            let mut sum = [0u64; 4];
            let mut count = 0u64;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    if let Some(p) = image.pixel(sx, sy) {
                        for (s, v) in sum.iter_mut().zip(p) {
                            *s += u64::from(v);
                        }
                        count += 1;
                    }
                }
            }
            let count = count.max(1);
            pixels.extend(sum.map(|s| u8::try_from(s / count).unwrap_or(u8::MAX)));
        }
    }
    Rgba {
        width,
        height,
        pixels,
    }
}

/// Shapes an engine PNG: at most [`MAX_WIDTH`] wide and [`MAX_BYTES`] large (scaled down further in steps of
/// three quarters until it fits; six steps at most).
///
/// # Errors
/// A text reason when the PNG cannot be decoded or still does not fit.
pub fn shape(png_bytes: &[u8]) -> Result<Shot, String> {
    let decoded = decode(png_bytes)?;
    let mut width = decoded.width.min(MAX_WIDTH);
    for _ in 0..6 {
        let scaled = downscale(&decoded, width);
        let png = encode(&scaled)?;
        if png.len() <= MAX_BYTES {
            return Ok(Shot {
                png,
                width: scaled.width,
                height: scaled.height,
            });
        }
        width = (width * 3 / 4).max(1);
    }
    Err("the screenshot stays above the size limit even scaled down".to_owned())
}
