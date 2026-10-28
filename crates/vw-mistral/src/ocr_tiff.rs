//! Converts every TIFF frame to bounded PNG input for image OCR, which otherwise reads only frame one.
//! Uses image-rs tiff/png; called by ocr before any network request. Errors identify the one-based page; a TIFF of
//! more than [`MAX_PAGES`] frames is refused before any request.

use std::io::{Cursor, Write};
use tiff::ColorType;
use tiff::decoder::{Decoder, DecodingResult, Limits};

use crate::OcrError;

/// Same 50 MiB document bound as vw-documents, also applied to decoded frames and total PNG bytes.
pub(crate) const MAX_BYTES: usize = 50 * 1024 * 1024;
/// The most frames one TIFF may have. Each frame is its own OCR request, so a TIFF gets the page budget Mistral OCR
/// applies to one PDF (1000 pages); a larger file is refused before any request.
pub(crate) const MAX_PAGES: usize = 1000;

pub(crate) fn page_error(page: usize, reason: impl std::fmt::Display) -> OcrError {
    OcrError::ImagePage {
        page,
        reason: reason.to_string(),
    }
}

pub(crate) fn frames(bytes: &[u8], deadline: std::time::Instant) -> Result<Vec<Vec<u8>>, OcrError> {
    convert(bytes, MAX_BYTES, deadline)
}

fn convert(
    bytes: &[u8],
    limit: usize,
    deadline: std::time::Instant,
) -> Result<Vec<Vec<u8>>, OcrError> {
    if bytes.len() > MAX_BYTES {
        return Err(page_error(1, "the TIFF exceeds the 50 MiB document limit"));
    }
    let mut limits = Limits::default();
    limits.decoding_buffer_size = limit;
    limits.intermediate_buffer_size = limit;
    let mut decoder = Decoder::new(Cursor::new(bytes))
        .map_err(|e| page_error(1, e))?
        .with_limits(limits);
    let mut result = Vec::new();
    let mut remaining = limit;
    loop {
        let page = result.len() + 1;
        if std::time::Instant::now() >= deadline {
            return Err(page_error(page, "reading the document timed out"));
        }
        let (width, height) = decoder.dimensions().map_err(|e| page_error(page, e))?;
        let color = decoder.colortype().map_err(|e| page_error(page, e))?;
        let pixels = decoder.read_image().map_err(|e| {
            if matches!(e, tiff::TiffError::LimitsExceeded) {
                page_error(page, "decoded image exceeds the 50 MiB limit")
            } else {
                page_error(page, e)
            }
        })?;
        let (color, depth, pixels) = png_pixels(color, pixels).map_err(|e| page_error(page, e))?;
        let mut output = Limited {
            bytes: Vec::new(),
            remaining,
        };
        {
            let mut encoder = png::Encoder::new(&mut output, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            let mut writer = encoder.write_header().map_err(|e| page_error(page, e))?;
            writer
                .write_image_data(&pixels)
                .map_err(|e| page_error(page, e))?;
            writer.finish().map_err(|e| page_error(page, e))?;
        }
        remaining -= output.bytes.len();
        result.push(output.bytes);
        if !decoder.more_images() {
            break;
        }
        if result.len() == MAX_PAGES {
            return Err(page_error(
                page + 1,
                format!(
                    "the TIFF has more than {MAX_PAGES} pages, the most OCR reads in one document"
                ),
            ));
        }
        decoder.next_image().map_err(|e| page_error(page + 1, e))?;
    }
    Ok(result)
}

#[allow(clippy::cast_possible_truncation)] // CMYK channel product / 255 is in 0..=255.
fn png_pixels(
    color: ColorType,
    pixels: DecodingResult,
) -> Result<(png::ColorType, png::BitDepth, Vec<u8>), String> {
    let bits = color.bit_depth();
    let png_color = match color {
        ColorType::Gray(_) => png::ColorType::Grayscale,
        ColorType::GrayA(_) => png::ColorType::GrayscaleAlpha,
        ColorType::RGB(_) => png::ColorType::Rgb,
        ColorType::RGBA(_) => png::ColorType::Rgba,
        ColorType::CMYK(8) => {
            let DecodingResult::U8(pixels) = pixels else {
                return Err(
                    "unsupported CMYK sample format; export this page as PNG or PDF".into(),
                );
            };
            let rgb = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    [p[0], p[1], p[2]]
                        .map(|v| ((u16::from(255 - v) * u16::from(255 - p[3])) / 255) as u8)
                })
                .collect();
            return Ok((png::ColorType::Rgb, png::BitDepth::Eight, rgb));
        }
        _ => {
            return Err(format!(
                "unsupported TIFF colour type {color:?}; export this page as PNG or PDF"
            ));
        }
    };
    let (depth, bytes) = match pixels {
        DecodingResult::U8(bytes) => (
            match bits {
                1 => png::BitDepth::One,
                2 => png::BitDepth::Two,
                4 => png::BitDepth::Four,
                8 => png::BitDepth::Eight,
                _ => return Err("unsupported TIFF sample depth".into()),
            },
            bytes,
        ),
        DecodingResult::U16(values) if bits == 16 => (
            png::BitDepth::Sixteen,
            values.into_iter().flat_map(u16::to_be_bytes).collect(),
        ),
        _ => return Err("unsupported TIFF sample format; export this page as PNG or PDF".into()),
    };
    Ok((png_color, depth, bytes))
}

struct Limited {
    bytes: Vec<u8>,
    remaining: usize,
}

impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(std::io::Error::other(
                "converted TIFF images exceed the 50 MiB document limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn deadline() -> std::time::Instant {
        std::time::Instant::now() + std::time::Duration::from_secs(10)
    }

    #[test]
    fn later_oversized_frame_is_named_and_no_partial_document_is_returned() {
        let mut file = Cursor::new(Vec::new());
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut file).unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::Gray8>(1, 1, &[42])
            .unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::Gray8>(32, 32, &[0; 1024])
            .unwrap();
        let error = convert(file.get_ref(), 512, deadline()).unwrap_err();
        assert!(
            matches!(error, OcrError::ImagePage { page: 2, .. }),
            "{error}"
        );
        assert!(error.to_string().contains("limit"), "{error}");
    }

    #[test]
    fn distinct_frame_pixels_and_sixteen_bit_samples_survive_conversion() {
        let mut file = Cursor::new(Vec::new());
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut file).unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::Gray16>(1, 1, &[0x1234])
            .unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::Gray16>(1, 1, &[0xabcd])
            .unwrap();
        let pages = frames(file.get_ref(), deadline()).unwrap();
        assert_eq!(pages.len(), 2);
        for (bytes, expected) in pages.iter().zip([[0x12, 0x34], [0xab, 0xcd]]) {
            let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
            let mut pixels = [0; 2];
            reader.next_frame(&mut pixels).unwrap();
            assert_eq!(pixels, expected);
        }
    }
}
