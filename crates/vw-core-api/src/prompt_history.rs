//! Shared CLI prompt history I/O, called by typed composer commands off the runtime thread.
//! Reads Vibe's JSON-string/legacy lines; writes only complete appended records, never replaces a file.
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAX_ENTRIES: usize = 100;

fn open(path: &Path, append: bool) -> io::Result<File> {
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    if append {
        vw_platform::ensure_private_dir(parent).map_err(io::Error::other)?;
    }
    let dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
    let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    if let Ok(meta) = dir.symlink_metadata(name) {
        #[cfg(windows)]
        let linked = {
            use cap_std::fs::MetadataExt;
            meta.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let linked = meta.file_type().is_symlink();
        if linked || !meta.is_file() {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .append(append)
        .create(append)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = dir.open_with(name, &options)?.into_std();
    if !file.metadata()?.is_file() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(file)
}
fn entries(file: &mut File) -> io::Result<Vec<String>> {
    file.seek(SeekFrom::Start(0))?;
    let mut entries = VecDeque::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        entries.push_back(serde_json::from_str::<String>(&line).unwrap_or(line));
        if entries.len() > MAX_ENTRIES {
            entries.pop_front();
        }
    }
    Ok(entries.into())
}
pub(crate) fn read(path: &Path) -> io::Result<Vec<String>> {
    match open(path, false) {
        Ok(mut file) => entries(&mut file),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}
pub(crate) fn append(path: &Path, text: &str, safe: &dyn Fn(&str) -> bool) -> io::Result<()> {
    append_with_open(text, safe, &mut || open(path, true))
}

fn append_with_open(
    text: &str,
    safe: &dyn Fn(&str) -> bool,
    opener: &mut dyn FnMut() -> io::Result<File>,
) -> io::Result<()> {
    let submitted = text;
    let text = text.trim_end();
    if text.is_empty() || !safe(submitted) {
        return Ok(());
    }
    // A concurrent first creation through a directory handle can report NotFound on macOS.
    // Reopen through the same no-follow checks once; other failures retain their original meaning.
    let mut file = match opener() {
        Err(error) if error.kind() == io::ErrorKind::NotFound => opener()?,
        result => result?,
    };
    // OS lock serializes Vibe Work writers across windows/processes. Vibe CLI does not take this lock.
    file.lock()?;
    if entries(&mut file)?.last().is_some_and(|last| last == text) {
        return Ok(());
    }
    let len = file.metadata()?.len();
    let mut record = String::new();
    if len > 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut end = [0];
        file.read_exact(&mut end)?;
        if end[0] != b'\n' {
            record.push('\n');
        }
    }
    record.push_str(&serde_json::to_string(text)?);
    record.push('\n');
    file.write_all(record.as_bytes())?;
    file.sync_data()
}

#[cfg(test)]
#[path = "prompt_history_tests.rs"]
mod tests;
