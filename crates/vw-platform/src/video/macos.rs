//! `AVFoundation` adapter for the macOS 11 floor. The helper owns blocking work and its external deadline.
//! Actual `CMTime` values and preferred display transforms accompany each bounded CoreGraphics image.
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
use vw_video_types::{Code, Error};
/// Fresh `AVFoundation` generator; each job owns its asset and callback state.
pub struct Decoder {
    generator: Retained<AVAssetImageGenerator>,
    meta: Metadata,
}
impl Decoder {
    /// Opens the sandbox-authorized file URL and validates its video track.
    /// # Errors
    /// Unsupported format, missing track or unbounded source metadata.
    pub fn open(path: &Path, _edge: u32) -> Result<Self, Error> {
        super::container::check(path)?;
        let path = path
            .to_str()
            .ok_or(Error::new(Code::DecodeFailed, "input path encoding"))?;
        // SAFETY: the URL is a local file; owned Objective-C objects remain alive across every synchronous call.
        unsafe {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            let asset = AVAsset::assetWithURL(&url);
            let tracks = asset.tracksWithMediaType(AVMediaTypeVideo.ok_or(Error::new(
                Code::MediaUnavailable {
                    media: vw_video_types::Media::AvFoundation,
                },
                "AVFoundation video type",
            ))?);
            let Some(track) = tracks.firstObject() else {
                // An ISO file that passed the structural preflight but shows no track at all was never opened
                // by AVFoundation (an extensionless snapshot did this): a worker failure, not the file's format.
                return Err(if asset.tracks().count() == 0 && iso_media(path) {
                    Error::new(
                        Code::MediaOpen,
                        "AVFoundation could not open the video snapshot",
                    )
                } else {
                    Error::new(Code::NoVideoTrack, "AVFoundation has no video track")
                });
            };
            let descriptions = track.formatDescriptions();
            let format = descriptions.firstObject().ok_or(Error::new(
                Code::NoVideoTrack,
                "no video format description",
            ))?;
            // AVAssetTrack's formatDescriptions contains CMFormatDescription CF objects (Apple API contract).
            let format = &*(Retained::as_ptr(&format).cast::<CMFormatDescription>());
            let size = CMVideoFormatDescriptionGetDimensions(format);
            let width = u32::try_from(size.width)
                .map_err(|error| Error::at(Code::Dimensions, "source width", error))?;
            let height = u32::try_from(size.height)
                .map_err(|error| Error::at(Code::Dimensions, "source height", error))?;
            geometry::geometry(width, height)?;
            let display = CMVideoFormatDescriptionGetPresentationDimensions(format, true, false);
            let par = (display.width / f64::from(width) * 1_000_000.0).round();
            if !par.is_finite() || par < 1.0 || par > 16_000_000.0 {
                return Err(Error::new(Code::PixelAspect, "pixel aspect ratio"));
            }
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "rounded finite pixel ratio was checked in 1..=16000000 immediately above"
            )]
            let par = par as u32;
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
                return Err(Error::new(Code::Duration, "duration"));
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
                    par: (par, 1_000_000),
                    codec: String::from_utf8_lossy(&codec).into_owned(),
                },
            })
        }
    }
    /// Validated source metadata.
    /// # Errors
    /// No error after a successful open.
    pub fn metadata(&self) -> Result<Metadata, Error> {
        Ok(self.meta.clone())
    }
    /// Returns a display-oriented square-pixel frame and `AVFoundation`'s actual presentation timestamp.
    /// # Errors
    /// EOF, unsupported conversion or failed native image generation.
    pub fn frame(&mut self, ns: u64, edge: u32) -> Result<Frame, Error> {
        if ns >= self.meta.duration_ns {
            return Err(Error::new(
                Code::FrameMissing,
                "requested time is at or after EOF",
            ));
        }
        if edge == 0 || edge > 4096 {
            return Err(Error::new(Code::ImageEdge, "image edge"));
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
                        i64::try_from(ns)
                            .map_err(|error| Error::at(Code::DecodeLimit, "time", error))?,
                        1_000_000_000,
                    ),
                    &raw mut actual,
                )
                .map_err(|error| {
                    // Only AVErrorDecoderNotFound (-11833) means this computer lacks the decoder. A decoder that
                    // exists but fails here (-11821; the IOSurface sandbox denial looked exactly like this) is
                    // not a missing codec. The framework's domain and code stay in the reason.
                    let kind = if error.code() == -11833 {
                        Code::UnsupportedCodec { codec: None }
                    } else {
                        Code::DecodeFailed
                    };
                    Error::new(
                        kind,
                        format!(
                            "AVFoundation could not decode {} ({} {})",
                            self.meta.codec,
                            error.domain(),
                            error.code()
                        ),
                    )
                })?;
            let pts_ns = time_ns(actual)?;
            if pts_ns > ns {
                return Err(Error::new(
                    Code::FrameMissing,
                    "AVFoundation returned a later frame",
                ));
            }
            let width = CGImage::width(Some(&image));
            let height = CGImage::height(Some(&image));
            if width == 0 || height == 0 || width > edge as usize || height > edge as usize {
                return Err(Error::new(Code::Dimensions, "generated geometry"));
            }
            let frame_width = u32::try_from(width)
                .map_err(|error| Error::at(Code::Dimensions, "generated width", error))?;
            let frame_height = u32::try_from(height)
                .map_err(|error| Error::at(Code::Dimensions, "generated height", error))?;
            if CGImage::bits_per_component(Some(&image)) > 8 {
                return Err(Error::new(
                    Code::Hdr,
                    "HDR or high-bit-depth display conversion unavailable",
                ));
            }
            let mut pixels = vec![0; width * height * 4];
            let color = CGColorSpace::new_device_rgb()
                .ok_or(Error::new(Code::DecodeFailed, "RGB color space"))?;
            let context = CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                width,
                height,
                8,
                width * 4,
                Some(&color),
                1 | 0x4000,
            )
            .ok_or(Error::new(Code::DecodeFailed, "bitmap context"))?;
            // No CTM flip: drawing into a bitmap context already stores the image's top row first in `pixels`.
            // A flip here delivered every macOS frame upside down (owned-corpus barcodes appeared at the bottom).
            CGContext::draw_image(
                Some(&context),
                CGRect {
                    origin: CGPoint { x: 0.0, y: 0.0 },
                    size: CGSize {
                        width: f64::from(frame_width),
                        height: f64::from(frame_height),
                    },
                },
                Some(&image),
            );
            drop(context);
            Ok(Frame {
                pts_ns,
                width: frame_width,
                height: frame_height,
                pixels,
            })
        }
    }
}
fn iso_media(path: &str) -> bool {
    use std::io::Read;
    let mut header = [0; 8];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok_and(|()| &header[4..8] == b"ftyp")
}
fn time_ns(time: CMTime) -> Result<u64, Error> {
    super::native_values::time_ns(time.value, time.timescale, time.epoch, time.flags.0)
}
