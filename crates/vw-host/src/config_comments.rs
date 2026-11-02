//! Restores only missing app-owned comment lines after Vibe serializes its config.
//! Called by config default ensure functions; values and every existing byte stay unchanged.
use std::path::Path;

pub(crate) fn restore(path: &Path, existing: &str, notes: &[&str]) -> Result<(), String> {
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut prefix = String::new();
    for note in notes {
        if !existing.lines().any(|line| line == *note) {
            prefix.push_str(note);
            prefix.push_str(newline);
        }
    }
    if prefix.is_empty() {
        return Ok(());
    }
    let updated = prefix + existing;
    if updated.parse::<toml::Table>().ok() != existing.parse::<toml::Table>().ok() {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.len() > 1024 * 1024
        || std::fs::read_to_string(path).map_err(|e| e.to_string())? != existing
    {
        return Ok(());
    }
    vw_platform::replace_private(path, updated.as_bytes()).map_err(|e| e.to_string())
}
