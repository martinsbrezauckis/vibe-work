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
    _com: Com,
}
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
    format!(
        "unsupported_codec: Media Foundation decoder or media support unavailable ({:#x})",
        e.code().0
    )
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
        // SAFETY: a valid reader and stack-owned PROPVARIANT remain alive for the synchronous call.
        unsafe {
            self.reader
                .SetCurrentPosition(&GUID::zeroed(), &PROPVARIANT::from(target))
                .map_err(failure)?;
        }
        let mut previous = None;
        for _ in 0..100_000 {
            let Some((pts, sample)) = self.sample()? else {
                break;
            };
            if pts > target {
                break;
            }
            previous = Some((pts, sample));
        }
        let (pts, sample) = previous.ok_or("decode_failed: no covering sample")?;
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
                if ptr.is_null()
                    || stride.unsigned_abs() < w * 4
                    || u64::from(len) < u64::from(stride.unsigned_abs()) * u64::from(h)
                {
                    return Err("decode_failed: invalid RGB32 layout".into());
                }
                let bytes = std::slice::from_raw_parts(ptr, len as usize);
                let mut out =
                    Vec::with_capacity(self.meta.width as usize * self.meta.height as usize * 4);
                for y in 0..self.meta.height {
                    let y = if stride < 0 { h - 1 - y } else { y };
                    let row = y as usize * stride.unsigned_abs() as usize;
                    for x in 0..self.meta.width {
                        let at = row + x as usize * 4;
                        out.extend_from_slice(&[bytes[at + 2], bytes[at + 1], bytes[at], 255]);
                    }
                }
                Ok(out)
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
