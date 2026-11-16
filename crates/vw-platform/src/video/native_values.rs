//! Runtime-free native media values. Windows row layouts and macOS rational times are checked before use.
//! Both adapters call these functions; their unit tests also run on Linux.

use vw_video_types::{Code, Error};
#[cfg(any(test, target_os = "macos"))]
pub(super) fn time_ns(value: i64, scale: i32, epoch: i64, flags: u32) -> Result<u64, Error> {
    if scale <= 0 || value < 0 || epoch != 0 || flags & 0x1d != 1 {
        return Err(Error::new(Code::DecodeFailed, "invalid native timeline"));
    }
    let ns = u128::try_from(value).map_err(|error| Error::at(Code::DecodeFailed, "time", error))?
        * 1_000_000_000
        / u128::try_from(scale)
            .map_err(|error| Error::at(Code::DecodeFailed, "time scale", error))?;
    u64::try_from(ns).map_err(|error| Error::at(Code::DecodeLimit, "time", error))
}

#[cfg(any(test, windows))]
pub(super) fn rgb32(
    bytes: &[u8],
    encoded: (u32, u32),
    visible: (u32, u32),
    stride: i32,
) -> Result<Vec<u8>, Error> {
    let (w, h) = encoded;
    let (width, height) = visible;
    super::geometry::geometry(w, h)?;
    super::geometry::geometry(width, height)?;
    if width > w
        || height > h
        || stride.unsigned_abs() < w * 4
        || (bytes.len() as u64) < u64::from(stride.unsigned_abs()) * u64::from(h)
    {
        return Err(Error::new(Code::DecodeFailed, "invalid RGB32 layout"));
    }
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        let y = if stride < 0 { h - 1 - y } else { y };
        let row = y as usize * stride.unsigned_abs() as usize;
        for x in 0..width {
            let at = row + x as usize * 4;
            out.extend_from_slice(&[bytes[at + 2], bytes[at + 1], bytes[at], 255]);
        }
    }
    Ok(out)
}

/// Reason for a failed Media Foundation call. `E_OUTOFMEMORY` means the decoder needed more than the worker's Job
/// memory, so it is a limit, not missing codec support.
#[cfg(any(test, windows))]
pub(super) fn media_foundation(code: i32) -> Error {
    if code.cast_unsigned() == 0x8007_000E {
        return Error::new(Code::Memory, "video worker memory");
    }
    Error::new(
        if code.cast_unsigned() == 0xC00D_5212 {
            Code::UnsupportedCodec { codec: None }
        } else {
            Code::DecodeFailed
        },
        format!("Media Foundation error {code:#x}"),
    )
}

/// Media Foundation step of a Windows source open with its own reading of a failure.
#[cfg(any(test, windows))]
#[derive(Clone, Copy, Debug)]
pub(super) enum Opening {
    /// Reading the source's presentation duration (`MF_PD_DURATION`).
    Duration,
    /// Decoding the first frame, after the RGB32 decoder chain was accepted.
    FirstFrame,
}

/// Reason for a failed Media Foundation call while a source is opened. Two codes there name a format this
/// computer cannot read: a source without a duration (`MF_E_ATTRIBUTENOTFOUND` for `MF_PD_DURATION`, a live `WebM`
/// recording) is an unsupported container, and a generic failure (`E_FAIL`) on the first frame after the decoder
/// accepted the stream (H.264 High 10) is an unsupported codec. Every other code, and every failure after the
/// first frame, keeps the general reading of [`media_foundation`].
#[cfg(any(test, windows))]
pub(super) fn media_foundation_open(step: Opening, code: i32) -> Error {
    match (step, code.cast_unsigned()) {
        (Opening::Duration, 0xC00D_36E6) => Error::new(
            Code::UnsupportedContainer,
            format!("Media Foundation reports no source duration ({code:#x})"),
        ),
        (Opening::FirstFrame, 0x8000_4005) => Error::new(
            Code::UnsupportedCodec { codec: None },
            format!("Media Foundation cannot decode the first frame of this format ({code:#x})"),
        ),
        _ => media_foundation(code),
    }
}

