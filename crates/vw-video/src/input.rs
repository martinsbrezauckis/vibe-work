//! Snapshot an already authorized open regular file without whole-file buffering or path reopening.
//! Hashing both passes detects in-place changes; the snapshot is private and read-only in the helper.
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
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
    pub fn copy(mut source: File, expected: &Path) -> Result<Self, String> {
        let before = source
            .metadata()
            .map_err(|_| "changed_input: source metadata")?;
        if !before.is_file() || before.len() == 0 || before.len() > MAX_FILE_BYTES {
            return Err("limit_exceeded: regular video must be between 1 byte and 512 MiB".into());
        }
        if vw_platform::final_path::final_path(&source)
            .map_err(|_| "changed_input: source identity")?
            != expected
        {
            return Err("changed_input: opened file identity".into());
        }
        let mut file = tempfile::Builder::new()
            .suffix(&suffix(&mut source, expected)?)
            .tempfile()
            .map_err(|_| "backend_unavailable: private input snapshot")?;
        let first = hash(&mut source, Some(file.as_file_mut()))?;
        source
            .seek(SeekFrom::Start(0))
            .map_err(|_| "changed_input: source seek")?;
        let second = hash(&mut source, None)?;
        let after = source
            .metadata()
            .map_err(|_| "changed_input: source metadata")?;
        if first != second
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || vw_platform::final_path::final_path(&source).ok().as_deref() != Some(expected)
        {
            return Err("changed_input: source changed during snapshot".into());
        }
        file.flush().map_err(|_| "decode_failed: snapshot write")?;
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
fn suffix(source: &mut File, expected: &Path) -> Result<String, String> {
    let mut header = [0; 12];
    let mut read = 0;
    while read < header.len() {
        match source.read(&mut header[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(_) => return Err("changed_input: source read".into()),
        }
    }
    source
        .seek(SeekFrom::Start(0))
        .map_err(|_| "changed_input: source seek")?;
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
fn hash(source: &mut File, mut output: Option<&mut File>) -> Result<[u8; 32], String> {
    let mut sum = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = source
            .read(&mut buffer)
            .map_err(|_| "changed_input: source read")?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_FILE_BYTES {
            return Err("limit_exceeded: source bytes".into());
        }
        sum.update(&buffer[..n]);
        if let Some(out) = output.as_mut() {
            out.write_all(&buffer[..n])
                .map_err(|_| "decode_failed: snapshot write")?;
        }
    }
    Ok(sum.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(name: &str, bytes: &[u8]) -> (tempfile::TempDir, Input) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        let real = path.canonicalize().unwrap();
        let input = Input::copy(File::open(&real).unwrap(), &real).unwrap();
        (dir, input)
    }
    fn suffix(input: &Input) -> Option<String> {
        input
            .path()
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
    }
    #[test]
    fn snapshot_carries_a_media_type_suffix_for_the_native_framework() {
        // AVFoundation chooses its container reader from the file name: an extensionless copy has no video track.
        let iso = b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2";
        assert_eq!(suffix(&snapshot("clip.MOV", iso).1).as_deref(), Some("mp4"));
        assert_eq!(suffix(&snapshot("clip", iso).1).as_deref(), Some("mp4"));
        let qt = b"\0\0\0\x14ftypqt  \0\0\x02\0qt  ";
        assert_eq!(suffix(&snapshot("clip.mp4", qt).1).as_deref(), Some("mov"));
        assert_eq!(
            suffix(&snapshot("clip.WebM", b"\x1a\x45\xdf\xa3 webm").1).as_deref(),
            Some("webm")
        );
        // Windows refuses `"` in file names, so that odd name is only made where the file system accepts it.
        let quoted: &[&str] = if cfg!(windows) { &[] } else { &["clip.a\"b"] };
        for odd in ["clip", "clip.waytoolongext", "clip.m p4"]
            .iter()
            .chain(quoted)
        {
            assert_eq!(suffix(&snapshot(odd, b"not a container").1), None, "{odd}");
        }
    }
}
