//! AVFoundation adapter for the macOS 11 floor. The helper owns blocking work and its external deadline.
//! Actual CMTime values and preferred display transforms accompany each bounded CoreGraphics image.
#![allow(deprecated)] // Synchronous image generation and track reads are available on the macOS 11 floor.
use super::{Frame, Metadata, geometry};
use objc2::rc::Retained;
use objc2_av_foundation::{AVAsset, AVAssetImageGenerator, AVMediaTypeVideo};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImage};
use objc2_core_media::{
    CMFormatDescription, CMFormatDescriptionGetMediaSubType, CMTime, CMTimeMake,
    CMVideoFormatDescriptionGetDimensions, CMVideoFormatDescriptionGetPresentationDimensions,
};
use objc2_foundation::{NSString, NSURL};
use std::path::Path;
/// Fresh AVFoundation generator; each job owns its asset and callback state.
pub struct Decoder {
    generator: Retained<AVAssetImageGenerator>,
    meta: Metadata,
}
impl Decoder {
    /// Opens the sandbox-authorized file URL and validates its video track.
    /// # Errors
    /// Unsupported format, missing track or unbounded source metadata.
    pub fn open(path: &Path, _edge: u32) -> Result<Self, String> {
        super::container::check(path)?;
        let path = path.to_str().ok_or("decode_failed: input path encoding")?;
        // SAFETY: the URL is a local file; owned Objective-C objects remain alive across every synchronous call.
        unsafe {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            let asset = AVAsset::assetWithURL(&url);
            let tracks = asset.tracksWithMediaType(
                AVMediaTypeVideo.ok_or("backend_unavailable: AVFoundation video type")?,
            );
            let track = tracks
                .firstObject()
                .ok_or("unsupported_container: AVFoundation has no video track")?;
            let descriptions = track.formatDescriptions();
            let format = descriptions
                .firstObject()
                .ok_or("unsupported_codec: no video format description")?;
            // AVAssetTrack's formatDescriptions contains CMFormatDescription CF objects (Apple API contract).
            let format = &*(Retained::as_ptr(&format).cast::<CMFormatDescription>());
            let size = CMVideoFormatDescriptionGetDimensions(format);
            let width = u32::try_from(size.width).map_err(|_| "limit_exceeded: source width")?;
            let height = u32::try_from(size.height).map_err(|_| "limit_exceeded: source height")?;
            geometry::geometry(width, height)?;
            let display = CMVideoFormatDescriptionGetPresentationDimensions(format, true, false);
            let par = (display.width / f64::from(width) * 1_000_000.0).round();
            if !par.is_finite() || par < 1.0 || par > 16_000_000.0 {
                return Err("limit_exceeded: pixel aspect ratio".into());
            }
            let transform = track.preferredTransform();
            let rotation = if transform.b > 0.5 {
                90
            } else if transform.b < -0.5 {
                270
            } else if transform.a < -0.5 {
                180
            } else {
                0
            };
            let codec = CMFormatDescriptionGetMediaSubType(format).to_be_bytes();
            let duration_ns = time_ns(asset.duration())?;
            if duration_ns == 0 || duration_ns > 7_200_000_000_000 {
                return Err("limit_exceeded: duration".into());
            }
            let generator = AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset);
            generator.setAppliesPreferredTrackTransform(true);
            generator.setRequestedTimeToleranceBefore(CMTimeMake(0, 1));
            generator.setRequestedTimeToleranceAfter(CMTimeMake(0, 1));
            Ok(Self {
                generator,
                meta: Metadata {
                    duration_ns,
                    width,
                    height,
                    rotation,
                    par: (par as u32, 1_000_000),
                    codec: String::from_utf8_lossy(&codec).into_owned(),
                },
            })
        }
    }
    /// Validated source metadata.
    /// # Errors
    /// No error after a successful open.
    pub fn metadata(&self) -> Result<Metadata, String> {
        Ok(self.meta.clone())
    }
    /// Returns a display-oriented square-pixel frame and AVFoundation's actual presentation timestamp.
    /// # Errors
    /// EOF, unsupported conversion or failed native image generation.
    pub fn frame(&mut self, ns: u64, edge: u32) -> Result<Frame, String> {
        if ns >= self.meta.duration_ns {
            return Err("decode_failed: requested time is at or after EOF".into());
        }
        if edge == 0 || edge > 4096 {
            return Err("limit_exceeded: image edge".into());
        }
        // SAFETY: generator and actual-time storage outlive the synchronous call; allocations are bounded below.
        unsafe {
            self.generator.setMaximumSize(CGSize {
                width: f64::from(edge),
                height: f64::from(edge),
            });
            let mut actual = CMTimeMake(0, 1);
            let image = self
                .generator
                .copyCGImageAtTime_actualTime_error(
                    CMTimeMake(
                        i64::try_from(ns).map_err(|_| "limit_exceeded: time")?,
                        1_000_000_000,
                    ),
                    &raw mut actual,
                )
                .map_err(|_| {
                    format!(
                        "unsupported_codec: AVFoundation could not decode {}",
                        self.meta.codec
                    )
                })?;
            let pts_ns = time_ns(actual)?;
            if pts_ns > ns {
                return Err("decode_failed: AVFoundation returned a later frame".into());
            }
            let width = CGImage::width(Some(&image));
            let height = CGImage::height(Some(&image));
            if width == 0 || height == 0 || width > edge as usize || height > edge as usize {
                return Err("limit_exceeded: generated geometry".into());
            }
            if CGImage::bits_per_component(Some(&image)) > 8 {
                return Err(
                    "unsupported_codec: HDR or high-bit-depth display conversion unavailable"
                        .into(),
                );
            }
            let mut pixels = vec![0; width * height * 4];
            let color =
                CGColorSpace::new_device_rgb().ok_or("backend_unavailable: RGB color space")?;
            let context = CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                width,
                height,
                8,
                width * 4,
                Some(&color),
                1 | 0x4000,
            )
            .ok_or("decode_failed: bitmap context")?;
            CGContext::translate_ctm(Some(&context), 0.0, height as f64);
            CGContext::scale_ctm(Some(&context), 1.0, -1.0);
            CGContext::draw_image(
                Some(&context),
                CGRect {
                    origin: CGPoint { x: 0.0, y: 0.0 },
                    size: CGSize {
                        width: width as f64,
                        height: height as f64,
                    },
                },
                Some(&image),
            );
            drop(context);
            Ok(Frame {
                pts_ns,
                width: width as u32,
                height: height as u32,
                pixels,
            })
        }
    }
}
fn time_ns(time: CMTime) -> Result<u64, String> {
    if time.timescale <= 0 || time.value < 0 || time.epoch != 0 || time.flags.0 & 0x1d != 1 {
        return Err("decode_failed: invalid native timeline".into());
    }
    let ns = u128::try_from(time.value).map_err(|_| "decode_failed: time")? * 1_000_000_000
        / time.timescale as u128;
    u64::try_from(ns).map_err(|_| "limit_exceeded: time".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_time_is_rational_and_nonnegative() {
        assert_eq!(
            time_ns(unsafe { CMTimeMake(1001, 30000) }).unwrap(),
            33_366_666
        );
        assert!(time_ns(unsafe { CMTimeMake(-1, 1000) }).is_err());
    }
}