/// Reason `AVFoundation` shows no video track: `tracks` counts the asset's tracks of every media type and `iso`
/// says the file starts with an ISO `ftyp` box.
#[cfg(any(test, target_os = "macos"))]
pub(super) fn avfoundation_without_video(tracks: usize, iso: bool) -> Error {
    match (tracks, iso) {
        // An ISO file that passed the structural preflight but shows no track at all was never opened
        // by AVFoundation (an extensionless snapshot did this): a worker failure, not the file's format.
        (0, true) => Error::new(
            Code::MediaOpen,
            "AVFoundation could not open the video snapshot",
        ),
        // No track of any type in a file that is not ISO media: AVFoundation cannot read this container (`WebM`).
        (0, false) => Error::new(
            Code::UnsupportedContainer,
            "AVFoundation cannot read this container",
        ),
        // The file was read and has tracks, none of them video (an audio-only MP4).
        _ => Error::new(Code::NoVideoTrack, "AVFoundation has no video track"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_foundation_out_of_memory_is_the_worker_memory_limit() {
        // E_OUTOFMEMORY: the decoder ran out of the worker's Job memory, not of codec support.
        assert_eq!(
            media_foundation(0x8007_000E_u32.cast_signed()).code,
            Code::Memory
        );
        let missing = media_foundation(0xC00D_5212_u32.cast_signed());
        assert_eq!(missing.code, Code::UnsupportedCodec { codec: None });
        assert!(missing.detail.contains("0xc00d5212"));
        assert_eq!(
            media_foundation(0x8000_4005_u32.cast_signed()).code,
            Code::DecodeFailed
        );
    }
    #[test]
    fn media_foundation_open_names_formats_this_computer_cannot_read() {
        // PREDICT (red before the fix): every open failure reads through the general mapping, so a live WebM
        // without a duration (MF_E_ATTRIBUTENOTFOUND for MF_PD_DURATION) and an H.264 High 10 first frame
        // (E_FAIL) are DecodeFailed instead of the named unsupported container / codec.
        let live = media_foundation_open(Opening::Duration, 0xC00D_36E6_u32.cast_signed());
        assert_eq!(live.code, Code::UnsupportedContainer);
        assert!(live.detail.contains("0xc00d36e6"));
        let high10 = media_foundation_open(Opening::FirstFrame, 0x8000_4005_u32.cast_signed());
        assert_eq!(high10.code, Code::UnsupportedCodec { codec: None });
        assert!(high10.detail.contains("0x80004005"));
        // Everything else keeps the general reading: a missing decoder stays unsupported, the worker memory
        // limit stays a limit, and any other failure, at open or while decoding later frames, is DecodeFailed.
        for step in [Opening::Duration, Opening::FirstFrame] {
            assert_eq!(
                media_foundation_open(step, 0xC00D_5212_u32.cast_signed()).code,
                Code::UnsupportedCodec { codec: None }
            );
            assert_eq!(
                media_foundation_open(step, 0x8007_000E_u32.cast_signed()).code,
                Code::Memory
            );
            assert_eq!(
                media_foundation_open(step, 0xC00D_36B2_u32.cast_signed()).code,
                Code::DecodeFailed
            );
        }
        assert_eq!(
            media_foundation_open(Opening::Duration, 0x8000_4005_u32.cast_signed()).code,
            Code::DecodeFailed
        );
        assert_eq!(
            media_foundation_open(Opening::FirstFrame, 0xC00D_36E6_u32.cast_signed()).code,
            Code::DecodeFailed
        );
    }
    #[test]
    fn avfoundation_without_video_separates_unreadable_containers_from_files_without_video() {
        // PREDICT (red before the fix): a file AVFoundation cannot read at all (no track of any type, not ISO
        // media, e.g. every WebM) reads NoVideoTrack instead of the named unsupported container.
        assert_eq!(
            avfoundation_without_video(0, false).code,
            Code::UnsupportedContainer
        );
        // A readable file with tracks but none of them video (an audio-only MP4) has no video track.
        assert_eq!(avfoundation_without_video(1, true).code, Code::NoVideoTrack);
        assert_eq!(
            avfoundation_without_video(2, false).code,
            Code::NoVideoTrack
        );
        // ISO media that passed the preflight but was never opened stays a media-open failure.
        assert_eq!(avfoundation_without_video(0, true).code, Code::MediaOpen);
    }
    #[test]
    fn rational_time_rejects_invalid_flags_epochs_and_overflow() {
        assert_eq!(time_ns(1001, 30000, 0, 1).unwrap(), 33_366_666);
        assert_eq!(time_ns(1, 3, 0, 3).unwrap(), 333_333_333);
        for args in [
            (-1, 1000, 0, 1),
            (1, 0, 0, 1),
            (1, 1, 1, 1),
            (1, 1, 0, 0),
            (1, 1, 0, 5),
            (1, 1, 0, 9),
            (1, 1, 0, 17),
            (i64::MAX, 1, 0, 1),
        ] {
            assert!(time_ns(args.0, args.1, args.2, args.3).is_err());
        }
    }
    #[test]
    fn rgb32_rows_are_cropped_flipped_and_converted_without_padding() {
        let bytes = [1, 2, 3, 0, 99, 99, 99, 99, 4, 5, 6, 0, 88, 88, 88, 88];
        assert_eq!(
            rgb32(&bytes, (2, 2), (1, 2), 8).unwrap(),
            [3, 2, 1, 255, 6, 5, 4, 255]
        );
        assert_eq!(
            rgb32(&bytes, (2, 2), (1, 2), -8).unwrap(),
            [6, 5, 4, 255, 3, 2, 1, 255]
        );
        assert!(rgb32(&bytes[..15], (2, 2), (1, 2), 8).is_err());
        assert!(rgb32(&bytes, (2, 2), (1, 2), 4).is_err());
        assert!(rgb32(&bytes, (2, 2), (3, 2), 8).is_err());
        assert!(rgb32(&bytes, (0, 2), (1, 2), 8).is_err());
    }
}
