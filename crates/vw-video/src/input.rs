//! Snapshot an already authorized open regular file without whole-file buffering or path reopening.
//! Hashing both passes detects in-place changes; the snapshot is private and read-only in the helper.
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use vw_video_types::{Code, Error};
/// Source file byte ceiling, independent of the much smaller image output limit.
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
/// Private immutable source snapshot, removed when its job ends.
pub struct Input {
    file: tempfile::NamedTempFile,
}
impl Input {
    /// Pins authorized bytes and verifies the source did not change during the bounded copy.
    /// # Errors
    /// Nonregular, oversized or changed source, or local I/O failure.
    pub fn copy(mut source: File, expected: &Path) -> Result<Self, Error> {
        let before = source
            .metadata()
            .map_err(|error| Error::at(Code::ChangedInput, "source metadata", error))?;
        if !before.is_file() || before.len() == 0 || before.len() > MAX_FILE_BYTES {
            return Err(Error::new(
                Code::InputSize,
                "regular video must be between 1 byte and 512 MiB",
            ));
        }
        if vw_platform::final_path::final_path(&source)
            .map_err(|error| Error::at(Code::ChangedInput, "source identity", error))?
            != expected
        {
            return Err(Error::new(Code::ChangedInput, "opened file identity"));
        }
        let mut file = tempfile::Builder::new()
            .suffix(&suffix(&mut source, expected)?)
            .tempfile()
            .map_err(|error| Error::at(Code::InputStaging, "private input snapshot", error))?;
        let first = hash(&mut source, Some(file.as_file_mut()))?;
        source
            .seek(SeekFrom::Start(0))
            .map_err(|error| Error::at(Code::ChangedInput, "source seek", error))?;
        let second = hash(&mut source, None)?;
        let after = source
            .metadata()
            .map_err(|error| Error::at(Code::ChangedInput, "source metadata", error))?;
        if first != second
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || vw_platform::final_path::final_path(&source).ok().as_deref() != Some(expected)
        {
            return Err(Error::new(
                Code::ChangedInput,
                "source changed during snapshot",
            ));
        }
        file.flush()
            .map_err(|error| Error::at(Code::InputStaging, "snapshot write", error))?;
        Ok(Self { file })
    }
    /// Private snapshot path, never returned to the agent.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}
/// Media-type suffix for the snapshot name. `AVFoundation` selects its container reader from the file name, so an
/// extensionless snapshot of a valid MP4 reports no video track. ISO media content wins over the original name;
/// otherwise a short ASCII extension of the authorized file is kept. Linux and Windows decode fixed-name copies.
fn suffix(source: &mut File, expected: &Path) -> Result<String, Error> {
    let mut header = [0; 12];
    let mut read = 0;
    while read < header.len() {
        match source.read(&mut header[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(_) => return Err(Error::new(Code::ChangedInput, "source read")),
        }
    }
    source
        .seek(SeekFrom::Start(0))
        .map_err(|error| Error::at(Code::ChangedInput, "source seek", error))?;
    if read == header.len() && &header[4..8] == b"ftyp" {
        return Ok(if &header[8..12] == b"qt  " {
            ".mov"
        } else {
            ".mp4"
        }
        .into());
    }
    Ok(expected
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| (1..=8).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map_or_else(String::new, |e| format!(".{}", e.to_ascii_lowercase())))
}
fn hash(source: &mut File, mut output: Option<&mut File>) -> Result<[u8; 32], Error> {
    let mut sum = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = source
            .read(&mut buffer)
            .map_err(|error| Error::at(Code::ChangedInput, "source read", error))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_FILE_BYTES {
            return Err(Error::new(Code::InputSize, "source bytes"));
        }
        sum.update(&buffer[..n]);
        if let Some(out) = output.as_mut() {
            out.write_all(&buffer[..n])
                .map_err(|error| Error::at(Code::InputStaging, "snapshot write", error))?;
        }
    }
    Ok(sum.finalize().into())
}
