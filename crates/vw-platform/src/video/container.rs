//! Bounded ISO media structural preflight. Refuses truncated boxes, oversized tables and external references
//! before native frameworks can follow references or allocate from untrusted sample counts.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
pub(super) fn check(path: &Path) -> Result<bool, String> {
    let mut file = File::open(path).map_err(|_| "decode_failed: input unavailable")?;
    let len = file
        .metadata()
        .map_err(|_| "decode_failed: input metadata")?
        .len();
    if len == 0 || len > 512 * 1024 * 1024 {
        return Err("limit_exceeded: video file size".into());
    }
    let mut header = [0; 8];
    file.read_exact(&mut header)
        .map_err(|_| "decode_failed: incomplete header")?;
    if &header[4..8] != b"ftyp" {
        return Ok(false);
    }
    let mut at = 0;
    let mut moov = false;
    let mut fragmented = false;
    for _ in 0..100_000 {
        if at == len {
            return if moov {
                Ok(fragmented)
            } else {
                Err("decode_failed: missing movie metadata".into())
            };
        }
        file.seek(SeekFrom::Start(at))
            .map_err(|_| "decode_failed: box seek")?;
        file.read_exact(&mut header)
            .map_err(|_| "decode_failed: truncated box header")?;
        let size = u32::from_be_bytes(
            header[..4]
                .try_into()
                .map_err(|_| "decode_failed: box header")?,
        );
        let (size, head) = if size == 1 {
            let mut large = [0; 8];
            file.read_exact(&mut large)
                .map_err(|_| "decode_failed: truncated large box")?;
            (u64::from_be_bytes(large), 16)
        } else if size == 0 {
            (len - at, 8)
        } else {
            (u64::from(size), 8)
        };
        if size < head || size > len - at {
            return Err("decode_failed: truncated container box".into());
        }
        fragmented |= &header[4..8] == b"moof";
        if &header[4..8] == b"moov" {
            if moov || size > 16 * 1024 * 1024 {
                return Err("limit_exceeded: movie metadata".into());
            }
            moov = true;
            let mut body = vec![
                0;
                usize::try_from(size - head)
                    .map_err(|_| "limit_exceeded: metadata bytes")?
            ];
            file.read_exact(&mut body)
                .map_err(|_| "decode_failed: truncated movie metadata")?;
            boxes(&body, 0, &mut 0)?;
        }
        at += size;
    }
    Err("limit_exceeded: container boxes".into())
}
fn boxes(mut data: &[u8], depth: usize, count: &mut usize) -> Result<(), String> {
    if depth > 8 {
        return Err("limit_exceeded: container nesting".into());
    }
    while !data.is_empty() {
        *count += 1;
        if *count > 100_000 || data.len() < 8 {
            return Err("decode_failed: invalid movie boxes".into());
        }
        let n =
            u32::from_be_bytes(data[..4].try_into().map_err(|_| "decode_failed: box")?) as usize;
        if n < 8 || n > data.len() {
            return Err("decode_failed: incomplete movie box".into());
        }
        let kind = &data[4..8];
        let body = &data[8..n];
        match kind {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"dinf" => {
                boxes(body, depth + 1, count)?;
            }
            b"rmra" | b"rmda" | b"rdrf" => {
                return Err("unsupported_container: external movie reference".into());
            }
            b"dref" => {
                if body.len() < 8 {
                    return Err("decode_failed: data reference table".into());
                }
                let mut refs = &body[8..];
                while !refs.is_empty() {
                    if refs.len() < 12 {
                        return Err("decode_failed: data reference".into());
                    }
                    let size = u32::from_be_bytes(
                        refs[..4]
                            .try_into()
                            .map_err(|_| "decode_failed: data reference")?,
                    ) as usize;
                    if size < 12 || size > refs.len() || refs[11] & 1 == 0 {
                        return Err("unsupported_container: external media reference".into());
                    }
                    refs = &refs[size..];
                }
            }
            b"stts" | b"ctts" | b"stsc" | b"stss" | b"stco" | b"co64" => {
                if body.len() < 8 {
                    return Err("decode_failed: sample table".into());
                }
                let entries = u32::from_be_bytes(
                    body[4..8]
                        .try_into()
                        .map_err(|_| "decode_failed: sample count")?,
                ) as usize;
                let unit = match kind {
                    b"stsc" => 12,
                    b"stss" | b"stco" => 4,
                    _ => 8,
                };
                if entries > 1_000_000 || entries > (body.len() - 8) / unit {
                    return Err("limit_exceeded: sample table".into());
                }
            }
            b"stsz" => {
                if body.len() < 12 {
                    return Err("decode_failed: sample sizes".into());
                }
                let entries = u32::from_be_bytes(
                    body[8..12]
                        .try_into()
                        .map_err(|_| "decode_failed: sample count")?,
                ) as usize;
                if entries > 1_000_000
                    || (body[4..8] == [0, 0, 0, 0] && entries > (body.len() - 12) / 4)
                {
                    return Err("limit_exceeded: sample sizes".into());
                }
            }
            _ => {}
        }
        data = &data[n..];
    }
    Ok(())
}
