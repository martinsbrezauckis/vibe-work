//! Bounded ISO media structural preflight. Refuses truncated boxes, oversized tables and external references
//! before native frameworks can follow references or allocate from untrusted sample counts.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
use vw_video_types::{Code, Error};
pub(super) fn check(path: &Path) -> Result<bool, Error> {
    let mut file = File::open(path)
        .map_err(|error| Error::at(Code::DecodeFailed, "input unavailable", error))?;
    let len = file
        .metadata()
        .map_err(|error| Error::at(Code::DecodeFailed, "input metadata", error))?
        .len();
    if len == 0 || len > 512 * 1024 * 1024 {
        return Err(Error::new(Code::InputSize, "video file size"));
    }
    let mut header = [0; 8];
    file.read_exact(&mut header)
        .map_err(|error| Error::at(Code::DecodeFailed, "incomplete header", error))?;
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
                Err(Error::new(Code::DecodeFailed, "missing movie metadata"))
            };
        }
        file.seek(SeekFrom::Start(at))
            .map_err(|error| Error::at(Code::DecodeFailed, "box seek", error))?;
        file.read_exact(&mut header)
            .map_err(|error| Error::at(Code::DecodeFailed, "truncated box header", error))?;
        let size = u32::from_be_bytes(
            header[..4]
                .try_into()
                .map_err(|error| Error::at(Code::DecodeFailed, "box header", error))?,
        );
        let (size, head) = if size == 1 {
            let mut large = [0; 8];
            file.read_exact(&mut large)
                .map_err(|error| Error::at(Code::DecodeFailed, "truncated large box", error))?;
            (u64::from_be_bytes(large), 16)
        } else if size == 0 {
            (len - at, 8)
        } else {
            (u64::from(size), 8)
        };
        if size < head || size > len - at {
            return Err(Error::new(Code::DecodeFailed, "truncated container box"));
        }
        fragmented |= &header[4..8] == b"moof";
        if &header[4..8] == b"moov" {
            if moov || size > 16 * 1024 * 1024 {
                return Err(Error::new(Code::MetadataLimit, "movie metadata"));
            }
            moov = true;
            let mut body = vec![
                0;
                usize::try_from(size - head).map_err(|error| Error::at(
                    Code::MetadataLimit,
                    "metadata bytes",
                    error
                ))?
            ];
            file.read_exact(&mut body).map_err(|error| {
                Error::at(Code::DecodeFailed, "truncated movie metadata", error)
            })?;
            boxes(&body, 0, &mut 0)?;
        }
        at += size;
    }
    Err(Error::new(Code::MetadataLimit, "container boxes"))
}
fn boxes(mut data: &[u8], depth: usize, count: &mut usize) -> Result<(), Error> {
    if depth > 8 {
        return Err(Error::new(Code::MetadataLimit, "container nesting"));
    }
    while !data.is_empty() {
        *count += 1;
        if *count > 100_000 || data.len() < 8 {
            return Err(Error::new(Code::DecodeFailed, "invalid movie boxes"));
        }
        let n = u32::from_be_bytes(
            data[..4]
                .try_into()
                .map_err(|error| Error::at(Code::DecodeFailed, "box", error))?,
        ) as usize;
        if n < 8 || n > data.len() {
            return Err(Error::new(Code::DecodeFailed, "incomplete movie box"));
        }
        let kind = &data[4..8];
        let body = &data[8..n];
        match kind {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"dinf" => {
                boxes(body, depth + 1, count)?;
            }
            b"rmra" | b"rmda" | b"rdrf" => {
                return Err(Error::new(Code::ExternalMedia, "external movie reference"));
            }
            b"dref" => {
                if body.len() < 8 {
                    return Err(Error::new(Code::DecodeFailed, "data reference table"));
                }
                let mut refs = &body[8..];
                while !refs.is_empty() {
                    if refs.len() < 12 {
                        return Err(Error::new(Code::DecodeFailed, "data reference"));
                    }
                    let size =
                        u32::from_be_bytes(refs[..4].try_into().map_err(|error| {
                            Error::at(Code::DecodeFailed, "data reference", error)
                        })?) as usize;
                    if size < 12 || size > refs.len() || refs[11] & 1 == 0 {
                        return Err(Error::new(Code::ExternalMedia, "external media reference"));
                    }
                    refs = &refs[size..];
                }
            }
            b"stts" | b"ctts" | b"stsc" | b"stss" | b"stco" | b"co64" => {
                if body.len() < 8 {
                    return Err(Error::new(Code::DecodeFailed, "sample table"));
                }
                let entries = u32::from_be_bytes(
                    body[4..8]
                        .try_into()
                        .map_err(|error| Error::at(Code::DecodeFailed, "sample count", error))?,
                ) as usize;
                let unit = match kind {
                    b"stsc" => 12,
                    b"stss" | b"stco" => 4,
                    _ => 8,
                };
                if entries > 1_000_000 || entries > (body.len() - 8) / unit {
                    return Err(Error::new(Code::MetadataLimit, "sample table"));
                }
            }
            b"stsz" => {
                if body.len() < 12 {
                    return Err(Error::new(Code::DecodeFailed, "sample sizes"));
                }
                let entries = u32::from_be_bytes(
                    body[8..12]
                        .try_into()
                        .map_err(|error| Error::at(Code::DecodeFailed, "sample count", error))?,
                ) as usize;
                if entries > 1_000_000
                    || (body[4..8] == [0, 0, 0, 0] && entries > (body.len() - 12) / 4)
                {
                    return Err(Error::new(Code::MetadataLimit, "sample sizes"));
                }
            }
            _ => {}
        }
        data = &data[n..];
    }
    Ok(())
}
