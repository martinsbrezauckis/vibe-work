//! Reading an agent process's output log back (the bounded "ring" the monitor and the tools read): the last
//! bytes, the first lines, and the size cap.
//!
//! The log is appended to by the process itself (`spawn`), also while Vibe Work is closed. The cap is applied
//! whenever the core reads it: above the cap the file is emptied (`set_len(0)`). Appends keep working
//! (append mode writes at the current end), only older output is lost; the tools answer from the tail.
//!
//! Depends on: std. Called by: vw-core-api (`agent_tools`), tests.
//! Invariant: reads are bounded; text is decoded lossily and cut at a character boundary.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Default cap of a log on disk.
pub const LOG_CAP: u64 = 1024 * 1024;

/// The last `max` bytes of `path` as text (empty when the file is missing).
///
/// # Errors
/// The OS error for an existing file that cannot be read.
pub fn tail(path: &Path, max: usize) -> io::Result<String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e),
    };
    let len = file.metadata()?.len();
    let want = u64::try_from(max).unwrap_or(u64::MAX).min(len);
    file.seek(SeekFrom::Start(len - want))?;
    let mut bytes = Vec::with_capacity(usize::try_from(want).unwrap_or(0));
    file.take(want).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    // A cut in the middle of a line: start at the next line when there is one.
    Ok(if len > want {
        text.split_once('\n')
            .map_or(text.clone(), |(_, rest)| rest.to_owned())
    } else {
        text
    })
}

/// The first `lines` lines of `path`, at most `max` bytes.
///
/// # Errors
/// The OS error for an existing file that cannot be read.
pub fn head(path: &Path, lines: usize, max: usize) -> io::Result<Vec<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(u64::try_from(max).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .take(lines)
        .map(str::to_owned)
        .collect())
}

/// Empties `path` when it is larger than `cap`. Returns true when it did.
///
/// # Errors
/// The OS error for an existing file that cannot be truncated.
pub fn enforce_cap(path: &Path, cap: u64) -> io::Result<bool> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > cap => {
            OpenOptions::new().write(true).open(path)?.set_len(0)?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
