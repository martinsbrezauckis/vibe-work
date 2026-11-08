//! Restores missing app-owned comments beside their settings after Vibe serializes its config.
//! Called by config default ensure functions; values and every existing byte stay unchanged.
use std::path::Path;

pub(crate) fn insert_missing(
    existing: &str,
    section: Option<&str>,
    key: &str,
    notes: &[&str],
) -> Option<String> {
    let mut current_section = None;
    let mut offset = 0;
    let mut anchor = None;
    for line in existing.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            current_section = Some(trimmed);
        } else if current_section == section
            && trimmed
                .split_once('=')
                .is_some_and(|(k, _)| k.trim() == key)
        {
            anchor = Some(offset);
            break;
        }
        offset += line.len();
    }
    let anchor = anchor?;
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
        return None;
    }
    Some(format!(
        "{}{}{}",
        &existing[..anchor],
        prefix,
        &existing[anchor..]
    ))
}

pub(crate) fn restore(
    path: &Path,
    existing: &str,
    section: Option<&str>,
    key: &str,
    notes: &[&str],
) -> Result<(), String> {
    let Some(updated) = insert_missing(existing, section, key, notes) else {
        return Ok(());
    };
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
