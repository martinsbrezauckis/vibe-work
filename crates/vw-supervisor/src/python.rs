//! Finds the interpreter associated with the selected Vibe environment for read-only plugin helpers.
//! Uses script placement or the launcher's real format; unknown associations fail closed.
use std::path::{Path, PathBuf};

#[cfg(any(windows, test))]
#[path = "python_launcher.rs"]
mod launcher;
#[cfg(test)]
#[path = "python_tests.rs"]
mod tests;

/// The interpreter belonging to this Vibe script or Windows launcher, or `None` if unproven.
#[must_use]
pub fn server_python(server: &Path) -> Option<PathBuf> {
    let server = std::fs::canonicalize(server).ok()?;
    #[cfg(windows)]
    {
        windows_python(&server)
    }
    #[cfg(not(windows))]
    {
        let python = server.parent()?.join("python");
        if python.is_file() {
            return Some(python);
        }
        unix_shebang_python(&server)
    }
}

#[cfg(not(windows))]
fn unix_shebang_python(server: &Path) -> Option<PathBuf> {
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;

    if !server.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(server)
        .ok()?
        .take(4096)
        .read_to_end(&mut bytes)
        .ok()?;
    let line = bytes
        .strip_prefix(b"#!")?
        .split(|byte| *byte == b'\n')
        .next()?;
    if !bytes.contains(&b'\n') || line.is_empty() || line.iter().any(u8::is_ascii_whitespace) {
        return None;
    }
    let shebang = Path::new(std::ffi::OsStr::from_bytes(line));
    if !shebang.is_absolute() || !shebang.file_name()?.as_bytes().starts_with(b"python") {
        return None;
    }
    // Canonicalize the directory but keep the executable's venv symlink: resolving that final
    // symlink to the base Python would make Python lose its pyvenv.cfg association.
    let bin = std::fs::canonicalize(shebang.parent()?).ok()?;
    let python = bin.join(shebang.file_name()?);
    let target = std::fs::canonicalize(&python).ok()?;
    if !target.is_file()
        || !target.file_name()?.as_bytes().starts_with(b"python")
        || !bin.parent()?.join("pyvenv.cfg").is_file()
    {
        return None;
    }
    Some(python)
}

#[cfg(windows)]
fn windows_python(server: &Path) -> Option<PathBuf> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(server)
        .ok()?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 16 * 1024 * 1024 {
        return None;
    }
    let python = if bytes.starts_with(b"#!") {
        // A sibling is meaningful only for a script actually located in the environment's Scripts folder.
        server.parent()?.join("python.exe")
    } else {
        launcher::interpreter(&bytes)?
    };
    let scripts = python.parent()?;
    if !scripts
        .file_name()?
        .to_str()?
        .eq_ignore_ascii_case("Scripts")
        || !scripts.parent()?.join("pyvenv.cfg").is_file()
        || !scripts.parent()?.join("Lib/site-packages/vibe").is_dir()
        || !python.is_file()
    {
        return None;
    }
    Some(python)
}
