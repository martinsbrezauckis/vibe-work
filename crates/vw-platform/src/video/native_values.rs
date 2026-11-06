//! Runtime-free native media values. Windows row layouts and macOS rational times are checked before use.
//! Both adapters call these functions; their unit tests also run on Linux.

#[cfg(any(test, target_os = "macos"))]
pub(super) fn time_ns(value: i64, scale: i32, epoch: i64, flags: u32) -> Result<u64, String> {
    if scale <= 0 || value < 0 || epoch != 0 || flags & 0x1d != 1 {
        return Err("decode_failed: invalid native timeline".into());
    }
    let ns = u128::try_from(value).map_err(|_| "decode_failed: time")? * 1_000_000_000
        / u128::try_from(scale).map_err(|_| "decode_failed: time scale")?;
    u64::try_from(ns).map_err(|_| "limit_exceeded: time".into())
}

#[cfg(any(test, windows))]
pub(super) fn rgb32(
    bytes: &[u8],
    encoded: (u32, u32),
    visible: (u32, u32),
    stride: i32,
) -> Result<Vec<u8>, String> {
    let (w, h) = encoded;
    let (width, height) = visible;
    super::geometry::geometry(w, h)?;
    super::geometry::geometry(width, height)?;
    if width > w
        || height > h
        || stride.unsigned_abs() < w * 4
        || (bytes.len() as u64) < u64::from(stride.unsigned_abs()) * u64::from(h)
    {
        return Err("decode_failed: invalid RGB32 layout".into());
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
pub(super) fn media_foundation(code: i32) -> String {
    if code.cast_unsigned() == 0x8007_000E {
        return "limit_exceeded: video worker memory".into();
    }
    format!("unsupported_codec: Media Foundation decoder or media support unavailable ({code:#x})")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_foundation_out_of_memory_is_the_worker_memory_limit() {
        // E_OUTOFMEMORY: the decoder ran out of the worker's Job memory, not of codec support.
        assert_eq!(
            media_foundation(0x8007_000E_u32.cast_signed()),
            "limit_exceeded: video worker memory"
        );
        assert_eq!(
            media_foundation(0xC00D_5212_u32.cast_signed()),
            "unsupported_codec: Media Foundation decoder or media support unavailable (0xc00d5212)"
        );
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
