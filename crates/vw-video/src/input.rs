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
        let mut file = tempfile::NamedTempFile::new()
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
