//! Media Foundation Source Reader adapter. Called only inside the native helper after sandbox setup.
//! Normalizes the initial presentation offset, clips padded RGB rows and never uses decode timestamps.
use super::{Frame, Metadata, geometry};
use std::path::Path;
#[allow(
    clippy::wildcard_imports,
    reason = "Media Foundation constants and interfaces form the native API surface"
)]
use windows::{
    Win32::{
        Media::MediaFoundation::*,
        System::Com::{
            COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize, StructuredStorage::PROPVARIANT,
        },
    },
    core::{GUID, PCWSTR},
};
const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0.cast_unsigned();
/// Fresh Media Foundation video reader. Thread-affine COM state is owned for the whole decode job.
pub struct Decoder {
    reader: IMFSourceReader,
    meta: Metadata,
    offset: i64,
    position: Option<Position>,
    _com: Com,
}
/// Where forward decoding stopped: the sample last returned and the first one after it (`None` at EOF).
/// A seek always restarts at a key frame, so a close later request (a scene scan's next sample) continues from
/// here instead; each source frame is then decoded once rather than once per sample. The Job's CPU-time limit
/// (`isolation`) ended dense 1080p scans with long key-frame intervals when every sample seeked.
struct Position {
    covering: (i64, IMFSample),
    next: Option<(i64, IMFSample)>,
}
/// Furthest a request may lie past the decoded position (100 ns units) to continue without seeking. Bounds the
/// extra decoding to one second of video when a key frame sits between the position and the target.
const FORWARD: i64 = 10_000_000;
struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: matched to successful startup on this same helper thread.
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err consumes the native error"
)]
fn failure(e: windows::core::Error) -> String {
    super::native_values::media_foundation(e.code().0)
}
impl Decoder {
    /// Opens the authorized private source with only its video stream selected.
    /// # Errors
    /// Missing Media Foundation/codec, malformed source or excessive geometry.
    pub fn open(path: &Path, _edge: u32) -> Result<Self, String> {
        super::container::check(path)?;
        // SAFETY: all COM calls use owned interfaces, initialized on this helper thread; strings are NUL terminated.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(failure)?;
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(failure(e));
            }
            let com = Com;
            let mut attrs = None;
            MFCreateAttributes(&raw mut attrs, 2).map_err(failure)?;
            let attrs = attrs.ok_or("backend_unavailable: Media Foundation attributes")?;
            attrs
                .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
                .map_err(failure)?;
            attrs
                .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 0)
                .map_err(failure)?;
            let wide: Vec<u16> = path
                .to_string_lossy()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let reader =
                MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attrs).map_err(failure)?;
            reader
                .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0.cast_unsigned(), false)
                .map_err(failure)?;
            reader.SetStreamSelection(VIDEO, true).map_err(failure)?;
            let native = reader.GetNativeMediaType(VIDEO, 0).map_err(failure)?;
            let packed = native.GetUINT64(&MF_MT_FRAME_SIZE).map_err(failure)?;
            let width = (packed >> 32) as u32;
            let height = (packed & 0xffff_ffff) as u32;
            geometry::geometry(width, height)?;
            let par = native
                .GetUINT64(&MF_MT_PIXEL_ASPECT_RATIO)
                .unwrap_or((1u64 << 32) | 1);
            let rotation = native.GetUINT32(&MF_MT_VIDEO_ROTATION).unwrap_or(0);
            if ![0, 90, 180, 270].contains(&rotation) {
                return Err("unsupported_codec: video rotation".into());
            }
            let transfer = native.GetUINT32(&MF_MT_TRANSFER_FUNCTION).unwrap_or(0);
            if [15, 16, 18].contains(&transfer) {
                return Err("unsupported_codec: HDR conversion unavailable".into());
            }
            let subtype = native.GetGUID(&MF_MT_SUBTYPE).map_err(failure)?;
            let duration = reader
                .GetPresentationAttribute(
                    MF_SOURCE_READER_MEDIASOURCE.0.cast_unsigned(),
                    &MF_PD_DURATION,
                )
                .map_err(failure)?;
            let duration = u64::try_from(&duration)
                .map_err(failure)?
                .checked_mul(100)
                .filter(|n| *n > 0 && *n <= 7_200_000_000_000)
                .ok_or("limit_exceeded: duration")?;
            let mt = MFCreateMediaType().map_err(failure)?;
            mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(failure)?;
            mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                .map_err(failure)?;
            reader
                .SetCurrentMediaType(VIDEO, None, &mt)
                .map_err(failure)?;
            let meta = Metadata {
                duration_ns: duration,
                width,
                height,
                rotation,
                par: ((par >> 32) as u32, (par & 0xffff_ffff) as u32),
                codec: codec(subtype),
            };
            let mut d = Self {
                reader,
                meta,
                offset: 0,
                position: None,
                _com: com,
            };
            d.offset = d.sample()?.ok_or("decode_failed: empty video")?.0;
            Ok(d)
        }
    }
    /// Source metadata on the visible timeline.
    /// # Errors
    /// No error after a successful open.
    pub fn metadata(&self) -> Result<Metadata, String> {
        Ok(self.meta.clone())
    }
    /// Reads the sample covering the requested normalized presentation time.
    /// # Errors
    /// EOF, missing sample or a failed native decoder.
    pub fn frame(&mut self, ns: u64, edge: u32) -> Result<Frame, String> {
        if ns >= self.meta.duration_ns {
            return Err("decode_failed: requested time is at or after EOF".into());
        }
        let target = i64::try_from(ns / 100)
            .map_err(|_| "limit_exceeded: timestamp")?
            .checked_add(self.offset)
            .ok_or("limit_exceeded: timestamp")?;
        // Media Foundation refuses a seek past its duration (MF_E_INVALID_POSITION). With a presentation
        // offset (B-frame delay) the last frames lie beyond it, so seek no further than the duration and decode
        // forward: the seek lands on a key frame at or before `target` either way.
        let last = i64::try_from(self.meta.duration_ns / 100)
            .map_err(|_| "limit_exceeded: timestamp")?
            .saturating_sub(1);
        let seek = target.min(last.max(0));
        let (pts, sample) = match self.position.take() {
            // The covering sample is still the last one at or before `target` (or the last before EOF).
            Some(Position { covering, next })
                if covering.0 <= target && next.as_ref().is_none_or(|n| target < n.0) =>
            {
                self.position = Some(Position {
                    covering: covering.clone(),
                    next,
                });
                covering
            }
            Some(Position {
                covering,
                next: Some(next),
            }) if covering.0 <= target && target - next.0 <= FORWARD => {
                drop(covering);
                self.forward(Some(next), target)?
            }
            _ => {
                // SAFETY: a valid reader and stack-owned PROPVARIANT remain alive for the synchronous call.
                unsafe {
                    self.reader
                        .SetCurrentPosition(&GUID::zeroed(), &PROPVARIANT::from(seek))
                        .map_err(failure)?;
                }
                self.forward(None, target)?
            }
        };
        let pixels = self.pixels(&sample)?;
        let pts_ns = u64::try_from(pts - self.offset)
            .map_err(|_| "decode_failed: PTS before visible timeline")?
            .checked_mul(100)
            .ok_or("limit_exceeded: PTS")?;
        geometry::display(
            &Frame {
                pts_ns,
                width: self.meta.width,
                height: self.meta.height,
                pixels,
            },
            &self.meta,
            edge,
        )
    }
    /// Decodes forward to the last sample at or before `target`, keeping the position for the next request.
    /// The position is only stored on success; any error leaves none, so the next request seeks.
    fn forward(
        &mut self,
        mut previous: Option<(i64, IMFSample)>,
        target: i64,
    ) -> Result<(i64, IMFSample), String> {
        // `None` while the bound is not reached: neither EOF nor a later sample was seen, so keep no position.
        let mut ended = None;
        for _ in 0..100_000 {
            let Some((pts, sample)) = self.sample()? else {
                ended = Some(None);
                break;
            };
            if pts > target {
                ended = Some(Some((pts, sample)));
                break;
            }
            previous = Some((pts, sample));
        }
        let covering = previous.ok_or("decode_failed: no covering sample")?;
        if let Some(next) = ended {
            self.position = Some(Position {
                covering: covering.clone(),
                next,
            });
        }
        Ok(covering)
    }
    fn sample(&self) -> Result<Option<(i64, IMFSample)>, String> {
        for _ in 0..1024 {
            let (mut flags, mut pts, mut sample) = (0, 0, None);
            // SAFETY: output pointers are valid stack storage and the reader is initialized.
            unsafe {
                self.reader
                    .ReadSample(
                        VIDEO,
                        0,
                        None,
                        Some(&raw mut flags),
                        Some(&raw mut pts),
                        Some(&raw mut sample),
                    )
                    .map_err(failure)?;
            }
            if flags & MF_SOURCE_READERF_ERROR.0.cast_unsigned() != 0 {
                return Err("decode_failed: native decoder failure".into());
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0.cast_unsigned() != 0 {
                return Ok(None);
            }
            if let Some(sample) = sample {
                return Ok(Some((pts, sample)));
            }
        }
        Err("limit_exceeded: sample events".into())
    }
    fn pixels(&self, sample: &IMFSample) -> Result<Vec<u8>, String> {
        // SAFETY: all interfaces are owned; the borrowed native pointer is used only while its buffer is locked.
        unsafe {
            let mt = self.reader.GetCurrentMediaType(VIDEO).map_err(failure)?;
            // After the first decoded frame the reader can switch to a decoder format its video processor cannot
            // convert (10-bit VP9 becomes P010, with empty buffers). That video is unsupported, not a layout fault.
            if mt.GetGUID(&MF_MT_SUBTYPE).map_err(failure)? != MFVideoFormat_RGB32 {
                return Err(
                    "unsupported_codec: Media Foundation cannot convert this video's pixel format"
                        .into(),
                );
            }
            let packed = mt.GetUINT64(&MF_MT_FRAME_SIZE).map_err(failure)?;
            let w = (packed >> 32) as u32;
            let h = (packed & 0xffff_ffff) as u32;
            geometry::geometry(w, h)?;
            if w < self.meta.width || h < self.meta.height {
                return Err("decode_failed: changed video geometry".into());
            }
            let stride = mt
                .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                .map_or((w * 4).cast_signed(), u32::cast_signed);
            let buffer = sample.ConvertToContiguousBuffer().map_err(failure)?;
            let (mut ptr, mut len) = (std::ptr::null_mut(), 0);
            buffer
                .Lock(&raw mut ptr, None, Some(&raw mut len))
                .map_err(failure)?;
            let result = (|| {
                if ptr.is_null() {
                    return Err("decode_failed: invalid RGB32 layout".into());
                }
                let bytes = std::slice::from_raw_parts(ptr, len as usize);
                super::native_values::rgb32(
                    bytes,
                    (w, h),
                    (self.meta.width, self.meta.height),
                    stride,
                )
            })();
            buffer.Unlock().map_err(failure)?;
            result
        }
    }
}
fn codec(id: GUID) -> String {
    if id == MFVideoFormat_H264 {
        "H.264".into()
    } else if id == MFVideoFormat_HEVC {
        "HEVC".into()
    } else {
        format!("Media Foundation {id:?}")
    }
}
